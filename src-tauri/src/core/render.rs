//! Decode-and-fit: hand the webview only the pixels the window can light.
//!
//! Manzar used to read an image whole and let the engine decode every pixel of
//! it, then scale the full surface down to a window that is usually a fraction
//! of the size. This module does the decode and the scale in Rust instead, at
//! the smallest DCT scale that still covers the window, and hands over one
//! window-sized surface.
//!
//! Declining is always safe and always correct. Anything this returns `None`
//! for is served as the original file — exactly what shipped before this
//! module existed — so animated GIF, colour-managed images and files already
//! smaller than the window behave as they always did.

use super::{crop::still_decoder, metadata_preflight::MAX_SAFE_DECODED_RGBA_BYTES};
use std::{
    collections::VecDeque,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

use fast_image_resize::{
    images::{Image as FirImage, ImageRef},
    FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer,
};
use image::{
    codecs::bmp::BmpEncoder, metadata::Orientation, DynamicImage, ExtendedColorType, GrayImage,
    ImageDecoder, ImageFormat, RgbImage,
};

/// Below 1/2 there is no DCT scale to take, and taking one anyway is a loss:
/// `jpeg-decoder` is scalar where the `image` crate's JPEG backend is AVX2, so
/// a 1/1 "scaled" decode pays the slower decoder for nothing. Measured in
/// `benches/render-bench`.
const DCT_DIVISOR_WORTH_SCALING: u32 = 2;

/// BMP, because the encode is a header plus a row copy and the bytes never
/// leave the process. PNG would compress a window-sized frame to a third of the
/// size and cost more than the decode this module just saved.
const FITTED_MIME_TYPE: &str = "image/bmp";

pub struct FittedImage {
    pub bytes: Vec<u8>,
    pub mime_type: &'static str,
}

// Cache only window-sized surfaces, never full decoded originals. A byte cap
// matters more than an image count when a viewer moves between monitors.
const FIT_CACHE_BYTES: usize = 64 * 1024 * 1024;
const FIT_CACHE_ENTRIES: usize = 64;
static FIT_CACHE: OnceLock<Mutex<VecDeque<CachedFit>>> = OnceLock::new();

struct CachedFit {
    path: PathBuf,
    viewport: (u32, u32),
    fingerprint: Fingerprint,
    image: FittedImage,
}

#[derive(PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl Fingerprint {
    fn for_path(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok()?,
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            },
        })
    }
}

struct Probe {
    width: u32,
    height: u32,
    orientation: Orientation,
    icc_profile: Option<Vec<u8>>,
}

