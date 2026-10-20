#![allow(dead_code)]

use image::{DynamicImage, ImageBuffer, Rgb};

/// A picture of random colored blocks with a soft gradient over it, the
/// same for the same seed. Different seeds give unrelated pictures.
pub fn picture(seed: u64, w: u32, h: u32) -> DynamicImage {
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as u32
    };
    let (cols, rows) = (12u32, 9u32);
    let cells: Vec<[u8; 3]> = (0..cols * rows)
        .map(|_| [next() as u8, next() as u8, next() as u8])
        .collect();
    let img = ImageBuffer::from_fn(w, h, |x, y| {
        let c = cells[((y * rows / h) * cols + x * cols / w) as usize];
        let shade = (x + y) as f32 / (w + h) as f32 * 40.0;
        Rgb([
            (c[0] as f32 * 0.8 + shade) as u8,
            (c[1] as f32 * 0.8 + shade) as u8,
            (c[2] as f32 * 0.8 + shade) as u8,
        ])
    });
    DynamicImage::ImageRgb8(img)
}

pub fn jpeg(img: &DynamicImage, quality: u8) -> Vec<u8> {
    let mut buf = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    img.to_rgb8().write_with_encoder(enc).unwrap();
    buf
}

pub fn png(img: &DynamicImage) -> Vec<u8> {
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    buf
}
