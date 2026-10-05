//! Isolated measurements for the Rust performance review. No webview is modeled.
//! Run: cargo run --release --bin rust_review -- rust-review-report.json
use fast_image_resize::{
    images::{Image, ImageRef},
    FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer,
};
use image::{
    codecs::{
        bmp::BmpEncoder,
        jpeg::{JpegDecoder, JpegEncoder},
    },
    ExtendedColorType, ImageDecoder, RgbImage,
};
use serde::Serialize;
use std::{
    cell::Cell,
    hint::black_box,
    io::{self, BufRead, Cursor, Read, Seek, SeekFrom},
    rc::Rc,
    time::Instant,
};

const SOURCE: (u32, u32) = (4000, 3000);
const TARGET: (u32, u32) = (1920, 1440);

#[derive(Serialize)]
struct Measurement {
    name: &'static str,
    baseline_ms: f64,
    candidate_ms: f64,
    ratio: f64,
}

fn median(mut operation: impl FnMut()) -> f64 {
    operation();
    let mut samples = Vec::new();
    for _ in 0..15 {
        let started = Instant::now();
        operation();
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn compare(
    rows: &mut Vec<Measurement>,
    name: &'static str,
    baseline: impl FnMut(),
    candidate: impl FnMut(),
) {
    let baseline_ms = median(baseline);
    let candidate_ms = median(candidate);
    println!(
        "{name}: {baseline_ms:.3} -> {candidate_ms:.3} ms ({:.2}x)",
        baseline_ms / candidate_ms
    );
    rows.push(Measurement {
        name,
        baseline_ms,
        candidate_ms,
        ratio: baseline_ms / candidate_ms,
    });
}

fn fixture() -> RgbImage {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    RgbImage::from_fn(SOURCE.0, SOURCE.1, |x, y| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let grain = (state >> 56) as u8 / 8;
        image::Rgb([
            (x % 256) as u8,
            (y % 256) as u8,
            grain.saturating_add(((x + y) % 200) as u8),
        ])
    })
}

fn resize(frame: &RgbImage, target: (u32, u32)) -> RgbImage {
    let source = ImageRef::new(
        frame.width(),
        frame.height(),
        frame.as_raw(),
        PixelType::U8x3,
    )
    .unwrap();
    let mut destination = Image::new(target.0, target.1, PixelType::U8x3);
    Resizer::new()
        .resize(
            &source,
            &mut destination,
            &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)),
        )
        .unwrap();
    RgbImage::from_raw(target.0, target.1, destination.into_vec()).unwrap()
}

fn bmp(frame: &RgbImage, reserve: bool) -> Vec<u8> {
    let stride = (frame.width() as usize * 3 + 3) & !3;
    let mut bytes = if reserve {
        Vec::with_capacity(54 + stride * frame.height() as usize)
    } else {
        Vec::new()
    };
    BmpEncoder::new(&mut bytes)
        .encode(
            frame.as_raw(),
            frame.width(),
            frame.height(),
            ExtendedColorType::Rgb8,
        )
        .unwrap();
    bytes
}

// Narrow safe prototype: RGB8 BMP with the same header, row order and padding
// as image's encoder. Byte-equivalence checks below cover odd-width padding.
fn direct_bmp(frame: &RgbImage) -> Vec<u8> {
    let stride = (frame.width() as usize * 3 + 3) & !3;
    let mut bytes = vec![0; 54 + stride * frame.height() as usize];
    let file_size = u32::try_from(bytes.len()).unwrap();
    bytes[0..2].copy_from_slice(b"BM");
    bytes[2..6].copy_from_slice(&file_size.to_le_bytes());
    bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
    bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
    bytes[18..22].copy_from_slice(&frame.width().to_le_bytes());
    bytes[22..26].copy_from_slice(&frame.height().to_le_bytes());
    bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
    bytes[34..38].copy_from_slice(
        &u32::try_from(stride * frame.height() as usize)
            .unwrap()
            .to_le_bytes(),
    );
    for (src, dst) in frame
        .as_raw()
        .chunks_exact(frame.width() as usize * 3)
        .rev()
        .zip(bytes[54..].chunks_exact_mut(stride))
    {
        for (rgb, bgr) in src.chunks_exact(3).zip(dst.chunks_exact_mut(3)) {
            bgr.copy_from_slice(&[rgb[2], rgb[1], rgb[0]]);
        }
    }
    bytes
}