impl Probe {
    /// The size the image is *displayed* at, which is the source size with the
    /// axes swapped when EXIF asks for a quarter turn.
    fn displayed_size(&self) -> (u32, u32) {
        if self.swaps_axes() {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }

    fn swaps_axes(&self) -> bool {
        matches!(
            self.orientation,
            Orientation::Rotate90
                | Orientation::Rotate270
                | Orientation::Rotate90FlipH
                | Orientation::Rotate270FlipH
        )
    }
}

/// Decodes `path` no larger than `viewport` needs and encodes it for transport.
///
/// `None` means "serve the original file": either there is nothing to gain or
/// fitting would change what the user sees.
pub fn fit_image(path: &Path, viewport: (u32, u32)) -> Option<FittedImage> {
    fit_image_if_current(path, viewport, &|| true)
}

pub fn fit_image_if_current(
    path: &Path,
    viewport: (u32, u32),
    is_current: &impl Fn() -> bool,
) -> Option<FittedImage> {
    if !is_current() {
        return None;
    }
    let fingerprint = Fingerprint::for_path(path)?;
    let cache = FIT_CACHE.get_or_init(Mutex::default);
    if let Ok(mut entries) = cache.lock() {
        // Also remove obsolete versions so overwriting a file cannot grow the
        // working set or return an old frame with the same opaque image ID.
        entries.retain(|entry| entry.path != path || entry.fingerprint == fingerprint);
        if let Some(index) = entries
            .iter()
            .position(|entry| entry.path == path && entry.viewport == viewport)
        {
            let entry = entries.remove(index)?;
            let image = FittedImage {
                bytes: entry.image.bytes.clone(),
                mime_type: entry.image.mime_type,
            };
            entries.push_back(entry);
            return Some(image);
        }
    }
    let image = fit_uncached(path, viewport, is_current)?;
    if !is_current() {
        return None;
    }
    if image.bytes.len() <= FIT_CACHE_BYTES
        && Fingerprint::for_path(path).as_ref() == Some(&fingerprint)
    {
        if let Ok(mut entries) = cache.lock() {
            entries.retain(|entry| !(entry.path == path && entry.viewport == viewport));
            let mut bytes: usize = entries.iter().map(|entry| entry.image.bytes.len()).sum();
            while bytes + image.bytes.len() > FIT_CACHE_BYTES || entries.len() >= FIT_CACHE_ENTRIES
            {
                if let Some(removed) = entries.pop_front() {
                    bytes -= removed.image.bytes.len();
                }
            }
            entries.push_back(CachedFit {
                path: path.to_path_buf(),
                viewport,
                fingerprint,
                image: FittedImage {
                    bytes: image.bytes.clone(),
                    mime_type: image.mime_type,
                },
            });
        }
    }
    Some(image)
}

fn fit_uncached(
    path: &Path,
    viewport: (u32, u32),
    is_current: &impl Fn() -> bool,
) -> Option<FittedImage> {
    if viewport.0 == 0 || viewport.1 == 0 {
        return None;
    }

    // An animated GIF is more than one frame. A fitted still would silently
    // drop every frame but the first, so GIF keeps the whole-file path.
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gif"))
    {
        return None;
    }

    let (format, mut decoder) = still_decoder(path).ok()?;
    let probe = probe(decoder.as_mut())?;
    if !is_current() {
        return None;
    }

    // A profile Manzar cannot honour must reach the engine, which can.
    if probe
        .icc_profile
        .as_deref()
        .is_some_and(|profile| !is_srgb(profile))
    {
        return None;
    }

    let displayed = probe.displayed_size();
    let fitted = fit_within(displayed.0, displayed.1, viewport);

    // Already inside the window. Re-encoding could only lose.
    if fitted == displayed {
        return None;
    }

    // `fitted` is in display axes; the decoder works in source axes.
    let required = if probe.swaps_axes() {
        (fitted.1, fitted.0)
    } else {
        fitted
    };

    let mut decoded = decode_to_cover(path, format, decoder, probe.width, probe.height, required)?;
    if !is_current() {
        return None;
    }
    decoded.apply_orientation(probe.orientation);
    if !is_current() {
        return None;
    }
    let resized = match decoded {
        // All RGB channels of a grayscale image are identical. Convolve once
        // before expanding, instead of allocating and convolving three source
        // channels. Keep orientation before convolution for identical rounding.
        DynamicImage::ImageLuma8(gray) => {
            let pixels = resize_pixels(gray.as_raw(), gray.dimensions(), fitted, PixelType::U8)?;
            DynamicImage::ImageLuma8(GrayImage::from_raw(fitted.0, fitted.1, pixels)?).into_rgb8()
        }
        image => resize_rgb(&image.into_rgb8(), fitted)?,
    };
    if !is_current() {
        return None;
    }
    // The decoded frame and resizer scratch are released before BMP encoding.
    encode_bmp(&resized)
}

/// Inspect the decoder that will also perform the full decode. JPEG decoder
/// construction reads the compressed file; reusing it avoids another read/copy.
fn probe(decoder: &mut dyn ImageDecoder) -> Option<Probe> {
    // The RGB fitting path cannot retain alpha. Let the webview handle it,
    // just as it handles animation and non-sRGB colour profiles.
    if decoder.color_type().has_alpha() {
        return None;
    }
    let (width, height) = decoder.dimensions();
    Some(Probe {
        width,
        height,
        orientation: decoder.orientation().ok()?,
        icc_profile: decoder.icc_profile().ok()?,
    })
}

/// ponytail: substring match on the profile's description tag, which catches
/// the sRGB variants cameras and phones embed. Anything else — Display P3,
/// Adobe RGB — declines to the original bytes and the engine colour-manages it
/// as before, so the failure mode is "no speedup", not "wrong colours". Swap in
/// a real CMM (`qcms`, `lcms2`) only if fitted output must carry its own
/// profile.
fn is_srgb(profile: &[u8]) -> bool {
    profile.windows(4).any(|window| window == b"sRGB")
}

/// Decodes at the smallest scale that still covers `required`.
fn decode_to_cover(
    path: &Path,
    format: ImageFormat,
    decoder: Box<dyn ImageDecoder>,
    width: u32,
    height: u32,
    required: (u32, u32),
) -> Option<DynamicImage> {
    let (scaled_width, _) = dct_scale_for(width, height, required);
    let divisor = width.div_ceil(scaled_width.max(1)).max(1);

    // Only RGB8 can use the scaled decoder. Grayscale stays single-channel in
    // the general decoder; CMYK keeps the general decoder's colour conversion.
    if format == ImageFormat::Jpeg
        && decoder.color_type() == image::ColorType::Rgb8
        && divisor >= DCT_DIVISOR_WORTH_SCALING
    {
        let bytes = std::fs::read(path).ok()?;
        if let Some(frame) = decode_jpeg_at_divisor(&bytes, divisor) {
            return Some(DynamicImage::ImageRgb8(frame));
        }
    }

    // from_decoder does not apply ImageReader's allocation reservation. Keep
    // that budget explicit when consuming the already-prepared decoder.
    if decoder.total_bytes() > MAX_SAFE_DECODED_RGBA_BYTES {
        return None;
    }
    DynamicImage::from_decoder(decoder).ok()
}

/// A JPEG stores frequency coefficients, so a decoder can rebuild at 1/2, 1/4
/// or 1/8 size by dropping the high-frequency ones before the inverse
/// transform. The full-size pixels are never allocated at all.
fn decode_jpeg_at_divisor(bytes: &[u8], divisor: u32) -> Option<RgbImage> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    decoder.set_max_decoding_buffer_size(MAX_SAFE_DECODED_RGBA_BYTES as usize);
    decoder.read_info().ok()?;
    let info = decoder.info()?;

