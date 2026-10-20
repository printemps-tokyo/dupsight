//! Output: text, JSON, an HTML page with thumbnails, and the list of extra
//! files. Pure: everything returns a String.

use std::fmt::Write as _;
use std::path::Path;

use crate::group::{Group, Keep, Thresholds};
use crate::scan::{Image, Skipped};

/// Text with control characters escaped (`\u{1b}`), so a file name or an
/// error message cannot drive the terminal.
pub fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// A path for people to read: control characters escaped.
pub fn shown(p: &Path) -> String {
    clean(&p.display().to_string())
}

/// A path as the bytes the system uses, so names that are not UTF-8 reach
/// another program unchanged.
#[cfg(unix)]
pub fn path_bytes(p: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
pub fn path_bytes(p: &Path) -> Vec<u8> {
    p.to_string_lossy().into_owned().into_bytes()
}

pub fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

/// What turns a copy into the kept image's orientation: k quarter turns
/// clockwise, then a left-right mirror for 4-7.
pub fn orientation(k: usize) -> &'static str {
    [
        "as is",
        "after a 90 degree clockwise turn",
        "after a 180 degree turn",
        "after a 90 degree counterclockwise turn",
        "after a mirror",
        "after a 90 degree clockwise turn and a mirror",
        "after a 180 degree turn and a mirror",
        "after a 90 degree counterclockwise turn and a mirror",
    ][k]
}

fn keep_reason(k: Keep) -> &'static str {
    match k {
        Keep::Largest => "the most pixels",
        Keep::LargerFile => "as many pixels as another, but a larger file",
        Keep::FirstPath => "the same pixels and bytes as another, the first by path",
    }
}

/// Files in groups other than the kept one.
pub fn extras<'a>(images: &'a [Image], groups: &[Group]) -> Vec<&'a Path> {
    groups
        .iter()
        .flat_map(|g| {
            g.members
                .iter()
                .skip(1)
                .map(|m| images[m.image].path.as_path())
        })
        .collect()
}

pub fn text(images: &[Image], groups: &[Group], skipped: &[Skipped], t: Thresholds) -> String {
    let mut s = String::new();
    for (n, g) in groups.iter().enumerate() {
        let _ = writeln!(
            s,
            "Group {} ({} images; keep: {})",
            n + 1,
            g.members.len(),
            keep_reason(g.keep)
        );
        let paths: Vec<String> = g
            .members
            .iter()
            .map(|m| shown(&images[m.image].path))
            .collect();
        let w = paths
            .iter()
            .map(|p| p.chars().count())
            .max()
            .unwrap_or(0)
            .min(60);
        for (m, path) in g.members.iter().zip(&paths) {
            let img = &images[m.image];
            let mark = if m.to_keep.is_none() { "keep" } else { "    " };
            let how = match (&m.to_keep, m.identical) {
                (None, _) => String::new(),
                (Some(_), true) => "identical file".to_string(),
                (Some(x), false) => {
                    let mut h = format!("dHash {:>2}  pHash {:>2}", x.dhash, x.phash);
                    if x.orientation > 0 {
                        let _ = write!(h, "  {}", orientation(x.orientation));
                    }
                    h
                }
            };
            let dims = format!("{}x{}", img.width, img.height);
            let line = format!(
                "  {mark}  {path:<w$}  {dims:>11}  {:>8}  {:<5} {how}",
                size(img.bytes),
                img.format
            );
            let _ = writeln!(s, "{}", line.trim_end());
        }
        let _ = writeln!(s);
    }
    let in_groups: usize = groups.iter().map(|g| g.members.len()).sum();
    let _ = writeln!(
        s,
        "{} group{}, {} images in them ({} besides the ones to keep); {} image{} scanned{}; limits dHash {}, pHash {}.",
        groups.len(),
        if groups.len() == 1 { "" } else { "s" },
        in_groups,
        in_groups - groups.len(),
        images.len(),
        if images.len() == 1 { "" } else { "s" },
        if skipped.is_empty() { String::new() } else { format!(", {} file{} could not be read", skipped.len(), if skipped.len() == 1 { "" } else { "s" }) },
        t.dhash,
        t.phash
    );
    if !skipped.is_empty() {
        for k in skipped.iter().take(20) {
            let _ = writeln!(s, "  skipped {}: {}", shown(&k.path), clean(&k.reason));
        }
        if skipped.len() > 20 {
            let _ = writeln!(s, "  ... and {} more", skipped.len() - 20);
        }
    }
    s
}

