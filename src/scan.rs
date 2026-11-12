//! Finding image files and hashing them, in parallel.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use image::{DynamicImage, ImageDecoder};

use crate::cache::{self, Cache, Entry};
use crate::hash::{exif_turn, hashes_of, orient, thumbnail, Hashes};

/// Extensions read as images (case does not matter).
pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff"];

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub format: String,
    pub hashes: Hashes,
}

impl Image {
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// What identifies a file on disk whatever path names it: device and inode
/// on Unix, the resolved path elsewhere.
pub fn file_key(p: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(p).ok()?;
        Some(format!("{}:{}", m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        std::fs::canonicalize(p)
            .ok()
            .map(|c| c.display().to_string())
    }
}

/// Image files under `paths` (files are taken as given; folders are walked).
/// Hidden files and folders are skipped unless `hidden`; symbolic links in
/// folders are not followed. Sorted by path, and each file only once even
/// when named twice (`a.png` and `./a.png`, overlapping folders, hard
/// links), under the first of its paths. A path that does not exist is an
/// error; a folder that cannot be read on the way is listed apart.
pub fn find(paths: &[PathBuf], hidden: bool) -> Result<(Vec<PathBuf>, Vec<Skipped>), String> {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for p in paths {
        let meta = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
        if meta.is_file() {
            out.push(p.clone());
            continue;
        }
        let walk = ignore::WalkBuilder::new(p)
            .standard_filters(false)
            .hidden(!hidden)
            .follow_links(false)
            .build();
        for entry in walk {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    let path = error_path(&e).unwrap_or(p).to_path_buf();
                    skipped.push(Skipped {
                        path,
                        reason: e.to_string(),
                    });
                    continue;
                }
            };
            if entry.file_type().is_some_and(|t| t.is_file()) && is_image(entry.path()) {
                out.push(entry.into_path());
            }
        }
    }
    out.sort();
    out.dedup();
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| file_key(p).is_none_or(|k| seen.insert(k)));
    Ok((out, skipped))
}

fn error_path(e: &ignore::Error) -> Option<&Path> {
    match e {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            error_path(err)
        }
        _ => None,
    }
}

/// Decode and hash one file, turned the way its Exif orientation says it is
/// meant to be seen. Width and height are as seen, too.
pub fn load(path: &Path, orientations: bool) -> Result<Image, String> {
    load_stamped(path, orientations).map(|(img, _)| img)
}

/// `load`, and the cache stamp of the file that was decoded: both come from
/// one open handle, so they belong to the same file even if the path is
/// pointed elsewhere meanwhile.
pub fn load_stamped(path: &Path, orientations: bool) -> Result<(Image, Option<String>), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    let bytes = meta.len();
    let stamp = cache::stamp_of(&meta);
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = reader
        .format()
        .map_or("unknown".to_string(), |f| format!("{f:?}").to_lowercase());
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    // The size check that ImageReader::decode makes, kept here because the
    // decoder is needed for its orientation: refuse a picture whose pixels
    // would not fit in the allocation limit before allocating them.
    let mut limits = image::Limits::default();
    limits
        .reserve(decoder.total_bytes())
        .map_err(|e| e.to_string())?;
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    let turn = decoder.orientation().map_or(0, exif_turn);
    let img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    if img.width() == 0 || img.height() == 0 {
        return Err("the image is empty".into());
    }
    let (width, height) = if turn % 2 == 1 {
        (img.height(), img.width())
    } else {
        (img.width(), img.height())
    };
    // Only the thumbnail is turned, not the whole image.
    let mut g = thumbnail(&img);
    drop(img);
    if turn != 0 {
        g = orient(&g, turn);
    }
    Ok((
        Image {
            path: path.to_path_buf(),
            width,
            height,
            bytes,
            format,
            hashes: hashes_of(&g, orientations),
        },
        stamp,
    ))
}

/// Hash every file, in parallel; files that cannot be read are listed apart.
pub fn scan(files: &[PathBuf], orientations: bool) -> (Vec<Image>, Vec<Skipped>) {
    let (images, skipped, _) = scan_cached(files, orientations, &mut Cache::default());
    (images, skipped)
}

/// Like `scan`, but a file `cache` holds unchanged is taken from it instead
/// of decoded, and every file hashed now is added to it. Also returns how
/// many came from the cache.
pub fn scan_cached(
    files: &[PathBuf],
    orientations: bool,
    cache: &mut Cache,
) -> (Vec<Image>, Vec<Skipped>, usize) {
    enum Hashed {
        Cached(Image),
        Decoded(Image, Option<(PathBuf, String)>),
    }
    let shared: &Cache = cache;
    let results: Vec<Result<Hashed, Skipped>> = files
        .par_iter()
        .map(|p| {
            // A hit needs the file the path names now to carry the stamp
            // its entry was made from.
            let key = cache::key(p);
            if let Some((key, stamp)) = key.as_ref().zip(cache::stamp(p)) {
                if let Some(img) = shared.get(key, &stamp, p, orientations) {
                    return Ok(Hashed::Cached(img));
                }
            }
            // The stamp stored is the decoded file's own (from its open
            // handle), so if the path pointed at another file meanwhile,
            // the entry will not match the file under `key` next time.
            load_stamped(p, orientations)
                .map(|(img, stamp)| Hashed::Decoded(img, key.zip(stamp)))
                .map_err(|reason| Skipped {
                    path: p.clone(),
                    reason,
                })
        })
        .collect();
    let mut images = Vec::new();
    let mut skipped = Vec::new();
    let mut reused = 0;
    for r in results {
        match r {
            Ok(Hashed::Decoded(img, id)) => {
                if let Some((key, stamp)) = id {
                    cache.put(
                        key,
                        Entry {
                            stamp,
                            width: img.width,
                            height: img.height,
                            format: img.format.clone(),
                            hashes: img.hashes.clone(),
                        },
                    );
                }
                images.push(img);
            }
            Ok(Hashed::Cached(img)) => {
                reused += 1;
                images.push(img);
            }
            Err(s) => skipped.push(s),
        }
    }
    (images, skipped, reused)
}
