mod common;

use common::picture;
use dupsight::hash::{
    dhash, distance, exif_turn, hashes, orient, phash, resample, thumbnail, Gray,
};
use image::metadata::Orientation;
use image::{imageops::FilterType, DynamicImage, ImageBuffer, Luma};

fn gray(side: usize, f: impl Fn(usize, usize) -> f32) -> Gray {
    Gray {
        side,
        px: (0..side * side).map(|i| f(i % side, i / side)).collect(),
    }
}

#[test]
fn resample_averages_areas() {
    // 4x2 down to 2x1: each output pixel is the mean of a 2x2 block.
    let src = [0.0, 2.0, 10.0, 20.0, 4.0, 6.0, 30.0, 40.0];
    assert_eq!(resample(&src, 4, 2, 2, 1), vec![3.0, 25.0]);
    // Down by a fraction: 3 to 2 splits the middle pixel.
    let out = resample(&[0.0, 3.0, 6.0], 3, 1, 2, 1);
    assert!(
        (out[0] - 1.0).abs() < 1e-5 && (out[1] - 5.0).abs() < 1e-5,
        "{out:?}"
    );
}

#[test]
fn dhash_compares_left_to_right() {
    // Brighter to the right: every left pixel is darker, so every bit is 1.
    assert_eq!(dhash(&gray(64, |x, _| x as f32)), u64::MAX);
    assert_eq!(dhash(&gray(64, |x, _| 64.0 - x as f32)), 0);
    // A flat image has no darker neighbors.
    assert_eq!(dhash(&gray(64, |_, _| 0.5)), 0);
}

#[test]
fn phash_splits_low_frequencies_at_their_mean() {
    // Left half light, right half dark: the first horizontal cosine (positive
    // on the left) is the strongest frequency, so its bit (row 0, column 1)
    // is set; the other way round it is clear.
    let light_left = phash(&gray(64, |x, _| if x < 32 { 1.0 } else { 0.0 }));
    let dark_left = phash(&gray(64, |x, _| if x < 32 { 0.0 } else { 1.0 }));
    assert_eq!(light_left >> 62 & 1, 1);
    assert_eq!(dark_left >> 62 & 1, 0);
    // Only frequencies the picture has differ between the two.
    assert!(distance(light_left, dark_left) >= 1);
}

#[test]
fn orientations_are_distinct_and_turn_clockwise() {
    let g = gray(64, |x, y| {
        (x * 3 + y * 7 % 13) as f32 + if x < 20 && y < 10 { 100.0 } else { 0.0 }
    });
    let all: Vec<Gray> = (0..8).map(|k| orient(&g, k)).collect();
    for i in 0..8 {
        for j in i + 1..8 {
            assert_ne!(all[i].px, all[j].px, "orientations {i} and {j}");
        }
    }
    // A clockwise turn puts the top-left corner at the top right.
    assert_eq!(orient(&g, 1).px[63], g.px[0]);
    // Four turns, or two mirrors, give the picture back.
    assert_eq!(orient(&orient(&orient(&orient(&g, 1), 1), 1), 1).px, g.px);
    assert_eq!(orient(&orient(&g, 4), 4).px, g.px);
}

#[test]
fn turned_copies_match_in_some_orientation() {
    let base = picture(7, 300, 200);
    let h = hashes(&base, false);
    for (name, turned) in [
        ("90", base.rotate90()),
        ("180", base.rotate180()),
        ("270", base.rotate270()),
        ("mirror", base.fliph()),
        ("upside down mirror", base.flipv()),
    ] {
        let t = hashes(&turned, true);
        assert_eq!(t.dhash.len(), 8);
        let best = (0..8)
            .map(|k| distance(h.dhash[0], t.dhash[k]) + distance(h.phash[0], t.phash[k]))
            .min()
            .unwrap();
        assert!(best <= 2, "{name}: {best}");
        assert!(
            distance(h.dhash[0], t.dhash[0]) > 10,
            "{name} should differ as it is"
        );
    }
}

#[test]
fn alpha_is_seen_over_white() {
    let clear =
        DynamicImage::ImageRgba8(ImageBuffer::from_pixel(10, 10, image::Rgba([0, 0, 0, 0])));
    let white = DynamicImage::ImageLuma8(ImageBuffer::from_pixel(10, 10, Luma([255])));
    assert_eq!(thumbnail(&clear).px, thumbnail(&white).px);
}

