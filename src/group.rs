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
    /// unrelated look-alikes. Krawetz's guide for dHash: 0 the same picture,
    /// up to 10 a likely variation, above 10 a different picture. The pHash
    /// limits come from measuring copies and unrelated pictures (see the
    /// README): pictures sharing a layout reach a pHash distance of 8 to 10.
    pub fn preset(name: &str) -> Option<Thresholds> {
        match name {
            "strict" => Some(Thresholds { dhash: 4, phash: 4 }),
            "normal" => Some(Thresholds { dhash: 8, phash: 7 }),
            "loose" => Some(Thresholds {
                dhash: 12,
                phash: 12,
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

/// The orientation of `b` that best matches `a` within the thresholds, or,
/// when none is within them, the closest one.
pub fn best_match(a: &Image, b: &Image, t: Thresholds) -> Match {
    let all = (0..b.hashes.dhash.len()).map(|k| Match {
        dhash: distance(a.hashes.dhash[0], b.hashes.dhash[k]),
        phash: distance(a.hashes.phash[0], b.hashes.phash[k]),
        orientation: k,
    });
    all.min_by_key(|m| (!within(m, t), m.dhash + m.phash, m.orientation))
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
    /// Every image with this hash.
    ids: Vec<usize>,
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
                ids: vec![id],
                children: Vec::new(),
            });
            return;
        }
        let mut at = 0;
        loop {
            let d = distance(hash, self.nodes[at].hash);
            if d == 0 {
                self.nodes[at].ids.push(id);
                return;
            }
            match self.nodes[at].children.iter().find(|(k, _)| *k == d) {
                Some(&(_, child)) => at = child,
                None => {
                    let new = self.nodes.len();
                    self.nodes.push(Node {
                        hash,
                        ids: vec![id],
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
                out.extend_from_slice(&node.ids);
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

/// Byte for byte the same, read in pieces so large files are not held in
/// memory. A file that cannot be read is not the same.
fn same_bytes(a: &Image, b: &Image) -> bool {
    use std::io::Read;
    if a.bytes != b.bytes {
        return false;
    }
    let (Ok(fa), Ok(fb)) = (std::fs::File::open(&a.path), std::fs::File::open(&b.path)) else {
        return false;
    };
    let (mut ra, mut rb) = (std::io::BufReader::new(fa), std::io::BufReader::new(fb));
    let (mut ba, mut bb) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        let n = match ra.read(&mut ba) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if n == 0 {
            // Both must end here.
            return matches!(rb.read(&mut bb[..1]), Ok(0));
        }
        if rb.read_exact(&mut bb[..n]).is_err() || ba[..n] != bb[..n] {
            return false;
        }
    }
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
        let mut hs = img.hashes.dhash.clone();
        hs.sort_unstable();
        hs.dedup();
        for h in hs {
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
            let m = best_match(k, &images[i], t);
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
        // The reason is what set the kept file apart from the runner-up.
        let second = members[1..]
            .iter()
            .map(|m| &images[m.image])
            .min_by(|a, b| preferred(a, b))
            .unwrap();
        let keep = if second.pixels() < k.pixels() {
            Keep::Largest
        } else if second.bytes < k.bytes {
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
    fn identical_hashes_share_one_node() {
        let mut tree = BkTree::new();
        for i in 0..10_000 {
            tree.insert(42, i);
        }
        assert_eq!(tree.nodes.len(), 1);
        let mut got = Vec::new();
        tree.within(43, 1, &mut got);
        assert_eq!(got.len(), 10_000);
    }

    fn image(name: &str, pixels: u32, bytes: u64, dhash: Vec<u64>, phash: Vec<u64>) -> Image {
        Image {
            path: name.into(),
            width: pixels,
            height: 1,
            bytes,
            format: "png".into(),
            hashes: crate::hash::Hashes { dhash, phash },
        }
    }

    #[test]
    fn best_match_prefers_an_orientation_within_the_limits() {
        let t = Thresholds {
            dhash: 8,
            phash: 10,
        };
        let a = image("a", 1, 1, vec![0], vec![0]);
        // Orientation 1 is closer in sum (9 + 0) but over the dHash limit;
        // orientation 2 (8 + 10) is within both.
        let b = image(
            "b",
            1,
            1,
            vec![u64::MAX, 0x1ff, 0xff],
            vec![u64::MAX, 0, 0x3ff],
        );
        let m = best_match(&a, &b, t);
        assert_eq!((m.dhash, m.phash, m.orientation), (8, 10, 2));
        assert!(within(&m, t));
        // With nothing within, the closest is still reported.
        let m = best_match(&a, &b, Thresholds { dhash: 1, phash: 1 });
        assert_eq!(m.orientation, 1);
    }

    #[test]
    fn members_match_the_kept_image_not_a_chain() {
        // a is near b and b near c, but a is not near c.
        let t = Thresholds {
            dhash: 4,
            phash: 64,
        };
        let imgs = vec![
            image("a", 100, 1, vec![0], vec![0]),
            image("b", 50, 1, vec![0xf], vec![0]),
            image("c", 10, 1, vec![0xff], vec![0]),
        ];
        let g = groups(&imgs, t);
        assert_eq!(g.len(), 1);
        let ids: Vec<usize> = g[0].members.iter().map(|m| m.image).collect();
        assert_eq!(ids, [0, 1]);
    }

    #[test]
    fn keep_reason_is_what_beat_the_runner_up() {
        let t = Thresholds { dhash: 0, phash: 0 };
        // a and b have the most pixels; a wins on bytes; c is smaller.
        let imgs = vec![
            image("a", 100, 9, vec![0], vec![0]),
            image("b", 100, 5, vec![0], vec![0]),
            image("c", 10, 50, vec![0], vec![0]),
        ];
        let g = groups(&imgs, t);
        assert_eq!(g[0].members[0].image, 0);
        assert_eq!(g[0].keep, Keep::LargerFile);
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
