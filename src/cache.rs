//! A file of hashes from earlier scans, so a scan of the same folders only
//! decodes the files that are new or changed.
//!
//! One line per file: the stamp the file had when it was hashed, its size
//! as seen, its format, the dHash and pHash lists, and the path. The path
//! goes last and in hex, so any byte a path may hold (tabs, newlines,
//! non-UTF-8) is safe. A line whose stamp no longer matches the file is not
//! used. A cache of another version or a damaged one is ignored and
//! rewritten; a file that is not a dupsight cache is never written over.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::hash::Hashes;
use crate::scan::Image;

/// First line of a cache file. Bump the number when the hashes change, so
/// old caches are dropped instead of trusted.
pub const HEADER: &str = "dupsight-cache 1";

/// How the first line of every version starts.
const MAGIC: &str = "dupsight-cache ";

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub stamp: String,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub hashes: Hashes,
}

#[derive(Debug, Default)]
pub struct Cache {
    entries: HashMap<PathBuf, Entry>,
}

/// What a file looks like on disk, for telling whether it changed since it
/// was hashed: device, inode, size, and modification and status change
/// times. The status change time moves on any write and cannot be set
/// back, unlike the modification time. Elsewhere than Unix there is no such
/// time to rely on, so there is no stamp and the cache is never used.
pub fn stamp_of(m: &std::fs::Metadata) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!(
            "{}:{}:{}:{}.{}:{}.{}",
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec()
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        None
    }
}

/// The stamp of the file `path` names now (links followed).
pub fn stamp(path: &Path) -> Option<String> {
    stamp_of(&std::fs::metadata(path).ok()?)
}

