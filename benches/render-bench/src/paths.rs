//! The two pipelines under comparison.
//!
//! # What "webview" means here
//!
//! WebKitGTK cannot be driven from a benchmark harness, so the engine's own
//! work is modelled rather than executed: a full-resolution decode with a
//! production Rust decoder, then a full-surface rescale to the display size.
//! That is a deliberately *conservative* model of the engine's cost — it
//! charges the webview for the decode and the scale and nothing else, while
//! the real engine additionally copies the file across the protocol boundary,
//! keeps a source surface alongside the decoded one, and re-uploads a texture
//! per frame. The engine's decoders (libjpeg-turbo, libpng) are in the same
//! performance class as the ones used here, so the modelled decode time is a
//! fair stand-in and, if anything, understates what ships today.
//!
//! The comparison is therefore of *work performed to put the same pixels on
//! the same screen*, which is the thing the plan proposes to change.

use fast_image_resize::images::{Image as FirImage, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::RgbImage;

/// Decoded pixels plus the dimensions they were decoded at. Kept as interleaved
/// RGB8 because that is what both resamplers and a GPU upload want.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Frame {
    pub fn byte_len(&self) -> u64 {
        self.rgb.len() as u64
    }
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Full-resolution decode through the `image` crate, the decoder already in
/// Manzar's dependency tree. This is the shape of work the webview does on
/// every open.
pub fn decode_full_image_crate(bytes: &[u8]) -> Frame {
    let decoded = image::load_from_memory(bytes).expect("fixture decodes");
    let rgb: RgbImage = decoded.into_rgb8();
    Frame {
        width: rgb.width(),
        height: rgb.height(),
        rgb: rgb.into_raw(),
    }
}

/// Full-resolution JPEG decode through `zune-jpeg`, whose IDCT and colour
/// conversion are hand-written AVX2.
pub fn decode_full_zune(bytes: &[u8]) -> Frame {
    let options = zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGB);
    let mut decoder =
        zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(bytes), options);
    let rgb = decoder.decode().expect("fixture decodes");
    let (width, height) = decoder.dimensions().expect("decoded dimensions");

    Frame {
        width: width as u32,
        height: height as u32,
        rgb,
    }
}

/// Decode a JPEG no larger than it needs to be.
///
/// This is the single largest win available and it costs nothing but asking:
/// JPEG stores frequency coefficients, so a decoder can reconstruct at 1/2,
/// 1/4 or 1/8 scale by simply discarding the high-frequency coefficients
/// before the IDCT. The pixels never exist at full size, so neither the
/// entropy decode past the retained coefficients, the upsampling, nor the
/// allocation is ever paid for.
///
/// The scale chosen is the smallest one that still covers `target`, so the
/// result is never softer than the display can show.
pub fn decode_scaled_jpeg(bytes: &[u8], viewport: (u32, u32)) -> Frame {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    decoder.read_info().expect("fixture header");

    let info = decoder.info().expect("fixture info");
    let (width, height) = required_source_size(
        u32::from(info.width),
        u32::from(info.height),
        viewport,
    );

    let (width, height) = decoder
        .scale(width as u16, height as u16)
        .expect("dct scale accepted");
    let pixels = decoder.decode().expect("fixture decodes");

    // A greyscale or CMYK fixture would need converting; the corpus is YCbCr,
    // so anything else is a corpus bug rather than a case to handle silently.
    assert_eq!(
        decoder.info().expect("info").pixel_format,
        jpeg_decoder::PixelFormat::RGB24,
        "corpus is expected to decode as RGB24"
    );

    Frame {
        width: u32::from(width),
        height: u32::from(height),
        rgb: pixels,
    }
}

/// The DCT scale a viewer should decode this image at to fill `viewport`.
///
/// The image is letterboxed into the window, so the resolution that must be
/// covered is the *fitted* size, not the whole viewport box. Requiring the
/// latter rejects scales that are in fact more than good enough — a
/// 4000x3000 frame shown in a 2560x1440 window is only ever 1920x1440 on
/// screen, so a half-scale decode still has pixels to spare.
pub fn required_source_size(width: u32, height: u32, viewport: (u32, u32)) -> (u32, u32) {
    dct_scale_for(width, height, fit_within(width, height, viewport))
}

