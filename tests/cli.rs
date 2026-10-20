mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::{jpeg, jpeg_with_orientation, picture, png};
use image::imageops::FilterType;

fn dupsight(args: &[&str], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dupsight"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

/// a.png (800x600) with a smaller copy, a JPEG copy and an exact copy; b.png
/// with a turned copy; c.png alone; and a file that is not an image.
fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let a = picture(1, 800, 600);
    std::fs::write(d.join("a.png"), png(&a)).unwrap();
    std::fs::create_dir(d.join("sub")).unwrap();
    std::fs::write(
        d.join("sub/a-small.png"),
        png(&a.resize_exact(400, 300, FilterType::Triangle)),
    )
    .unwrap();
    std::fs::write(d.join("sub/a.jpg"), jpeg(&a, 60)).unwrap();
    std::fs::write(d.join("sub/a-copy.png"), png(&a)).unwrap();
    let b = picture(2, 640, 480);
    std::fs::write(d.join("b.png"), png(&b)).unwrap();
    std::fs::write(d.join("b-turned.png"), png(&b.rotate90())).unwrap();
    std::fs::write(d.join("c.png"), png(&picture(3, 640, 480))).unwrap();
    std::fs::write(d.join("broken.jpg"), b"not an image").unwrap();
    dir
}

#[test]
fn scan_groups_copies_and_keeps_the_best() {
    let dir = folder();
    let o = dupsight(&["scan", "."], dir.path());
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = stdout(&o);
    // a.png and its exact copy tie on everything: the first path wins.
    assert!(
        out.starts_with(
            "Group 1 (4 images; keep: the same pixels and bytes as another, the first by path)"
        ),
        "{out}"
    );
    let keep = out.lines().find(|l| l.starts_with("  keep")).unwrap();
    assert!(keep.contains("./a.png"), "{keep}");
    assert!(out.contains("identical file"), "{out}");
    assert!(
        !out.contains("b-turned"),
        "turned copies need --rotations: {out}"
    );
    assert!(!out.contains("c.png"), "{out}");
    assert!(out.contains("1 group, 4 images in them (3 besides the ones to keep); 7 images scanned, 1 file could not be read"), "{out}");
    assert!(out.contains("skipped ./broken.jpg"), "{out}");
}

#[test]
fn rotations_match_turned_copies() {
    let dir = folder();
    let out = stdout(&dupsight(&["scan", ".", "--rotations"], dir.path()));
    assert!(out.contains("2 groups"), "{out}");
    let line = out
        .lines()
        .find(|l| l.contains("b-turned") || (l.contains("b.png") && !l.starts_with("  keep")))
        .unwrap();
    assert!(line.contains("turn"), "{line}");
}

#[test]
fn extras_list_everything_but_the_kept_files() {
    let dir = folder();
    let o = dupsight(&["scan", ".", "--extras"], dir.path());
    let mut got: Vec<String> = stdout(&o).lines().map(String::from).collect();
    got.sort();
    assert_eq!(
        got,
        ["./sub/a-copy.png", "./sub/a-small.png", "./sub/a.jpg"]
    );
    let o = dupsight(&["scan", ".", "--extras", "-0"], dir.path());
    assert_eq!(stdout(&o).matches('\0').count(), 3);
    assert!(!stdout(&o).contains('\n'));
}

#[test]
fn json_has_groups_and_skipped_files() {
    let dir = folder();
    let out = stdout(&dupsight(
        &["scan", ".", "--json", "--level", "strict"],
        dir.path(),
    ));
    assert!(out.starts_with("{\"thresholds\":{\"dhash\":4,\"phash\":4},\"scanned\":7,\"groups\":[{\"keepReason\":\"firstPath\""), "{out}");
    assert!(out.contains("\"identical\":true"), "{out}");
    assert!(
        out.contains("\"skipped\":[{\"path\":\"./broken.jpg\""),
        "{out}"
    );
}