/// The name a file is kept under: its full path with links resolved, so
/// `a.png`, `./a.png` and `/abs/a.png` are one entry.
pub fn key(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// The cache may only create a new file or replace an earlier dupsight
/// cache: never a picture, a link, or any other file. Returns whether a
/// cache is there.
pub fn check_path(path: &Path) -> Result<bool, String> {
    if crate::scan::is_image(path) {
        return Err("the cache needs a name that is not an image's (such as photos.cache)".into());
    }
    let meta = match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
        Ok(m) => m,
    };
    let mut head = [0u8; MAGIC.len()];
    let is_cache = meta.file_type().is_file()
        && std::fs::File::open(path)
            .and_then(|mut f| f.read_exact(&mut head))
            .is_ok()
        && head == *MAGIC.as_bytes();
    if is_cache {
        Ok(true)
    } else {
        Err("exists and is not a dupsight cache; choose another name".into())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(unix)]
fn path_from(bytes: Vec<u8>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

#[cfg(not(unix))]
fn path_from(bytes: Vec<u8>) -> Option<PathBuf> {
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

fn hashes_field(h: &[u64]) -> String {
    h.iter()
        .map(|x| format!("{x:016x}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Exactly what `hashes_field` writes: one or eight 16-digit lowercase hex
/// numbers, so a damaged digit count is not taken for another hash.
fn parse_hashes(s: &str) -> Option<Vec<u64>> {
    let v: Option<Vec<u64>> = s
        .split(',')
        .map(|x| {
            let ok = x.len() == 16 && x.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
            if ok {
                u64::from_str_radix(x, 16).ok()
            } else {
                None
            }
        })
        .collect();
    v.filter(|v| v.len() == 1 || v.len() == 8)
}

fn parse_size(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok().filter(|&n| n > 0)
}

/// Lowercase letters and digits, plus any of `extra`; not empty.
fn plain(s: &str, extra: &[u8]) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || extra.contains(&b))
}

fn parse_line(line: &str) -> Option<(PathBuf, Entry)> {
    let f: Vec<&str> = line.split('\t').collect();
    let [stamp, width, height, format, dhash, phash, path] = f.as_slice() else {
        return None;
    };
    let dhash = parse_hashes(dhash)?;
    let phash = parse_hashes(phash)?;
    // The format is shown in reports, so only a plain name such as the
    // image crate gives is taken back.
    if dhash.len() != phash.len() || !plain(stamp, b":.-") || !plain(format, b"") {
        return None;
    }
    Some((
        path_from(unhex(path)?)?,
        Entry {
            stamp: stamp.to_string(),
            width: parse_size(width)?,
            height: parse_size(height)?,
            format: format.to_string(),
            hashes: Hashes { dhash, phash },
        },
    ))
}

impl Cache {
    /// The cache in `text`, or None when it is not a cache of this version
    /// or a line is damaged.
    pub fn parse(text: &str) -> Option<Cache> {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return None;
        }
        let mut entries = HashMap::new();
        for line in lines {
            let (path, entry) = parse_line(line)?;
            entries.insert(path, entry);
        }
        Some(Cache { entries })
    }

    /// The cache at `path`: empty when there is none yet; empty with a
    /// warning when it is of another version or damaged. An error when
    /// something else is there, which the cache must not replace.
    pub fn load(path: &Path) -> Result<(Cache, Option<String>), String> {
        if !check_path(path)? {
            return Ok((Cache::default(), None));
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let first = bytes.split(|&b| b == b'\n').next().unwrap_or_default();
        if first != HEADER.as_bytes() {
            return Ok((Cache::default(), Some("it is of another version".into())));
        }
        Ok(
            match std::str::from_utf8(&bytes).ok().and_then(Cache::parse) {
                Some(c) => (c, None),
                None => (Cache::default(), Some("it is damaged".into())),
            },
        )
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The image at `path` from the cache, when its entry was made from the
    /// file as it is now (`stamp`) and has the orientations asked for.
    pub fn get(&self, key: &Path, stamp: &str, path: &Path, orientations: bool) -> Option<Image> {
        let e = self.entries.get(key)?;
        if e.stamp != stamp {
            return None;
        }
        let hashes = if orientations {
            if e.hashes.dhash.len() != 8 {
                return None;
            }
            e.hashes.clone()
        } else {
            Hashes {
                dhash: vec![e.hashes.dhash[0]],
                phash: vec![e.hashes.phash[0]],
            }
        };
        Some(Image {
            path: path.to_path_buf(),
            width: e.width,
            height: e.height,
            bytes: std::fs::metadata(path).ok()?.len(),
            format: e.format.clone(),
            hashes,
        })
    }

    /// Record a file hashed now. An entry with all eight orientations is
    /// not replaced by one with fewer for the same file as it is.
    pub fn put(&mut self, key: PathBuf, entry: Entry) {
        if let Some(old) = self.entries.get(&key) {
            if old.stamp == entry.stamp && old.hashes.dhash.len() > entry.hashes.dhash.len() {
                return;
            }
        }
        self.entries.insert(key, entry);
    }

    /// Drop the entries of files that are gone.
    pub fn prune(&mut self) {
        self.entries.retain(|p, _| p.exists());
    }

    pub fn render(&self) -> String {
        let mut keys: Vec<&PathBuf> = self.entries.keys().collect();
        keys.sort();
        let mut out = String::from(HEADER);
        out.push('\n');
        for k in keys {
            let e = &self.entries[k];
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                e.stamp,
                e.width,
                e.height,
                e.format,
                hashes_field(&e.hashes.dhash),
                hashes_field(&e.hashes.phash),
                hex(&crate::report::path_bytes(k)),
            ));
        }
        out
    }

    /// Write the cache to `path`: a new file only if nothing is there, or
    /// over the earlier cache, checked through the open file to be one
    /// (`write::write_over`), so nothing else is ever written to. A scan
    /// stopped half way can leave a damaged cache, which the next scan
    /// notices and rewrites.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let existing = check_path(path)?;
        crate::write::write_over(path, self.render().as_bytes(), existing, MAGIC.as_bytes())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(stamp: &str, n: usize) -> Entry {
        Entry {
            stamp: stamp.into(),
            width: 4,
            height: 3,
            format: "png".into(),
            hashes: Hashes {
                dhash: (0..n as u64).collect(),
                phash: (10..10 + n as u64).collect(),
            },
        }
    }

    #[test]
    fn a_cache_survives_rendering_and_parsing() {
        let mut c = Cache::default();
        c.put(PathBuf::from("/tmp/a\tb\nc.png"), entry("1:2:3:4.5:6.7", 1));
        c.put(PathBuf::from("/tmp/d.png"), entry("9", 8));
        c.put(PathBuf::from("/tmp/e.png"), entry("9:-", 1));
        let back = Cache::parse(&c.render()).unwrap();
        assert_eq!(back.entries, c.entries);
    }

    #[test]
    fn another_version_or_a_damaged_line_is_not_used() {
        assert!(Cache::parse("dupsight-cache 0\n").is_none());
        let good = "1\t4\t3\tpng\t000000000000000a\t000000000000000b\t2f\n";
        let ok = |line: &str| Cache::parse(&format!("{HEADER}\n{line}")).is_some();
        assert!(ok(good));
        // A digit short, a zero size, a format with a control character, a
        // path cut in half.
        assert!(!ok(&good.replace("000000000000000a", "00000000000000a")));
        assert!(!ok(&good.replace("\t4\t", "\t0\t")));
        assert!(!ok(&good.replace("png", "p\x1bng")));
        assert!(!ok(&good.replace("\t2f\n", "\t2\n")));
        assert!(Cache::parse(&Cache::default().render()).is_some());
    }

    #[test]
    fn a_full_entry_is_kept_over_a_partial_one_for_the_same_file() {
        let mut c = Cache::default();
        let k = PathBuf::from("/x.png");
        c.put(k.clone(), entry("s", 8));
        c.put(k.clone(), entry("s", 1));
        assert_eq!(c.entries[&k].hashes.dhash.len(), 8);
        // A changed file replaces it whatever it holds.
        c.put(k.clone(), entry("t", 1));
        assert_eq!(c.entries[&k].hashes.dhash.len(), 1);
    }

    #[test]
    fn an_entry_needs_the_same_stamp_and_enough_orientations() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        std::fs::write(&p, b"x").unwrap();
        let mut c = Cache::default();
        c.put(p.clone(), entry("s", 1));
        assert!(c.get(&p, "s", &p, false).is_some());
        assert!(c.get(&p, "other", &p, false).is_none());
        assert!(c.get(&p, "s", &p, true).is_none());
        c.put(p.clone(), entry("s", 8));
        let img = c.get(&p, "s", &p, false).unwrap();
        assert_eq!(img.hashes.dhash, vec![0]);
        assert_eq!(c.get(&p, "s", &p, true).unwrap().hashes.dhash.len(), 8);
    }

    #[test]
    fn only_a_cache_or_nothing_may_be_written_over() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(check_path(&d.join("new.cache")), Ok(false));
        assert!(check_path(&d.join("a.jpg")).is_err());
        std::fs::write(d.join("notes.txt"), "keep me").unwrap();
        assert!(check_path(&d.join("notes.txt")).is_err());
        assert!(Cache::load(&d.join("notes.txt")).is_err());
        assert!(Cache::default().save(&d.join("notes.txt")).is_err());
        assert_eq!(
            std::fs::read_to_string(d.join("notes.txt")).unwrap(),
            "keep me"
        );
        std::fs::write(d.join("old.cache"), "dupsight-cache 0\nwhatever\n").unwrap();
        assert_eq!(check_path(&d.join("old.cache")), Ok(true));
        let (c, warning) = Cache::load(&d.join("old.cache")).unwrap();
        assert!(c.is_empty() && warning.is_some());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.join("old.cache"), d.join("link.cache")).unwrap();
            assert!(check_path(&d.join("link.cache")).is_err());
        }
    }

    #[test]
    fn saving_creates_or_replaces_only_a_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.cache");
        let mut c = Cache::default();
        c.put(PathBuf::from("/a.png"), entry("1", 1));
        c.save(&path).unwrap();
        Cache::default().save(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{HEADER}\n")
        );
    }
}
