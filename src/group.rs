//! Grouping near-duplicates: candidate pairs by dHash through a BK-tree,
//! confirmed by pHash, joined into groups, and the file to keep chosen.

use crate::hash::distance;
use crate::scan::Image;

/// The largest Hamming distances (of 64 bits) still counted as the same
/// picture. Both must hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    pub dhash: u32,
    pub phash: u32,
}

impl Thresholds {
    /// strict: only near-identical copies; normal: resized, recompressed or
    /// lightly edited copies; loose: also heavier edits, with more chance of
    /// unrelated look-alikes. Krawetz's guide: 0 the same picture, up to 10 a
    /// likely variation, above 10 a different picture.
    pub fn preset(name: &str) -> Option<Thresholds> {
        match name {
            "strict" => Some(Thresholds { dhash: 3, phash: 4 }),
            "normal" => Some(Thresholds {
                dhash: 8,
                phash: 10,
            }),
            "loose" => Some(Thresholds {
                dhash: 12,
                phash: 14,
            }),
            _ => None,
        }
    }
}

/// How `b` matches `a`: the distances in the orientation of `b` closest to
/// `a`, if both are within the thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub dhash: u32,
    pub phash: u32,
    /// 0 as it is; 1-3 quarter turns; 4-7 mirrored.
    pub orientation: usize,
}

pub fn best_match(a: &Image, b: &Image) -> Match {
    (0..b.hashes.dhash.len())
        .map(|k| Match {
            dhash: distance(a.hashes.dhash[0], b.hashes.dhash[k]),
            phash: distance(a.hashes.phash[0], b.hashes.phash[k]),
            orientation: k,
        })
        .min_by_key(|m| (m.dhash + m.phash, m.orientation))
        .unwrap()
}

pub fn within(m: &Match, t: Thresholds) -> bool {
    m.dhash <= t.dhash && m.phash <= t.phash
}

/// A BK-tree over 64-bit hashes under Hamming distance.
struct BkTree {
    nodes: Vec<Node>,
}

struct Node {
    hash: u64,
    id: usize,
    /// (distance to this node's hash, child node index)
    children: Vec<(u32, usize)>,
}

impl BkTree {
    fn new() -> BkTree {
        BkTree { nodes: Vec::new() }
    }

    fn insert(&mut self, hash: u64, id: usize) {
        if self.nodes.is_empty() {
            self.nodes.push(Node {
                hash,
                id,
                children: Vec::new(),
            });
            return;
        }
        let mut at = 0;
        loop {
            let d = distance(hash, self.nodes[at].hash);
            match self.nodes[at].children.iter().find(|(k, _)| *k == d) {
                Some(&(_, child)) => at = child,
                None => {
                    let new = self.nodes.len();
                    self.nodes.push(Node {
                        hash,
                        id,
                        children: Vec::new(),
                    });
                    self.nodes[at].children.push((d, new));
                    return;
                }
            }
        }
    }

    /// Ids of every hash within `radius` of `hash`.
    fn within(&self, hash: u64, radius: u32, out: &mut Vec<usize>) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0];
        while let Some(at) = stack.pop() {
            let node = &self.nodes[at];
            let d = distance(hash, node.hash);
            if d <= radius {
                out.push(node.id);
            }
            for &(k, child) in &node.children {
                if k + radius >= d && k <= d + radius {
                    stack.push(child);
                }
            }
        }
    }
}

/// Why a file was chosen to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// The most pixels.
    Largest,
    /// The same pixels as another, but the larger file (less compression).
    LargerFile,
    /// All alike; the first by path.
    FirstPath,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub image: usize,
    /// How it matches the kept image (None for the kept image itself).
    pub to_keep: Option<Match>,
    /// Byte for byte the same file as the kept one.
    pub identical: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The kept image first, the rest by path.
    pub members: Vec<Member>,
    pub keep: Keep,
}