#[test]
fn edits_stay_close_and_other_pictures_far() {
    let base = picture(1, 800, 600);
    let h = hashes(&base, false);
    let near = |img: &DynamicImage| {
        let x = hashes(img, false);
        (
            distance(h.dhash[0], x.dhash[0]),
            distance(h.phash[0], x.phash[0]),
        )
    };
    let jpeg = image::load_from_memory(&common::jpeg(&base, 40)).unwrap();
    for (name, img) in [
        ("smaller", base.resize_exact(200, 150, FilterType::Triangle)),
        ("jpeg", jpeg),
        ("brighter", base.brighten(25)),
        ("blurred", base.blur(1.5)),
        ("gray", DynamicImage::ImageLuma8(base.to_luma8())),
    ] {
        let (d, p) = near(&img);
        assert!(d <= 4 && p <= 6, "{name}: dHash {d}, pHash {p}");
    }
    for seed in 2..30 {
        let (d, p) = near(&picture(seed, 800, 600));
        assert!(d > 12 || p > 14, "seed {seed}: dHash {d}, pHash {p}");
    }
}

fn close(a: &Gray, b: &Gray) -> bool {
    a.px.iter().zip(&b.px).all(|(x, y)| (x - y).abs() < 0.01)
}

#[test]
fn exif_orientations_map_to_the_same_turns() {
    // Each Exif orientation applied to the pixels gives the thumbnail that
    // `orient` gives, so a tagged photo and its baked-in export hash alike.
    let img = picture(4, 96, 64);
    let g = thumbnail(&img);
    for o in [
        Orientation::NoTransforms,
        Orientation::Rotate90,
        Orientation::Rotate180,
        Orientation::Rotate270,
        Orientation::FlipHorizontal,
        Orientation::FlipVertical,
        Orientation::Rotate90FlipH,
        Orientation::Rotate270FlipH,
    ] {
        let mut turned = img.clone();
        turned.apply_orientation(o);
        assert!(
            close(&thumbnail(&turned), &orient(&g, exif_turn(o))),
            "{o:?}"
        );
    }
}

#[test]
fn thumbnail_is_an_area_average_at_any_size() {
    // Two 64-wide halves of different gray, any size: the thumbnail's left
    // half is the one gray and its right half the other.
    for (w, h) in [(640, 480), (65, 7), (10, 10), (3000, 2), (2_000_000, 1)] {
        let img = DynamicImage::ImageLuma8(ImageBuffer::from_fn(w, h, |x, _| {
            Luma([if x < w / 2 { 40 } else { 200 }])
        }));
        let g = thumbnail(&img);
        if w % 2 == 0 {
            assert!(
                (g.px[0] - 40.0).abs() < 0.01 && (g.px[63] - 200.0).abs() < 0.01,
                "{w}x{h}"
            );
        }
        let mean = g.px.iter().sum::<f32>() / g.px.len() as f32;
        let want = (40.0 * (w / 2) as f32 + 200.0 * (w - w / 2) as f32) / w as f32;
        assert!((mean - want).abs() < 0.05, "{w}x{h}: {mean} {want}");
    }
}

#[test]
fn color_under_transparency_does_not_count() {
    // A large image, half of it transparent over random colors, looks the
    // same as the same image flattened onto white.
    let base = picture(5, 900, 700).to_rgba8();
    let hidden = image::RgbaImage::from_fn(900, 700, |x, y| {
        let p = base.get_pixel(x, y).0;
        if x < 450 {
            image::Rgba([p[2], p[0], p[1], 0])
        } else {
            image::Rgba(p)
        }
    });
    let flat = image::RgbaImage::from_fn(900, 700, |x, y| {
        if x < 450 {
            image::Rgba([255, 255, 255, 255])
        } else {
            *base.get_pixel(x, y)
        }
    });
    let a = thumbnail(&DynamicImage::ImageRgba8(hidden));
    let b = thumbnail(&DynamicImage::ImageRgba8(flat));
    assert!(close(&a, &b));
}