    // The format is known from the header. Reject before decoding so grayscale
    // and CMYK do not pay for pixels that the general decoder must redo.
    if info.pixel_format != jpeg_decoder::PixelFormat::RGB24 {
        return None;
    }

    let (width, height) = decoder
        .scale(
            u32::from(info.width).div_ceil(divisor).try_into().ok()?,
            u32::from(info.height).div_ceil(divisor).try_into().ok()?,
        )
        .ok()?;
    let pixels = decoder.decode().ok()?;

    RgbImage::from_raw(u32::from(width), u32::from(height), pixels)
}

/// SIMD convolution — AVX2 or NEON, dispatched at runtime. This is where the
/// bulk of the measured win comes from, not the scaled decode.
fn resize_rgb(frame: &RgbImage, target: (u32, u32)) -> Option<RgbImage> {
    let pixels = resize_pixels(frame.as_raw(), frame.dimensions(), target, PixelType::U8x3)?;
    RgbImage::from_raw(target.0, target.1, pixels)
}

fn resize_pixels(
    pixels: &[u8],
    dimensions: (u32, u32),
    target: (u32, u32),
    pixel_type: PixelType,
) -> Option<Vec<u8>> {
    let source = ImageRef::new(dimensions.0, dimensions.1, pixels, pixel_type).ok()?;
    let mut destination = FirImage::new(target.0, target.1, pixel_type);

    Resizer::new()
        .resize(
            &source,
            &mut destination,
            &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)),
        )
        .ok()?;

    Some(destination.into_vec())
}

fn encode_bmp(frame: &RgbImage) -> Option<FittedImage> {
    let mut bytes = Vec::new();

    BmpEncoder::new(&mut bytes)
        .encode(
            frame.as_raw(),
            frame.width(),
            frame.height(),
            ExtendedColorType::Rgb8,
        )
        .ok()?;

    Some(FittedImage {
        bytes,
        mime_type: FITTED_MIME_TYPE,
    })
}