fn same_bytes(a: &Image, b: &Image) -> bool {
    a.bytes == b.bytes
        && matches!((std::fs::read(&a.path), std::fs::read(&b.path)), (Ok(x), Ok(y)) if x == y)
}

/// The order in which files are preferred to keep: the most pixels, then
/// the largest file (the least compressed), then the first path.
fn preferred(a: &Image, b: &Image) -> std::cmp::Ordering {
    b.pixels()
        .cmp(&a.pixels())
        .then(b.bytes.cmp(&a.bytes))
        .then(a.path.cmp(&b.path))
}

/// Groups of two or more images that look the same. Every member is within
/// the thresholds of its group's kept image, not merely linked through
/// another member: the most preferred image not yet in a group takes every
/// other free image that matches it, and so on down the list.
pub fn groups(images: &[Image], t: Thresholds) -> Vec<Group> {
    let mut tree = BkTree::new();
    for (i, img) in images.iter().enumerate() {
        for &h in &img.hashes.dhash {
            tree.insert(h, i);
        }
    }
    let mut order: Vec<usize> = (0..images.len()).collect();
    order.sort_by(|&a, &b| preferred(&images[a], &images[b]));
    let mut taken = vec![false; images.len()];
    let mut found = Vec::new();
    let mut out = Vec::new();
    for &best in &order {
        if taken[best] {
            continue;
        }
        let k = &images[best];
        found.clear();
        tree.within(k.hashes.dhash[0], t.dhash, &mut found);
        found.sort_by(|&a, &b| images[a].path.cmp(&images[b].path));
        found.dedup();
        let mut members = vec![Member {
            image: best,
            to_keep: None,
            identical: false,
        }];
        for &i in &found {
            if i == best || taken[i] {
                continue;
            }
            let m = best_match(k, &images[i]);
            if within(&m, t) {
                taken[i] = true;
                members.push(Member {
                    image: i,
                    to_keep: Some(m),
                    identical: same_bytes(k, &images[i]),
                });
            }
        }
        taken[best] = true;
        if members.len() < 2 {
            continue;
        }
        let others = || members.iter().skip(1).map(|m| &images[m.image]);
        let keep = if others().any(|o| o.pixels() < k.pixels()) {
            Keep::Largest
        } else if others().any(|o| o.bytes < k.bytes) {
            Keep::LargerFile
        } else {
            Keep::FirstPath
        };
        out.push(Group { members, keep });
    }
    // Largest groups first, then by the kept file's path.
    out.sort_by(|a, b| {
        b.members.len().cmp(&a.members.len()).then_with(|| {
            images[a.members[0].image]
                .path
                .cmp(&images[b.members[0].image].path)
        })
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bk_tree_finds_exactly_what_a_full_search_finds() {
        let mut state = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        // Clusters of nearby hashes, so small radii find something.
        let mut hashes = Vec::new();
        for _ in 0..40 {
            let c = next();
            for _ in 0..10 {
                hashes.push(c ^ (next() & next() & next() & next()));
            }
        }
        let mut tree = BkTree::new();
        for (i, &h) in hashes.iter().enumerate() {
            tree.insert(h, i);
        }
        for radius in [0, 3, 8, 12, 20] {
            for q in hashes.iter().step_by(7) {
                let mut got = Vec::new();
                tree.within(*q, radius, &mut got);
                got.sort_unstable();
                let want: Vec<usize> = (0..hashes.len())
                    .filter(|&i| distance(*q, hashes[i]) <= radius)
                    .collect();
                assert_eq!(got, want, "radius {radius}");
            }
        }
    }

    #[test]
    fn presets_get_looser() {
        let s = Thresholds::preset("strict").unwrap();
        let n = Thresholds::preset("normal").unwrap();
        let l = Thresholds::preset("loose").unwrap();
        assert!(s.dhash < n.dhash && n.dhash < l.dhash && s.phash < n.phash && n.phash < l.phash);
        assert_eq!(Thresholds::preset("other"), None);
    }
}