struct CountRead<R> {
    reader: R,
    bytes: Rc<Cell<usize>>,
}
impl<R: Read> Read for CountRead<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let n = self.reader.read(output)?;
        self.bytes.set(self.bytes.get() + n);
        Ok(n)
    }
}
impl<R: BufRead> BufRead for CountRead<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.reader.fill_buf()
    }
    fn consume(&mut self, n: usize) {
        self.bytes.set(self.bytes.get() + n);
        self.reader.consume(n);
    }
}
impl<R: Seek> Seek for CountRead<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.reader.seek(from)
    }
}

fn decode_scaled(bytes: &[u8], reject_before_decode: bool) -> Option<RgbImage> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    decoder.read_info().unwrap();
    let info = decoder.info().unwrap();
    if reject_before_decode && info.pixel_format != jpeg_decoder::PixelFormat::RGB24 {
        return None;
    }
    let (w, h) = decoder
        .scale(info.width.div_ceil(2), info.height.div_ceil(2))
        .unwrap();
    let pixels = decoder.decode().unwrap();
    if decoder.info().unwrap().pixel_format != jpeg_decoder::PixelFormat::RGB24 {
        return None;
    }
    Some(RgbImage::from_raw(u32::from(w), u32::from(h), pixels).unwrap())
}

fn main() {
    let frame = fixture();
    let fitted = resize(&frame, TARGET);
    for w in [1, 2, 3, 5, 8] {
        let sample = RgbImage::from_fn(w, 7, |x, y| image::Rgb([x as u8, y as u8, 190]));
        assert_eq!(bmp(&sample, false), direct_bmp(&sample));
        assert_eq!(
            image::load_from_memory(&direct_bmp(&sample))
                .unwrap()
                .into_rgb8(),
            sample
        );
    }
    assert_eq!(bmp(&fitted, false), direct_bmp(&fitted));

    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode(frame.as_raw(), SOURCE.0, SOURCE.1, ExtendedColorType::Rgb8)
        .unwrap();
    let consumed = Rc::new(Cell::new(0));
    let mut decoder = JpegDecoder::new(CountRead {
        reader: Cursor::new(&jpeg),
        bytes: Rc::clone(&consumed),
    })
    .unwrap();
    decoder.orientation().unwrap();
    decoder.icc_profile().unwrap();
    let image_probe_read_bytes = consumed.get();
    consumed.set(0);
    let mut decoder = jpeg_decoder::Decoder::new(CountRead {
        reader: Cursor::new(&jpeg),
        bytes: Rc::clone(&consumed),
    });
    decoder.read_info().unwrap();
    let dimensions_only_read_bytes = consumed.get();
    assert_eq!(image_probe_read_bytes, jpeg.len());
    println!("JPEG file {} bytes; image metadata constructor reads {} bytes; jpeg-decoder dimensions reads {} bytes (not equivalent metadata)", jpeg.len(), image_probe_read_bytes, dimensions_only_read_bytes);

    let mut rows = Vec::new();
    compare(
        &mut rows,
        "bmp_preallocation",
        || {
            black_box(bmp(&fitted, false));
        },
        || {
            black_box(bmp(&fitted, true));
        },
    );
    compare(
        &mut rows,
        "bmp_direct_write",
        || {
            black_box(bmp(&fitted, true));
        },
        || {
            black_box(direct_bmp(&fitted));
        },
    );

    let source = ImageRef::new(SOURCE.0, SOURCE.1, frame.as_raw(), PixelType::U8x3).unwrap();
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    let mut resizer = Resizer::new();
    let mut destination = Image::new(TARGET.0, TARGET.1, PixelType::U8x3);
    compare(
        &mut rows,
        "resize_reuse_scratch_and_destination",
        || {
            let mut destination = Image::new(TARGET.0, TARGET.1, PixelType::U8x3);
            Resizer::new()
                .resize(&source, &mut destination, &options)
                .unwrap();
            black_box(destination);
        },
        || {
            resizer.resize(&source, &mut destination, &options).unwrap();
            black_box(destination.buffer());
        },
    );

    let first_rotate = resize(&image::imageops::rotate90(&frame), (TARGET.1, TARGET.0));
    let last_rotate = image::imageops::rotate90(&resize(&frame, TARGET));
    let max_orientation_pixel_difference = first_rotate
        .as_raw()
        .iter()
        .zip(last_rotate.as_raw())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    println!("Resize/orientation reordered maximum RGB channel difference: {max_orientation_pixel_difference}");
    compare(
        &mut rows,
        "orientation_after_resize",
        || {
            black_box(resize(
                &image::imageops::rotate90(&frame),
                (TARGET.1, TARGET.0),
            ));
        },
        || {
            black_box(image::imageops::rotate90(&resize(&frame, TARGET)));
        },
    );

    compare(
        &mut rows,
        "rgb_jpeg_scaled_vs_full_decode_and_resize",
        || {
            black_box(resize(&decode_scaled(&jpeg, false).unwrap(), TARGET));
        },
        || {
            black_box(resize(
                &image::load_from_memory(&jpeg).unwrap().into_rgb8(),
                TARGET,
            ));
        },
    );

    let scaled_frame = decode_scaled(&jpeg, false).unwrap();
    compare(
        &mut rows,
        "orientation_after_resize_on_scaled_jpeg_frame",
        || {
            black_box(resize(
                &image::imageops::rotate90(&scaled_frame),
                (TARGET.1, TARGET.0),
            ));
        },
        || {
            black_box(image::imageops::rotate90(&resize(&scaled_frame, TARGET)));
        },
    );

    let crop_before =
        image::imageops::crop_imm(&image::imageops::rotate90(&frame), 200, 400, 700, 600)
            .to_image();
    let crop_after = image::imageops::rotate90(
        &image::imageops::crop_imm(&frame, 400, SOURCE.1 - 200 - 700, 600, 700).to_image(),
    );
    assert_eq!(crop_before, crop_after);
    compare(
        &mut rows,
        "crop_before_orientation",
        || {
            black_box(
                image::imageops::crop_imm(&image::imageops::rotate90(&frame), 200, 400, 700, 600)
                    .to_image(),
            );
        },
        || {
            black_box(image::imageops::rotate90(
                &image::imageops::crop_imm(&frame, 400, SOURCE.1 - 200 - 700, 600, 700).to_image(),
            ));
        },
    );

    let gray = image::DynamicImage::ImageRgb8(frame).into_luma8();
    let mut gray_jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut gray_jpeg, 85)
        .encode(gray.as_raw(), SOURCE.0, SOURCE.1, ExtendedColorType::L8)
        .unwrap();
    assert!(decode_scaled(&gray_jpeg, true).is_none());
    assert!(decode_scaled(&gray_jpeg, false).is_none());
    compare(
        &mut rows,
        "grayscale_reject_before_decode",
        || {
            assert!(decode_scaled(&gray_jpeg, false).is_none());
            black_box(image::load_from_memory(&gray_jpeg).unwrap().into_rgb8());
        },
        || {
            assert!(decode_scaled(&gray_jpeg, true).is_none());
            black_box(image::load_from_memory(&gray_jpeg).unwrap().into_rgb8());
        },
    );

    let report = serde_json::json!({
        "method": "one warmup then 15 samples, medians, isolated CPU stages; no disk or webview timings",
        "rustc": env!("BENCH_RUSTC_VERSION"), "source": SOURCE, "target": TARGET,
        "jpeg_bytes": jpeg.len(), "image_probe_read_bytes": image_probe_read_bytes,
        "dimensions_only_read_bytes": dimensions_only_read_bytes,
        "max_orientation_pixel_difference": max_orientation_pixel_difference,
        "retained_resizer_scratch_bytes": resizer.size_of_internal_buffers(), "rows": rows
    });
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rust-review-report.json".into());
    std::fs::write(&output, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!("wrote {output}");
}