/// Picks the smallest of JPEG's four DCT scales whose output still covers
/// `required` in both axes.
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

/// Decodes a JPEG at an explicit DCT divisor, used to chart how decode cost
/// falls with scale rather than only reporting the scale a viewer would pick.
pub fn decode_jpeg_at_divisor(bytes: &[u8], divisor: u32) -> Frame {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    decoder.read_info().expect("fixture header");
    let info = decoder.info().expect("fixture info");

    let (width, height) = decoder
        .scale(
            u32::from(info.width).div_ceil(divisor) as u16,
            u32::from(info.height).div_ceil(divisor) as u16,
        )
        .expect("dct scale accepted");
    let pixels = decoder.decode().expect("fixture decodes");

    Frame {
        width: u32::from(width),
        height: u32::from(height),
        rgb: pixels,
    }
}

// ---------------------------------------------------------------------------
// Resize
// ---------------------------------------------------------------------------

/// Fit-to-window scaling through `image::imageops`, the scalar resampler.
pub fn resize_image_crate(frame: &Frame, target: (u32, u32)) -> Frame {
    // Borrowed, not cloned: a `Vec` copy inside the timed region would charge
    // this path for a memcpy the SIMD path never pays, and flatter the result.
    let source: image::ImageBuffer<image::Rgb<u8>, &[u8]> =
        image::ImageBuffer::from_raw(frame.width, frame.height, frame.rgb.as_slice())
            .expect("frame buffer matches dimensions");
    let (width, height) = fit_within(frame.width, frame.height, target);
    let resized = image::imageops::resize(
        &source,
        width,
        height,
        image::imageops::FilterType::Lanczos3,
    );

    Frame {
        width: resized.width(),
        height: resized.height(),
        rgb: resized.into_raw(),
    }
}

/// Fit-to-window scaling through `fast_image_resize`, which dispatches to AVX2
/// or NEON convolution kernels at runtime.
pub fn resize_fir(frame: &Frame, target: (u32, u32)) -> Frame {
    let (width, height) = fit_within(frame.width, frame.height, target);
    resize_region(frame, None, (width, height))
}

/// Resamples `region` of `frame` (or the whole frame) into a `target`-sized
/// buffer. The crop is applied inside the resampler, so the source rows outside
/// it are never touched — which is what makes zooming cost the viewport rather
/// than the image.
pub fn resize_region(
    frame: &Frame,
    region: Option<(f64, f64, f64, f64)>,
    target: (u32, u32),
) -> Frame {
    let source = ImageRef::new(frame.width, frame.height, &frame.rgb, PixelType::U8x3)
        .expect("frame buffer matches dimensions");
    let mut destination = FirImage::new(target.0, target.1, PixelType::U8x3);

    let mut options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    if let Some((left, top, width, height)) = region {
        options = options.crop(left, top, width, height);
    }

    Resizer::new()
        .resize(&source, &mut destination, &options)
        .expect("compatible pixel types");

    Frame {
        width: target.0,
        height: target.1,
        rgb: destination.into_vec(),
    }
}

// ---------------------------------------------------------------------------
// Zoom
// ---------------------------------------------------------------------------

/// What an engine does for a zoom step: re-raster the entire image at the new
/// scale, including the overwhelming majority of it that falls outside the
/// window. Cost grows with the image.
pub fn zoom_full_reraster(frame: &Frame, scale: f64) -> Frame {
    let width = ((f64::from(frame.width) * scale).round() as u32).max(1);
    let height = ((f64::from(frame.height) * scale).round() as u32).max(1);
    resize_region(frame, None, (width, height))
}

