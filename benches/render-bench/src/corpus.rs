//! Synthetic test images with photographic statistics.
//!
//! Real photos cannot be committed to the repository, and a flat-colour test
//! pattern would be dishonest: it compresses to almost nothing, so a JPEG
//! decode benchmark over it measures the file header rather than the entropy
//! decode. The generator below builds smooth lighting ramps, a mid-frequency
//! subject and per-pixel grain, which lands JPEG quality-85 output in the same
//! bits-per-pixel range as a camera file.

use std::path::{Path, PathBuf};

use image::{ImageEncoder, RgbImage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixture {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    /// Formats generated for this size.
    pub formats: &'static [Format],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
        }
    }
}

/// Sizes chosen to bracket what a viewer is actually opened on: a phone or
/// mid-range camera frame, a full-frame camera frame, and a high-resolution
/// body whose decoded surface alone exceeds most laptops' free memory.
pub const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "12mp",
        width: 4000,
        height: 3000,
        formats: &[Format::Jpeg, Format::Png],
    },
    Fixture {
        name: "24mp",
        width: 6000,
        height: 4000,
        formats: &[Format::Jpeg, Format::Png],
    },
    Fixture {
        name: "61mp",
        width: 9504,
        height: 6336,
        formats: &[Format::Jpeg],
    },
];

impl Fixture {
    pub fn megapixels(&self) -> f64 {
        f64::from(self.width) * f64::from(self.height) / 1_000_000.0
    }

    /// Bytes an engine must hold to keep this image decoded, at 4 bytes per
    /// pixel. This is the number that decides whether a viewer survives a
    /// 61-megapixel file.
    pub fn decoded_rgba_bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4
    }

    pub fn path(&self, directory: &Path, format: Format) -> PathBuf {
        directory.join(format!("{}.{}", self.name, format.extension()))
    }
}

pub fn corpus_directory() -> PathBuf {
    std::env::var_os("MANZAR_BENCH_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("manzar-render-bench-corpus"))
}

/// Generates every fixture that is not already on disk and returns the
/// directory holding them. Generation is deterministic, so a cached corpus and
/// a freshly built one produce identical measurements.
pub fn ensure_corpus() -> std::io::Result<PathBuf> {
    let directory = corpus_directory();
    std::fs::create_dir_all(&directory)?;

    for fixture in FIXTURES {
        let missing: Vec<Format> = fixture
            .formats
            .iter()
            .copied()
            .filter(|format| !fixture.path(&directory, *format).exists())
            .collect();

        if missing.is_empty() {
            continue;
        }

        eprintln!(
            "generating {} ({}x{}, {:.1} MP)…",
            fixture.name, fixture.width, fixture.height, fixture.megapixels()
        );
        let image = synthesise(fixture.width, fixture.height);

        for format in missing {
            let path = fixture.path(&directory, format);
            match format {
                Format::Jpeg => {
                    let file = std::fs::File::create(&path)?;
                    let writer = std::io::BufWriter::new(file);
                    image::codecs::jpeg::JpegEncoder::new_with_quality(writer, 85)
                        .write_image(
                            image.as_raw(),
                            image.width(),
                            image.height(),
                            image::ExtendedColorType::Rgb8,
                        )
                        .map_err(std::io::Error::other)?;
                }
                Format::Png => {
                    let file = std::fs::File::create(&path)?;
                    let writer = std::io::BufWriter::new(file);
                    image::codecs::png::PngEncoder::new_with_quality(
                        writer,
                        image::codecs::png::CompressionType::Default,
                        image::codecs::png::FilterType::Adaptive,
                    )
                    .write_image(
                        image.as_raw(),
                        image.width(),
                        image.height(),
                        image::ExtendedColorType::Rgb8,
                    )
                    .map_err(std::io::Error::other)?;
                }
            }
        }
    }

    Ok(directory)
}

/// Builds one deterministic frame: a broad lighting gradient, an elliptical
/// subject with mid-frequency detail, and fine grain. The grain is what keeps
/// the entropy coder honest.
fn synthesise(width: u32, height: u32) -> RgbImage {
    let mut buffer = vec![0u8; (width as usize) * (height as usize) * 3];
    let (fw, fh) = (f64::from(width), f64::from(height));

    for y in 0..height {
        let ny = f64::from(y) / fh;
        let row = (y as usize) * (width as usize) * 3;

        for x in 0..width {
            let nx = f64::from(x) / fw;

            // Broad sky-to-ground ramp.
            let ramp = 0.35 + 0.5 * (1.0 - ny);

            // Subject: an off-centre ellipse carrying most of the mid
            // frequencies, which is where a real photo's DCT energy sits.
            let dx = (nx - 0.42) * 1.6;
            let dy = (ny - 0.55) * 2.1;
            let radius = (dx * dx + dy * dy).sqrt();
            let subject = (-radius * 3.2).exp();

            // Mid-frequency texture, scaled with the image so every fixture has
            // comparable detail per unit of frame rather than per pixel.
            let texture = ((nx * 47.0).sin() * (ny * 61.0).cos()
                + (nx * 113.0 + ny * 89.0).sin() * 0.5)
                * 0.06;

            let grain = f64::from(hash_noise(x, y)) / 255.0 - 0.5;

            let base = ramp + subject * 0.4 + texture + grain * 0.05;
            let index = row + (x as usize) * 3;
            buffer[index] = channel(base * 1.04 + subject * 0.10);
            buffer[index + 1] = channel(base * 0.98);
            buffer[index + 2] = channel(base * 0.90 - subject * 0.08);
        }
    }

    RgbImage::from_raw(width, height, buffer).expect("buffer sized for dimensions")
}

fn channel(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A cheap integer hash, used instead of an RNG so the corpus is reproducible
/// across machines and runs without carrying a dependency.
fn hash_noise(x: u32, y: u32) -> u8 {
    let mut h = x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    (h & 0xFF) as u8
}
