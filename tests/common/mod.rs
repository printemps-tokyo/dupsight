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

/// A JPEG whose Exif orientation tag says `value` (1-8).
pub fn jpeg_with_orientation(img: &DynamicImage, value: u16) -> Vec<u8> {
    use image::ImageEncoder;
    // A big-endian TIFF header and one IFD entry: 0x0112 Orientation, SHORT.
    let mut exif = b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
    exif.extend_from_slice(&value.to_be_bytes());
    exif.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
    enc.set_exif_metadata(exif).unwrap();
    let rgb = img.to_rgb8();
    enc.write_image(
        &rgb,
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )
    .unwrap();
    buf
}
