//! Observe complete shipping Rust operations. Fixture creation and crop reset
//! are outside the timer; crop timing includes decode, encode, verification and
//! atomic replacement. No webview timing is inferred from these measurements.
use std::{path::Path, time::Instant};

use image::{codecs::png::PngEncoder, ImageEncoder, RgbImage};
use manzar_lib::core::{
    crop::{crop_image, CropRect},
    image_protocol::serve_media_path,
};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let corpus = Path::new(&args[2]);
    std::fs::create_dir_all(corpus).unwrap();
    if args[1] == "prepare" {
        for (name, width, height, gray, oriented) in [
            ("rgb.jpg", 4000, 3000, false, false),
            ("gray.jpg", 4000, 3000, true, false),
            ("rgb.png", 4000, 3000, false, false),
            ("rotated.png", 6000, 4000, false, true),
            ("upright.png", 6000, 4000, false, false),
        ] {
            let mut noise = 0x2545_F491_4F6C_DD1Du64;
            let frame = RgbImage::from_fn(width, height, |x, y| {
                noise ^= noise << 13;
                noise ^= noise >> 7;
                noise ^= noise << 17;
                let grain = (noise >> 56) as u8 / 8;
                let fx = f64::from(x) / f64::from(width);
                let fy = f64::from(y) / f64::from(height);
                let subject = ((fx * 24.0).sin() * (fy * 18.0).cos() * 60.0 + 128.0) as u8;
                image::Rgb([
                    subject.saturating_add(grain),
                    ((fy * 200.0) as u8).saturating_add(grain),
                    ((fx * 180.0) as u8).saturating_add(grain),
                ])
            });
            let path = corpus.join(name);
            if gray {
                image::DynamicImage::ImageRgb8(frame)
                    .into_luma8()
                    .save(path)
                    .unwrap();
            } else if oriented {
                let mut encoder = PngEncoder::new(std::fs::File::create(path).unwrap());
                encoder
                    .set_exif_metadata(vec![
                        73, 73, 42, 0, 8, 0, 0, 0, 1, 0, 18, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0,
                        0, 0,
                    ])
                    .unwrap();
                frame.write_with_encoder(encoder).unwrap();
            } else {
                frame.save(path).unwrap();
            }
        }
        return;
    }
    let case = &args[1];
    let output = Path::new(&args[3]);
    let started;
    let elapsed;
    if let Some(name) = case.strip_prefix("fit-") {
        started = Instant::now();
        let response = serve_media_path(&corpus.join(name), None, Some((1920, 1440))).unwrap();
        elapsed = started.elapsed();
        std::fs::write(output, response.bytes()).unwrap();
    } else {
        let name = case.strip_prefix("crop-").unwrap();
        let path = corpus.join("working.png");
        std::fs::copy(corpus.join(name), &path).unwrap();
        started = Instant::now();
        crop_image(
            &path,
            CropRect {
                x: 100,
                y: 200,
                width: 700,
                height: 600,
            },
        )
        .unwrap();
        elapsed = started.elapsed();
        std::fs::rename(path, output).unwrap();
    }
    println!("{}", elapsed.as_secs_f64() * 1000.0);
}
