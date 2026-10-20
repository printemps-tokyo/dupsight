//! Perceptual hashes, after Neal Krawetz's descriptions:
//! - dHash ("Kind of Like That", hackerfactor.com): a 9x8 grayscale
//!   image, one bit per pair of horizontal neighbors, set when the left one is
//!   darker; 64 bits, left to right, top to bottom.
//! - pHash ("Looks Like It", hackerfactor.com): a 32x32 grayscale
//!   image, its DCT, the top-left 8x8 low frequencies, each bit set when the
//!   value is above the mean of those 64 values without the first term.
//!
//! Both are computed from a 64x64 grayscale thumbnail (transparency over
//! white), so the eight rotations and flips can be hashed cheaply too.

use image::metadata::Orientation;
use image::DynamicImage;

/// Side of the grayscale thumbnail the hashes are taken from.
pub const THUMB: usize = 64;

/// A square grayscale image, row by row, values 0-255.
#[derive(Clone, Debug, PartialEq)]
pub struct Gray {
    pub side: usize,
    pub px: Vec<f32>,
}

/// One image's hashes, for its eight orientations when asked for (index 0
/// is the image as it is).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hashes {
    pub dhash: Vec<u64>,
    pub phash: Vec<u64>,
}

/// Luma (ITU-R BT.601 weights), 0-255, of a pixel laid over white; the
/// channels and alpha are 0-1.
fn luma_over_white(r: f32, g: f32, b: f32, a: f32) -> f32 {
    let a = a.clamp(0.0, 1.0);
    let c = |v: f32| v.clamp(0.0, 1.0) * a + (1.0 - a);
    255.0 * (0.299 * c(r) + 0.587 * c(g) + 0.114 * c(b))
}

fn luma8(p: [u8; 4]) -> f32 {
    let f = |v: u8| v as f32 / 255.0;
    luma_over_white(f(p[0]), f(p[1]), f(p[2]), f(p[3]))
}

fn luma16(p: [u16; 4]) -> f32 {
    let f = |v: u16| v as f32 / 65535.0;
    luma_over_white(f(p[0]), f(p[1]), f(p[2]), f(p[3]))
}

/// Area-average resampling of a w x h grid to nw x nh: every source pixel
/// contributes in proportion to the area it covers in each target pixel.
pub fn resample(src: &[f32], w: usize, h: usize, nw: usize, nh: usize) -> Vec<f32> {
    let (sx, sy) = (w as f64 / nw as f64, h as f64 / nh as f64);
    let mut out = vec![0f32; nw * nh];
    for ty in 0..nh {
        let (y0, y1) = (ty as f64 * sy, (ty + 1) as f64 * sy);
        for tx in 0..nw {
            let (x0, x1) = (tx as f64 * sx, (tx + 1) as f64 * sx);
            let (mut sum, mut area) = (0f64, 0f64);
            for y in y0.floor() as usize..(y1.ceil() as usize).min(h) {
                let wy = (y1.min(y as f64 + 1.0) - y0.max(y as f64)).max(0.0);
                for x in x0.floor() as usize..(x1.ceil() as usize).min(w) {
                    let wx = (x1.min(x as f64 + 1.0) - x0.max(x as f64)).max(0.0);
                    sum += src[y * w + x] as f64 * wx * wy;
                    area += wx * wy;
                }
            }
            out[ty * nw + tx] = if area > 0.0 { (sum / area) as f32 } else { 0.0 };
        }
    }
    out
}

/// Calls `f(target, weight)` for each cell of an `n`-long axis that source
/// index `x` of a `w`-long axis overlaps, the weight in target cells (each
/// target cell's weights add up to 1).
fn span(x: usize, w: usize, n: usize, mut f: impl FnMut(usize, f64)) {
    let scale = n as f64 / w as f64;
    let (a, b) = (x as f64 * scale, (x + 1) as f64 * scale);
    for t in a.floor() as usize..(b.ceil() as usize).min(n) {
        let wt = b.min(t as f64 + 1.0) - a.max(t as f64);
        if wt > 0.0 {
            f(t, wt);
        }
    }
}

/// Area average of a w x h image, given pixel by pixel, down (or up) to the
/// thumbnail, in one pass, without a copy of the image and with memory that
/// does not grow with its size.
fn area_average(w: usize, h: usize, mut pixel: impl FnMut(usize, usize) -> f32) -> Gray {
    let mut acc = vec![0f64; THUMB * THUMB];
    let mut row = [0f64; THUMB];
    for y in 0..h {
        row.fill(0.0);
        for x in 0..w {
            let v = pixel(x, y) as f64;
            span(x, w, THUMB, |t, wt| row[t] += v * wt);
        }
        span(y, h, THUMB, |ty, wy| {
            for (a, r) in acc[ty * THUMB..(ty + 1) * THUMB].iter_mut().zip(&row) {
                *a += r * wy;
            }
        });
    }
    Gray {
        side: THUMB,
        px: acc.into_iter().map(|v| v as f32).collect(),
    }
}

