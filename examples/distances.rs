// Distances between each given image and edited copies of it, and the closest
// pairs among the images themselves. A tool for choosing thresholds:
//   cargo run --release --example distances -- photos/*.jpg
use dupsight::hash::{distance, hashes};
use image::{imageops::FilterType, DynamicImage};

fn jpeg(img: &DynamicImage, q: u8) -> DynamicImage {
    let mut buf = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q);
    img.to_rgb8().write_with_encoder(enc).unwrap();
    image::load_from_memory(&buf).unwrap()
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    let imgs: Vec<DynamicImage> = paths.iter().map(|p| image::open(p).unwrap()).collect();
    for (p, base) in paths.iter().zip(&imgs) {
        let (w, h) = (base.width(), base.height());
        let variants: Vec<(&str, DynamicImage)> = vec![
            (
                "quarter",
                base.resize_exact(w / 4, h / 4, FilterType::Triangle),
            ),
            ("jpeg30", jpeg(base, 30)),
            ("bright", base.brighten(30)),
            ("contrast", base.adjust_contrast(20.0)),
            ("blur", base.blur(2.0)),
            (
                "crop5%",
                base.crop_imm(w / 40, h / 40, w - w / 20, h - h / 20),
            ),
            (
                "crop10%",
                base.crop_imm(w / 20, h / 20, w - w / 10, h - h / 10),
            ),
            ("gray", DynamicImage::ImageLuma8(base.to_luma8())),
        ];
        let hb = hashes(base, false);
        let row: Vec<String> = variants
            .iter()
            .map(|(n, v)| {
                let hv = hashes(v, false);
                format!(
                    "{n}={}/{}",
                    distance(hb.dhash[0], hv.dhash[0]),
                    distance(hb.phash[0], hv.phash[0])
                )
            })
            .collect();
        println!("{:36} {}", p.rsplit('/').next().unwrap(), row.join(" "));
    }
    let hs: Vec<_> = imgs.iter().map(|i| hashes(i, false)).collect();
    let mut pairs = Vec::new();
    for i in 0..hs.len() {
        for j in i + 1..hs.len() {
            pairs.push((
                distance(hs[i].dhash[0], hs[j].dhash[0]),
                distance(hs[i].phash[0], hs[j].phash[0]),
                i,
                j,
            ));
        }
    }
    pairs.sort_by_key(|p| p.0 + p.1);
    for p in pairs.iter().take(10) {
        println!(
            "pair d={:2} p={:2} {} {}",
            p.0,
            p.1,
            paths[p.2].rsplit('/').next().unwrap(),
            paths[p.3].rsplit('/').next().unwrap()
        );
    }
}