#[test]
fn check_fails_only_when_something_is_found() {
    let dir = folder();
    assert_eq!(
        dupsight(&["scan", ".", "--check"], dir.path())
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        dupsight(&["scan", "c.png", "b.png", "--check"], dir.path())
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn thresholds_can_be_overridden() {
    let dir = folder();
    let out = stdout(&dupsight(
        &["scan", ".", "--max-dhash", "64", "--max-phash", "64"],
        dir.path(),
    ));
    assert!(out.contains("1 group, 7 images"), "{out}");
    let o = dupsight(&["scan", ".", "--max-dhash", "65"], dir.path());
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn hidden_files_are_left_out_unless_asked() {
    let dir = folder();
    std::fs::create_dir(dir.path().join(".cache")).unwrap();
    std::fs::copy(dir.path().join("c.png"), dir.path().join(".cache/c.png")).unwrap();
    assert!(!stdout(&dupsight(&["scan", "."], dir.path())).contains(".cache"));
    assert!(stdout(&dupsight(&["scan", ".", "--hidden"], dir.path())).contains(".cache/c.png"));
}

#[test]
fn html_report_escapes_names() {
    let dir = folder();
    std::fs::copy(dir.path().join("c.png"), dir.path().join("c <&>.png")).unwrap();
    let o = dupsight(&["scan", ".", "--html", "report.html"], dir.path());
    assert!(o.status.success());
    let page = std::fs::read_to_string(dir.path().join("report.html")).unwrap();
    assert!(page.contains("c &lt;&amp;&gt;.png"), "names are escaped");
    assert!(page.contains("c%20%3C%26%3E.png"), "URLs are encoded");
    assert!(page.contains("<figure class=\"keep\">"));
}

#[test]
fn compare_reports_distances_and_exit_code() {
    let dir = folder();
    let o = dupsight(&["compare", "a.png", "sub/a.jpg"], dir.path());
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).ends_with("the same picture\n"), "{}", stdout(&o));
    let o = dupsight(&["compare", "a.png", "c.png"], dir.path());
    assert_eq!(o.status.code(), Some(1));
    let o = dupsight(
        &["compare", "b.png", "b-turned.png", "--rotations"],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(
        stdout(&o).contains("closest after a 90 degree counterclockwise turn"),
        "{}",
        stdout(&o)
    );
    let o = dupsight(&["compare", "a.png", "broken.jpg"], dir.path());
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn missing_paths_are_errors() {
    let dir = folder();
    let o = dupsight(&["scan", "nope"], dir.path());
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("dupsight: nope:"));
}

#[test]
fn a_file_named_twice_is_scanned_once() {
    let dir = folder();
    let out = stdout(&dupsight(
        &["scan", "c.png", "./c.png", "sub/../c.png"],
        dir.path(),
    ));
    assert!(out.starts_with("0 groups"), "{out}");
    assert!(out.contains("1 image scanned"), "{out}");
    // Overlapping folders, too.
    let out = stdout(&dupsight(&["scan", ".", "sub", "./sub/"], dir.path()));
    assert!(out.contains("7 images scanned"), "{out}");
}

#[test]
fn the_html_report_never_replaces_a_scanned_file() {
    let dir = folder();
    let before = std::fs::read(dir.path().join("c.png")).unwrap();
    let o = dupsight(&["scan", ".", "--html", "c.png"], dir.path());
    assert_eq!(o.status.code(), Some(2));
    #[cfg(unix)]
    {
        std::fs::hard_link(dir.path().join("c.png"), dir.path().join("c.html")).unwrap();
        let o = dupsight(&["scan", ".", "--html", "c.html"], dir.path());
        assert_eq!(
            o.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(String::from_utf8_lossy(&o.stderr).contains("one of the scanned files"));
    }
    assert_eq!(std::fs::read(dir.path().join("c.png")).unwrap(), before);
}

#[test]
fn extras_still_tell_about_skipped_files() {
    let dir = folder();
    let o = dupsight(&["scan", ".", "--extras"], dir.path());
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("skipped ./broken.jpg"));
}

#[test]
fn nothing_readable_is_an_error() {
    let dir = folder();
    let o = dupsight(&["scan", "broken.jpg", "--check"], dir.path());
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("none of the files could be read"));
}

#[test]
fn control_characters_in_names_are_escaped() {
    let dir = folder();
    std::fs::write(dir.path().join("red\x1b[31m.jpg"), b"nope").unwrap();
    let o = dupsight(&["scan", "."], dir.path());
    assert!(!o.stdout.contains(&0x1b), "stdout carries a raw escape");
    assert!(stdout(&o).contains("red\\u{1b}[31m.jpg"), "{}", stdout(&o));
    let o = dupsight(&["scan", "gone\x1b[31m.png"], dir.path());
    assert!(!o.stderr.contains(&0x1b), "stderr carries a raw escape");
}

#[test]
fn exif_orientation_is_applied() {
    let dir = folder();
    let d = picture(9, 600, 400);
    // Stored sideways with a tag saying "turn 90 degrees clockwise" (6),
    // next to a copy with the turn baked into its pixels.
    std::fs::write(
        dir.path().join("d-tagged.jpg"),
        jpeg_with_orientation(&d.rotate270(), 6),
    )
    .unwrap();
    std::fs::write(dir.path().join("d.png"), png(&d)).unwrap();
    let out = stdout(&dupsight(&["scan", "d-tagged.jpg", "d.png"], dir.path()));
    assert!(out.starts_with("Group 1 (2 images"), "{out}");
    // Shown as seen: 600x400, not the stored 400x600.
    assert!(!out.contains("400x600"), "{out}");
}