/// Largest box within `target` that keeps the source aspect ratio — the
/// geometry behind "fit to window". Never upscales.
pub fn fit_within(width: u32, height: u32, target: (u32, u32)) -> (u32, u32) {
    let scale = (f64::from(target.0) / f64::from(width))
        .min(f64::from(target.1) / f64::from(height))
        .min(1.0);

    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

/// Picks the smallest of JPEG's four DCT scales whose output still covers
/// `required` in both axes, so the result is never softer than the window can
/// show.
pub fn dct_scale_for(width: u32, height: u32, required: (u32, u32)) -> (u32, u32) {
    let mut chosen = (width, height);

    for divisor in [2u32, 4, 8] {
        let candidate = (width.div_ceil(divisor), height.div_ceil(divisor));
        if candidate.0 >= required.0 && candidate.1 >= required.1 {
            chosen = candidate;
        }
    }

    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const WINDOW: (u32, u32) = (2560, 1440);

    fn write_jpeg(path: &Path, width: u32, height: u32) {
        // A gradient rather than a flat fill: a flat image compresses to almost
        // nothing and would not exercise the entropy decode at all.
        let frame = RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
        });
        frame.save(path).expect("jpeg fixture");
    }

    #[test]
    fn transparent_images_keep_the_original_alpha_channel() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("transparent.png");
        image::RgbaImage::from_pixel(100, 80, image::Rgba([50, 70, 90, 128]))
            .save(&path)
            .unwrap();
        assert!(fit_image(&path, (20, 20)).is_none());
    }

    #[test]
    fn fit_within_keeps_aspect_ratio_and_never_upscales() {
        assert_eq!(fit_within(4000, 3000, WINDOW), (1920, 1440));
        assert_eq!(fit_within(3000, 4000, WINDOW), (1080, 1440));
        // Smaller than the window: left alone rather than blown up.
        assert_eq!(fit_within(800, 600, WINDOW), (800, 600));
    }

    #[test]
    fn dct_scale_never_returns_fewer_pixels_than_the_window_needs() {
        for (width, height) in [(4000u32, 3000u32), (6000, 4000), (9504, 6336), (800, 600)] {
            let required = fit_within(width, height, WINDOW);
            let (scaled_width, scaled_height) = dct_scale_for(width, height, required);

            assert!(
                scaled_width >= required.0 && scaled_height >= required.1,
                "{width}x{height} scaled to {scaled_width}x{scaled_height}, \
                 under the {}x{} the window needs",
                required.0,
                required.1
            );
        }
    }

    #[test]
    fn a_fitted_image_is_window_sized_and_decodes_to_the_same_shape() {
        let directory = tempdir().expect("temp dir");
        let source = directory.path().join("large.jpg");
        write_jpeg(&source, 4000, 3000);

        let fitted = fit_image(&source, WINDOW).expect("large image is worth fitting");

        assert_eq!(fitted.mime_type, "image/bmp");

        let decoded = image::load_from_memory(&fitted.bytes).expect("fitted output decodes");
        assert_eq!((decoded.width(), decoded.height()), (1920, 1440));
    }

    #[test]
    fn an_image_already_inside_the_window_is_served_whole() {
        let directory = tempdir().expect("temp dir");
        let source = directory.path().join("small.jpg");
        write_jpeg(&source, 800, 600);

        assert!(
            fit_image(&source, WINDOW).is_none(),
            "an image smaller than the window has nothing to gain from fitting"
        );
    }

    #[test]
    fn gif_is_never_fitted_so_animation_survives() {
        let directory = tempdir().expect("temp dir");
        let source = directory.path().join("animation.gif");
        RgbImage::new(4000, 3000)
            .save(&source)
            .expect("gif fixture");

        assert!(
            fit_image(&source, WINDOW).is_none(),
            "fitting a GIF would drop every frame but the first"
        );
    }

    #[test]
    fn a_quarter_turn_swaps_the_axes_the_window_is_fitted_against() {
        // 4000x3000 shown under a quarter turn is displayed as 3000x4000, so
        // the height is what binds against the window, not the width.
        let upright = Probe {
            width: 4000,
            height: 3000,
            orientation: Orientation::NoTransforms,
            icc_profile: None,
        };
        let turned = Probe {
            orientation: Orientation::Rotate90,
            ..Probe {
                width: 4000,
                height: 3000,
                orientation: Orientation::NoTransforms,
                icc_profile: None,
            }
        };

        assert_eq!(upright.displayed_size(), (4000, 3000));
        assert_eq!(turned.displayed_size(), (3000, 4000));
        assert_eq!(fit_within(3000, 4000, WINDOW), (1080, 1440));
    }

    #[test]
    fn a_non_srgb_profile_declines_so_the_engine_keeps_colour_management() {
        assert!(is_srgb(b"\x00\x00\x02\x0cdesc\x00\x00sRGB IEC61966-2.1"));
        assert!(!is_srgb(b"\x00\x00\x02\x0cdesc\x00\x00Display P3"));
    }
}