pub fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn json(images: &[Image], groups: &[Group], skipped: &[Skipped], t: Thresholds) -> String {
    let group_json = |g: &Group| {
        let members = g
            .members
            .iter()
            .map(|m| {
                let img = &images[m.image];
                let mut s = format!(
                    "{{\"path\":{},\"width\":{},\"height\":{},\"bytes\":{},\"format\":{},\"keep\":{}",
                    json_str(&img.path.display().to_string()),
                    img.width,
                    img.height,
                    img.bytes,
                    json_str(&img.format),
                    m.to_keep.is_none()
                );
                if let Some(x) = m.to_keep {
                    let _ = write!(
                        s,
                        ",\"dhash\":{},\"phash\":{},\"orientation\":{},\"identical\":{}",
                        x.dhash, x.phash, x.orientation, m.identical
                    );
                }
                s.push('}');
                s
            })
            .collect::<Vec<_>>()
            .join(",");
        let reason = match g.keep {
            Keep::Largest => "largest",
            Keep::LargerFile => "largerFile",
            Keep::FirstPath => "firstPath",
        };
        format!("{{\"keepReason\":\"{reason}\",\"images\":[{members}]}}")
    };
    let skipped = skipped
        .iter()
        .map(|k| {
            format!(
                "{{\"path\":{},\"reason\":{}}}",
                json_str(&k.path.display().to_string()),
                json_str(&k.reason)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"thresholds\":{{\"dhash\":{},\"phash\":{}}},\"scanned\":{},\"groups\":[{}],\"skipped\":[{}]}}\n",
        t.dhash,
        t.phash,
        images.len(),
        groups.iter().map(group_json).collect::<Vec<_>>().join(","),
        skipped
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A file: URL for a local path, every byte outside the unreserved set and
/// "/" percent-encoded.
pub fn file_url(p: &Path) -> String {
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|d| d.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    });
    let mut out = String::from("file://");
    for b in path_bytes(&abs) {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// How every report page starts; an existing file is only replaced when it
/// starts like this.
pub const HTML_MARK: &str = "<!doctype html>\n<!-- dupsight report -->\n";

/// A self-contained page showing each group side by side, the images loaded
/// from their files (open it on the same machine).
pub fn html(images: &[Image], groups: &[Group]) -> String {
    let mut s = String::from(
        "<!doctype html>\n<!-- dupsight report -->\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>dupsight report</title><style>\
         :root{--bg:#fafaf8;--card:#fff;--text:#1d1d1f;--muted:#666;--line:#e3e1dc;--keep:#2e7d32}\
         @media (prefers-color-scheme: dark){:root{--bg:#16171a;--card:#202226;--text:#ececec;--muted:#a3a7ae;--line:#33363c;--keep:#7ccf82}}\
         body{margin:0;padding:16px;background:var(--bg);color:var(--text);font:14px/1.5 system-ui,sans-serif}\
         section{margin:0 0 24px}h2{font-size:16px;margin:0 0 8px}\
         .row{display:flex;flex-wrap:wrap;gap:12px}\
         figure{margin:0;width:220px;background:var(--card);border:1px solid var(--line);border-radius:8px;padding:8px}\
         figure.keep{border-color:var(--keep);border-width:2px}\
         img{width:100%;height:160px;object-fit:contain;background:repeating-conic-gradient(#ccc 0 25%,#fff 0 50%) 0 0/16px 16px;border-radius:4px}\
         figcaption{font-size:12px;color:var(--muted);word-break:break-all}figcaption b{color:var(--keep)}\
         </style></head><body>\n",
    );
    let _ = writeln!(
        s,
        "<h1>dupsight: {} group{}</h1>",
        groups.len(),
        if groups.len() == 1 { "" } else { "s" }
    );
    for (n, g) in groups.iter().enumerate() {
        let _ = writeln!(
            s,
            "<section><h2>Group {} ({} images; keep: {})</h2><div class=\"row\">",
            n + 1,
            g.members.len(),
            keep_reason(g.keep)
        );
        for m in &g.members {
            let img = &images[m.image];
            let note = match (m.to_keep, m.identical) {
                (None, _) => "<b>keep</b>".to_string(),
                (Some(_), true) => "identical file".to_string(),
                (Some(x), false) if x.orientation > 0 => {
                    format!(
                        "dHash {} / pHash {}, {}",
                        x.dhash,
                        x.phash,
                        orientation(x.orientation)
                    )
                }
                (Some(x), false) => format!("dHash {} / pHash {}", x.dhash, x.phash),
            };
            let _ = writeln!(
                s,
                "<figure{}><img loading=\"lazy\" src=\"{}\" alt=\"\"><figcaption>{}<br>{}x{}, {}, {}<br>{}</figcaption></figure>",
                if m.to_keep.is_none() { " class=\"keep\"" } else { "" },
                html_escape(&file_url(&img.path)),
                html_escape(&img.path.display().to_string()),
                img.width,
                img.height,
                size(img.bytes),
                html_escape(&img.format),
                note
            );
        }
        s.push_str("</div></section>\n");
    }
    s.push_str("</body></html>\n");
    s
}