/// The image as a 64x64 grayscale thumbnail: every pixel laid over white
/// first, then averaged by area, so color hidden under transparent pixels
/// does not count.
pub fn thumbnail(img: &DynamicImage) -> Gray {
    use DynamicImage as D;
    let (w, h) = (img.width() as usize, img.height() as usize);
    match img {
        D::ImageRgb8(b) => area_average(w, h, |x, y| {
            let [r, g, bl] = b.get_pixel(x as u32, y as u32).0;
            luma8([r, g, bl, 255])
        }),
        D::ImageRgba8(b) => area_average(w, h, |x, y| luma8(b.get_pixel(x as u32, y as u32).0)),
        D::ImageLuma8(b) => area_average(w, h, |x, y| b.get_pixel(x as u32, y as u32).0[0] as f32),
        D::ImageLumaA8(b) => area_average(w, h, |x, y| {
            let [l, a] = b.get_pixel(x as u32, y as u32).0;
            luma8([l, l, l, a])
        }),
        D::ImageRgb16(b) => area_average(w, h, |x, y| {
            let [r, g, bl] = b.get_pixel(x as u32, y as u32).0;
            luma16([r, g, bl, u16::MAX])
        }),
        D::ImageRgba16(b) => area_average(w, h, |x, y| luma16(b.get_pixel(x as u32, y as u32).0)),
        D::ImageLuma16(b) => area_average(w, h, |x, y| {
            b.get_pixel(x as u32, y as u32).0[0] as f32 * 255.0 / 65535.0
        }),
        D::ImageLumaA16(b) => area_average(w, h, |x, y| {
            let [l, a] = b.get_pixel(x as u32, y as u32).0;
            luma16([l, l, l, a])
        }),
        D::ImageRgb32F(b) => area_average(w, h, |x, y| {
            let [r, g, bl] = b.get_pixel(x as u32, y as u32).0;
            luma_over_white(r, g, bl, 1.0)
        }),
        D::ImageRgba32F(b) => area_average(w, h, |x, y| {
            let [r, g, bl, a] = b.get_pixel(x as u32, y as u32).0;
            luma_over_white(r, g, bl, a)
        }),
        // Kinds added to the image crate later: brought to 8 bits first.
        other => {
            let b = other.to_rgba8();
            area_average(w, h, |x, y| luma8(b.get_pixel(x as u32, y as u32).0))
        }
    }
}

/// The index of `orient` that shows an image the way its Exif orientation
/// says it is meant to be seen.
pub fn exif_turn(o: Orientation) -> usize {
    match o {
        Orientation::NoTransforms => 0,
        Orientation::Rotate90 => 1,
        Orientation::Rotate180 => 2,
        Orientation::Rotate270 => 3,
        Orientation::FlipHorizontal => 4,
        Orientation::Rotate90FlipH => 5,
        Orientation::FlipVertical => 6,
        Orientation::Rotate270FlipH => 7,
    }
}

/// The eight orientations: k quarter turns clockwise, then for 4-7 a
/// left-right mirror of the result.
pub fn orient(g: &Gray, k: usize) -> Gray {
    let n = g.side;
    let mut px = vec![0f32; n * n];
    for y in 0..n {
        for x in 0..n {
            let (mut sx, mut sy) = (x, y);
            if k >= 4 {
                sx = n - 1 - sx;
            }
            for _ in 0..k % 4 {
                // One quarter turn clockwise, read back from the target.
                let (ox, oy) = (sy, n - 1 - sx);
                sx = ox;
                sy = oy;
            }
            px[y * n + x] = g.px[sy * n + sx];
        }
    }
    Gray { side: n, px }
}

pub fn dhash(g: &Gray) -> u64 {
    let s = resample(&g.px, g.side, g.side, 9, 8);
    let mut bits = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            bits <<= 1;
            if s[y * 9 + x] < s[y * 9 + x + 1] {
                bits |= 1;
            }
        }
    }
    bits
}

pub fn phash(g: &Gray) -> u64 {
    let n = 32;
    let s = resample(&g.px, g.side, g.side, n, n);
    // Only the 8x8 lowest frequencies of the 32x32 DCT-II are needed.
    let cos: Vec<f64> = (0..8)
        .flat_map(|u| {
            (0..n).map(move |x| {
                ((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / (2 * n) as f64).cos()
            })
        })
        .collect();
    let mut coef = [0f64; 64];
    for v in 0..8 {
        for u in 0..8 {
            let mut sum = 0f64;
            for y in 0..n {
                let cy = cos[v * n + y];
                for x in 0..n {
                    sum += s[y * n + x] as f64 * cos[u * n + x] * cy;
                }
            }
            coef[v * 8 + u] = sum;
        }
    }
    let mean = coef[1..].iter().sum::<f64>() / 63.0;
    coef.iter()
        .fold(0u64, |bits, &c| (bits << 1) | (c > mean) as u64)
}

/// Hashes of a thumbnail, in all eight orientations when `orientations` is
/// set (index 0 as it is). An orientation that looks the same as an earlier
/// one, as in a symmetric picture, still gets its place in the list.
pub fn hashes_of(g: &Gray, orientations: bool) -> Hashes {
    let ks = if orientations { 0..8 } else { 0..1 };
    let mut h = Hashes {
        dhash: Vec::new(),
        phash: Vec::new(),
    };
    for k in ks {
        let o = if k == 0 { g.clone() } else { orient(g, k) };
        h.dhash.push(dhash(&o));
        h.phash.push(phash(&o));
    }
    h
}

/// Hashes of an image as its pixels are stored (no Exif orientation).
pub fn hashes(img: &DynamicImage, orientations: bool) -> Hashes {
    hashes_of(&thumbnail(img), orientations)
}

pub fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}
