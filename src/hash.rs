//! Perceptual hashes, after Neal Krawetz's descriptions:
//! - dHash ("Kind of Like That", hackerfactor.com, 2013): a 9x8 grayscale
//!   image, one bit per pair of horizontal neighbors, set when the left one is
//!   darker; 64 bits, left to right, top to bottom.
//! - pHash ("Looks Like It", hackerfactor.com, 2011): a 32x32 grayscale
//!   image, its DCT, the top-left 8x8 low frequencies, each bit set when the
//!   value is above the mean of those 64 values without the first term.
//!
//! Both are computed from a 64x64 grayscale thumbnail (transparency over
//! white), so the eight rotations and flips can be hashed cheaply too.

use image::{DynamicImage, GenericImageView};

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

/// Luma (ITU-R BT.601 weights) of a pixel laid over white.
fn luma_over_white(p: [u8; 4]) -> f32 {
    let a = p[3] as f32 / 255.0;
    let c = |v: u8| v as f32 * a + 255.0 * (1.0 - a);
    0.299 * c(p[0]) + 0.587 * c(p[1]) + 0.114 * c(p[2])
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

/// The image as a 64x64 grayscale thumbnail. Large images are first brought
/// down with the image crate's fast thumbnail filter, then averaged by area.
pub fn thumbnail(img: &DynamicImage) -> Gray {
    let (w, h) = img.dimensions();
    let small = if w > 512 || h > 512 {
        img.thumbnail_exact(512.min(w), 512.min(h))
    } else {
        img.clone()
    };
    let rgba = small.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let luma: Vec<f32> = rgba.pixels().map(|p| luma_over_white(p.0)).collect();
    Gray {
        side: THUMB,
        px: resample(&luma, w, h, THUMB, THUMB),
    }
}

/// The eight orientations: k quarter turns clockwise, then a mirror for 4-7.
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

/// Hashes of an image, in all eight orientations when `orientations` is set.
pub fn hashes(img: &DynamicImage, orientations: bool) -> Hashes {
    let g = thumbnail(img);
    let ks = if orientations { 0..8 } else { 0..1 };
    let mut h = Hashes {
        dhash: Vec::new(),
        phash: Vec::new(),
    };
    for k in ks {
        let o = if k == 0 { g.clone() } else { orient(&g, k) };
        h.dhash.push(dhash(&o));
        h.phash.push(phash(&o));
    }
    h
}

pub fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}
