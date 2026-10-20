//! Finding image files and hashing them, in parallel.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use image::{DynamicImage, ImageDecoder};

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
    let bytes = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = reader
        .format()
        .map_or("unknown".to_string(), |f| format!("{f:?}").to_lowercase());
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
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
    Ok(Image {
        path: path.to_path_buf(),
        width,
        height,
        bytes,
        format,
        hashes: hashes_of(&g, orientations),
    })
}

/// Hash every file, in parallel; files that cannot be read are listed apart.
pub fn scan(files: &[PathBuf], orientations: bool) -> (Vec<Image>, Vec<Skipped>) {
    let results: Vec<Result<Image, Skipped>> = files
        .par_iter()
        .map(|p| {
            load(p, orientations).map_err(|reason| Skipped {
                path: p.clone(),
                reason,
            })
        })
        .collect();
    let mut images = Vec::new();
    let mut skipped = Vec::new();
    for r in results {
        match r {
            Ok(i) => images.push(i),
            Err(s) => skipped.push(s),
        }
    }
    (images, skipped)
}