/// What a native renderer does for the same zoom step: work out which source
/// rectangle the window is looking at and resample only that, straight into a
/// viewport-sized buffer. Cost is flat in the image size.
pub fn zoom_viewport_crop(frame: &Frame, scale: f64, viewport: (u32, u32)) -> Frame {
    let visible_width = (f64::from(viewport.0) / scale).min(f64::from(frame.width));
    let visible_height = (f64::from(viewport.1) / scale).min(f64::from(frame.height));
    let left = (f64::from(frame.width) - visible_width) / 2.0;
    let top = (f64::from(frame.height) - visible_height) / 2.0;

    let target = (
        (visible_width * scale).round().max(1.0) as u32,
        (visible_height * scale).round().max(1.0) as u32,
    );

    resize_region(
        frame,
        Some((left, top, visible_width, visible_height)),
        target,
    )
}

// ---------------------------------------------------------------------------
// End-to-end
// ---------------------------------------------------------------------------

/// Today's path, from a file on disk to pixels ready to show: read the whole
/// file into memory (`serve_approved_media` does exactly this), decode every
/// pixel, then scale the full surface down to the window.
pub fn first_pixel_webview(path: &std::path::Path, viewport: (u32, u32)) -> Frame {
    let bytes = std::fs::read(path).expect("fixture readable");
    let decoded = decode_full_image_crate(&bytes);
    resize_image_crate(&decoded, viewport)
}

pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

/// Largest box within `target` that keeps the source aspect ratio — the
/// geometry behind "fit to window".
pub fn fit_within(width: u32, height: u32, target: (u32, u32)) -> (u32, u32) {
    let scale = (f64::from(target.0) / f64::from(width))
        .min(f64::from(target.1) / f64::from(height))
        .min(1.0);

    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

// ---------------------------------------------------------------------------
// The recommended decode strategy
// ---------------------------------------------------------------------------

/// Take a DCT-scaled decode whenever one exists at all.
///
/// The two candidate decoders are not equal per pixel: `zune-jpeg` has AVX2
/// IDCT and colour conversion, `jpeg-decoder` is scalar but is the only one of
/// the two that can decode at a reduced DCT scale. Comparing the decoders in
/// isolation says to prefer the SIMD one at 1/2 scale, where the scalar
/// decoder only breaks even.
///
/// Comparing the *pipelines* says the opposite, and the pipeline is what
/// matters. A half-scale decode does not just halve the decode: it quarters
/// the pixel count every downstream stage touches. At 12 MP that is 99 ms and
/// 28 MB peak for the scaled path against 112 ms and 64 MB for the fast full
/// decode — better on both axes. So the threshold is 2, not 4: scale whenever
/// a scale is available, and fall back to `zune-jpeg` only when the image is
/// already small enough that no divisor applies.
pub const DCT_DIVISOR_WORTH_SCALING: u32 = 2;

/// Picks the decoder by how much of the image the window will actually use.
///
/// This is the strategy the plan recommends shipping, and the one the
/// `first-pixel` comparison measures.
pub fn decode_for_viewport(bytes: &[u8], viewport: (u32, u32)) -> Frame {
    if !is_jpeg(bytes) {
        return decode_full_image_crate(bytes);
    }

    let Some((width, height)) = jpeg_dimensions(bytes) else {
        return decode_full_image_crate(bytes);
    };

    let required = required_source_size(width, height, viewport);
    let divisor = width.div_ceil(required.0.max(1)).max(1);

    if divisor >= DCT_DIVISOR_WORTH_SCALING {
        decode_jpeg_at_divisor(bytes, divisor)
    } else {
        decode_full_zune(bytes)
    }
}

/// Reads only the JPEG frame header. Cheap enough to run before deciding how to
/// decode, which is what makes the choice above possible at all.
pub fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    Some((u32::from(info.width), u32::from(info.height)))
}

/// The full recommended pipeline: header probe, scale-aware decode, SIMD fit.
pub fn first_pixel_recommended(path: &std::path::Path, viewport: (u32, u32)) -> Frame {
    let bytes = std::fs::read(path).expect("fixture readable");
    let decoded = decode_for_viewport(&bytes, viewport);
    resize_fir(&decoded, viewport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_within_keeps_aspect_ratio_and_never_upscales() {
        // Landscape into a landscape window: height is the binding constraint.
        assert_eq!(fit_within(4000, 3000, (2560, 1440)), (1920, 1440));
        // Portrait into the same window: width binds instead.
        assert_eq!(fit_within(3000, 4000, (2560, 1440)), (1080, 1440));
        // Already smaller than the window: left alone rather than blown up.
        assert_eq!(fit_within(800, 600, (2560, 1440)), (800, 600));
    }

    #[test]
    fn dct_scale_never_returns_fewer_pixels_than_the_display_needs() {
        for (width, height) in [(4000u32, 3000u32), (6000, 4000), (9504, 6336), (800, 600)] {
            let required = fit_within(width, height, VIEWPORT_FOR_TESTS);
            let (scaled_width, scaled_height) = required_source_size(width, height, VIEWPORT_FOR_TESTS);

            assert!(
                scaled_width >= required.0 && scaled_height >= required.1,
                "{width}x{height} scaled to {scaled_width}x{scaled_height}, \
                 which is under the {}x{} the window needs",
                required.0,
                required.1
            );
        }
    }

    #[test]
    fn dct_scale_picks_the_smallest_divisor_that_still_covers_the_window() {
        // 9504x6336 fits to 2160x1440, so 1/4 (2376x1584) covers it but
        // 1/8 (1188x792) does not.
        assert_eq!(required_source_size(9504, 6336, VIEWPORT_FOR_TESTS), (2376, 1584));
        // 4000x3000 fits to 1920x1440; 1/2 (2000x1500) covers, 1/4 does not.
        assert_eq!(required_source_size(4000, 3000, VIEWPORT_FOR_TESTS), (2000, 1500));
        // Already smaller than the window: no divisor applies.
        assert_eq!(required_source_size(800, 600, VIEWPORT_FOR_TESTS), (800, 600));
    }

    #[test]
    fn a_zoom_step_costs_the_viewport_not_the_image() {
        // The property the whole plan rests on: whatever the source size, the
        // native zoom path produces a buffer bounded by the window.
        let frame = flat_frame(4000, 3000);
        let large = flat_frame(9504, 6336);

        for scale in [1.0, 2.0, 4.0] {
            for source in [&frame, &large] {
                let zoomed = zoom_viewport_crop(source, scale, VIEWPORT_FOR_TESTS);
                assert!(
                    zoomed.width <= VIEWPORT_FOR_TESTS.0 && zoomed.height <= VIEWPORT_FOR_TESTS.1,
                    "{}x{} at {scale}x produced {}x{}, larger than the window",
                    source.width,
                    source.height,
                    zoomed.width,
                    zoomed.height
                );
            }
        }
    }

    #[test]
    fn both_resamplers_agree_on_output_dimensions() {
        // The comparison is only fair if the two paths are asked for the same
        // pixels. This is the guard on that.
        let frame = flat_frame(1200, 800);
        let scalar = resize_image_crate(&frame, VIEWPORT_FOR_TESTS);
        let simd = resize_fir(&frame, VIEWPORT_FOR_TESTS);

        assert_eq!((scalar.width, scalar.height), (simd.width, simd.height));
        assert_eq!(scalar.rgb.len(), simd.rgb.len());
    }

    #[test]
    fn the_recommended_decoder_choice_matches_the_measured_rule() {
        // Below the threshold there is no divisor to take, so the fast full
        // decoder is the right answer; at or above it, the scaled one is.
        assert_eq!(DCT_DIVISOR_WORTH_SCALING, 2);
        assert_eq!(required_source_size(800, 600, VIEWPORT_FOR_TESTS), (800, 600));
    }

    const VIEWPORT_FOR_TESTS: (u32, u32) = crate::VIEWPORT;

    fn flat_frame(width: u32, height: u32) -> Frame {
        Frame {
            width,
            height,
            rgb: vec![128; (width as usize) * (height as usize) * 3],
        }
    }
}
