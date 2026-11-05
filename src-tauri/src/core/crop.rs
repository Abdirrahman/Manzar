use std::{fs::File, io::BufReader, path::Path};

use image::{
    codecs::{
        bmp::BmpEncoder, jpeg::JpegEncoder, png::PngDecoder, png::PngEncoder, webp::WebPDecoder,
        webp::WebPEncoder,
    },
    metadata::Orientation,
    ColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader,
};
use serde::Deserialize;

use super::metadata_preflight::MAX_SAFE_DECODED_RGBA_BYTES;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

// Keep samples in the decoder's native-endian representation. Pixel geometry
// needs only a byte stride, not separate imageops for every channel layout.
pub(super) struct ImagePixels {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub color: ColorType,
}

impl ImagePixels {
    pub fn decode(decoder: Box<dyn ImageDecoder>) -> Result<Self, image::ImageError> {
        let (width, height) = decoder.dimensions();
        let color = decoder.color_type();
        let length = decoder.total_bytes();
        let allocation_error = || {
            image::ImageError::Limits(image::error::LimitError::from_kind(
                image::error::LimitErrorKind::InsufficientMemory,
            ))
        };
        if length > MAX_SAFE_DECODED_RGBA_BYTES {
            return Err(allocation_error());
        }
        let length = usize::try_from(length).map_err(|_| allocation_error())?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| allocation_error())?;
        bytes.resize(length, 0);
        decoder.read_image(&mut bytes)?;
        Ok(Self {
            bytes,
            width,
            height,
            color,
        })
    }

    pub fn crop_orient(self, rect: CropRect, orientation: Orientation) -> Result<Self, CropError> {
        let (width, height) = oriented_dimensions((self.width, self.height), orientation);
        if rect.width == 0
            || rect.height == 0
            || rect.x >= width
            || rect.y >= height
            || rect.width > width - rect.x
            || rect.height > height - rect.y
        {
            return Err(CropError::InvalidBounds);
        }
        if matches!(orientation, Orientation::NoTransforms)
            && rect.x == 0
            && rect.y == 0
            && rect.width == self.width
            && rect.height == self.height
        {
            return Ok(self);
        }

        let stride = isize::from(self.color.bytes_per_pixel());
        let row = self.width as isize * stride;
        let (x, y) = (rect.x, rect.y);
        let (sx, sy, dx, dy) = match orientation {
            Orientation::NoTransforms => (x, y, stride, row),
            Orientation::FlipHorizontal => (self.width - x - 1, y, -stride, row),
            Orientation::Rotate180 => (self.width - x - 1, self.height - y - 1, -stride, -row),
            Orientation::FlipVertical => (x, self.height - y - 1, stride, -row),
            Orientation::Rotate90FlipH => (y, x, row, stride),
            Orientation::Rotate90 => (y, self.height - x - 1, -row, stride),
            Orientation::Rotate270FlipH => (self.width - y - 1, self.height - x - 1, -row, -stride),
            Orientation::Rotate270 => (self.width - y - 1, x, row, -stride),
        };
        let origin = sy as isize * row + sx as isize * stride;
        let pixel_bytes = stride as usize;
        let row_bytes = rect.width as usize * pixel_bytes;
        let mut bytes = vec![0; row_bytes * rect.height as usize];
        // The validated rectangle stays within a decoded buffer capped at
        // 512 MiB, so these signed offsets fit even on 32-bit targets.
        for (y, destination) in bytes.chunks_exact_mut(row_bytes).enumerate() {
            let start = origin + y as isize * dy;
            if dx == stride {
                destination.copy_from_slice(&self.bytes[start as usize..][..row_bytes]);
            } else {
                for (x, pixel) in destination.chunks_exact_mut(pixel_bytes).enumerate() {
                    let offset = (start + x as isize * dx) as usize;
                    pixel.copy_from_slice(&self.bytes[offset..][..pixel_bytes]);
                }
            }
        }
        Ok(Self {
            bytes,
            width: rect.width,
            height: rect.height,
            color: self.color,
        })
    }
}

#[derive(Debug)]
pub enum CropError {
    Unsupported,
    InvalidBounds,
    ImageTooLarge,
    SourceChanged,
    Image(image::ImageError),
    FileSystem(std::io::Error),
}

impl From<image::ImageError> for CropError {
    fn from(error: image::ImageError) -> Self {
        Self::Image(error)
    }
}

impl From<std::io::Error> for CropError {
    fn from(error: std::io::Error) -> Self {
        Self::FileSystem(error)
    }
}

// Shared with preflight: inspect headers once and never offer a destructive
// still-image operation for a file that contains animation.
pub fn still_decoder(path: &Path) -> Result<(ImageFormat, Box<dyn ImageDecoder>), CropError> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let format = reader.format().ok_or(CropError::Unsupported)?;
    let mut decoder: Box<dyn ImageDecoder> = match format {
        ImageFormat::Png => {
            let decoder = PngDecoder::new(reader.into_inner())?;
            if decoder.is_apng()? {
                return Err(CropError::Unsupported);
            }
            Box::new(decoder)
        }
        ImageFormat::WebP => {
            let decoder = WebPDecoder::new(reader.into_inner())?;
            if decoder.has_animation() {
                return Err(CropError::Unsupported);
            }
            Box::new(decoder)
        }
        ImageFormat::Jpeg | ImageFormat::Bmp => Box::new(reader.into_decoder()?),
        _ => return Err(CropError::Unsupported),
    };
    decoder.set_limits(image::Limits::default())?;
    Ok((format, decoder))
}

pub fn oriented_dimensions(dimensions: (u32, u32), orientation: Orientation) -> (u32, u32) {
    if matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    ) {
        (dimensions.1, dimensions.0)
    } else {
        dimensions
    }
}

pub fn crop_image(path: &Path, rect: CropRect) -> Result<(), CropError> {
    let before = std::fs::metadata(path)?;
    if before.permissions().readonly() {
        return Err(CropError::FileSystem(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "source is read-only",
        )));
    }
    let (format, mut decoder) = still_decoder(path)?;
    let orientation = decoder.orientation()?;
    let (width, height) = oriented_dimensions(decoder.dimensions(), orientation);
    if rect.width == 0
        || rect.height == 0
        || rect.x >= width
        || rect.y >= height
        || rect.width > width - rect.x
        || rect.height > height - rect.y
    {
        return Err(CropError::InvalidBounds);
    }
    if decoder.total_bytes() > MAX_SAFE_DECODED_RGBA_BYTES {
        return Err(CropError::ImageTooLarge);
    }
    let profile = decoder.icc_profile()?;
    let cropped = ImagePixels::decode(decoder)?.crop_orient(rect, orientation)?;

    // Encode beside the source, flush, then atomically replace it. An encoder,
    // permissions or disk-space failure leaves the original untouched; tempfile
    // also removes the unfinished file on every error path.
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().ok_or(CropError::Unsupported)?)?;
    match format {
        ImageFormat::Jpeg => encode_crop(
            &cropped,
            JpegEncoder::new_with_quality(temporary.as_file_mut(), 95),
            profile,
        )?,
        ImageFormat::Png => {
            encode_crop(&cropped, PngEncoder::new(temporary.as_file_mut()), profile)?
        }
        ImageFormat::WebP => encode_crop(
            &cropped,
            WebPEncoder::new_lossless(temporary.as_file_mut()),
            profile,
        )?,
        ImageFormat::Bmp => {
            encode_crop(&cropped, BmpEncoder::new(temporary.as_file_mut()), profile)?
        }
        _ => return Err(CropError::Unsupported),
    }
    // Verify the encoded file before replacing the only copy.
    if ImageReader::new(BufReader::new(File::open(temporary.path())?))
        .with_guessed_format()?
        .into_dimensions()?
        != (rect.width, rect.height)
    {
        return Err(CropError::InvalidBounds);
    }
    temporary.as_file().set_permissions(before.permissions())?;
    temporary.as_file().sync_all()?;
    let after = std::fs::metadata(path)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(CropError::SourceChanged);
    }
    temporary
        .persist(path)
        .map_err(|error| CropError::FileSystem(error.error))?;
    Ok(())
}

fn encode_crop(
    image: &ImagePixels,
    mut encoder: impl ImageEncoder,
    profile: Option<Vec<u8>>,
) -> Result<(), CropError> {
    if let Some(profile) = profile {
        encoder
            .set_icc_profile(profile)
            .map_err(image::ImageError::Unsupported)?;
    }
    // The source decoder already returns a layout accepted by its encoder:
    // JPEG L8/RGB8, BMP/WebP 8-bit, and PNG 8/16-bit. No conversion is needed.
    encoder.write_image(&image.bytes, image.width, image.height, image.color.into())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

    const COLORS: [ColorType; 8] = [
        ColorType::L8,
        ColorType::La8,
        ColorType::Rgb8,
        ColorType::Rgba8,
        ColorType::L16,
        ColorType::La16,
        ColorType::Rgb16,
        ColorType::Rgba16,
    ];

    fn fixture(color: ColorType) -> DynamicImage {
        let pixels = (0..9 * 7 * usize::from(color.bytes_per_pixel()))
            .map(|index| index.wrapping_mul(73).wrapping_add(index / 7) as u8)
            .collect::<Vec<_>>();
        let mut encoded = Vec::new();
        PngEncoder::new(&mut encoded)
            .write_image(&pixels, 9, 7, color.into())
            .unwrap();
        image::load_from_memory(&encoded).unwrap()
    }

    fn reference_encode(
        image: &DynamicImage,
        format: ImageFormat,
        profile: Option<Vec<u8>>,
        orientation: Option<u8>,
    ) -> Vec<u8> {
        fn write(
            image: &DynamicImage,
            mut encoder: impl ImageEncoder,
            profile: Option<Vec<u8>>,
            orientation: Option<u8>,
        ) {
            if let Some(profile) = profile {
                encoder.set_icc_profile(profile).unwrap();
            }
            if let Some(orientation) = orientation {
                encoder
                    .set_exif_metadata(vec![
                        73,
                        73,
                        42,
                        0,
                        8,
                        0,
                        0,
                        0,
                        1,
                        0,
                        18,
                        1,
                        3,
                        0,
                        1,
                        0,
                        0,
                        0,
                        orientation,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                    ])
                    .unwrap();
            }
            image.write_with_encoder(encoder).unwrap();
        }
        let mut bytes = Vec::new();
        match format {
            ImageFormat::Png => write(image, PngEncoder::new(&mut bytes), profile, orientation),
            ImageFormat::Jpeg => write(
                image,
                JpegEncoder::new_with_quality(&mut bytes, 95),
                profile,
                orientation,
            ),
            ImageFormat::WebP => write(
                image,
                WebPEncoder::new_lossless(&mut bytes),
                profile,
                orientation,
            ),
            ImageFormat::Bmp => write(image, BmpEncoder::new(&mut bytes), profile, orientation),
            _ => unreachable!(),
        }
        bytes
    }

    #[test]
    fn byte_geometry_matches_imageops_for_every_layout_orientation_and_crop_edge() {
        let orientations = [
            Orientation::NoTransforms,
            Orientation::FlipHorizontal,
            Orientation::Rotate180,
            Orientation::FlipVertical,
            Orientation::Rotate90FlipH,
            Orientation::Rotate90,
            Orientation::Rotate270FlipH,
            Orientation::Rotate270,
        ];
        for color in COLORS {
            let source = fixture(color);
            for orientation in orientations {
                let mut oriented = source.clone();
                oriented.apply_orientation(orientation);
                for rect in [
                    CropRect {
                        x: 0,
                        y: 0,
                        width: oriented.width(),
                        height: oriented.height(),
                    },
                    CropRect {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                    CropRect {
                        x: oriented.width() - 1,
                        y: oriented.height() - 1,
                        width: 1,
                        height: 1,
                    },
                    CropRect {
                        x: 1,
                        y: 2,
                        width: 3,
                        height: 2,
                    },
                ] {
                    let pixels = ImagePixels {
                        bytes: source.as_bytes().to_vec(),
                        width: source.width(),
                        height: source.height(),
                        color,
                    };
                    let actual = pixels.crop_orient(rect, orientation).unwrap();
                    let expected = oriented.crop_imm(rect.x, rect.y, rect.width, rect.height);
                    assert_eq!(
                        actual.bytes,
                        expected.as_bytes(),
                        "{color:?}, {orientation:?}, {rect:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn encoded_crops_match_dynamic_image_bytes_for_all_formats_layouts_and_orientations() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source");
        for format in [
            ImageFormat::Png,
            ImageFormat::Jpeg,
            ImageFormat::WebP,
            ImageFormat::Bmp,
        ] {
            for color in COLORS {
                if !match format {
                    ImageFormat::Png => true,
                    ImageFormat::Jpeg => matches!(color, ColorType::L8 | ColorType::Rgb8),
                    ImageFormat::WebP => matches!(color, ColorType::Rgb8 | ColorType::Rgba8),
                    ImageFormat::Bmp => {
                        matches!(color, ColorType::L8 | ColorType::Rgb8 | ColorType::Rgba8)
                    }
                    _ => unreachable!(),
                } {
                    continue;
                }
                let source = fixture(color);
                for orientation in 1..=if format == ImageFormat::Bmp { 1 } else { 8 } {
                    let profile =
                        (format != ImageFormat::Bmp).then(|| b"test colour profile".to_vec());
                    let exif = (format != ImageFormat::Bmp).then_some(orientation);
                    std::fs::write(&path, reference_encode(&source, format, profile, exif))
                        .unwrap();
                    let (_, mut decoder) = still_decoder(&path).unwrap();
                    let orientation = decoder.orientation().unwrap();
                    let profile = decoder.icc_profile().unwrap();
                    let mut reference = DynamicImage::from_decoder(decoder).unwrap();
                    reference.apply_orientation(orientation);
                    let rect = CropRect {
                        x: 1,
                        y: 2,
                        width: 3,
                        height: 2,
                    };
                    let expected = reference_encode(
                        &reference.crop_imm(rect.x, rect.y, rect.width, rect.height),
                        format,
                        profile,
                        None,
                    );
                    crop_image(&path, rect).unwrap();
                    assert_eq!(
                        std::fs::read(&path).unwrap(),
                        expected,
                        "{format:?}, {color:?}, {orientation:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn crop_replaces_source_preserving_pixels_alpha_format_and_cleans_up() {
        let directory = tempfile::tempdir().unwrap();
        for extension in ["png", "webp", "bmp", "jpg"] {
            let path = directory.path().join(format!("source.{extension}"));
            let source =
                RgbaImage::from_fn(8, 6, |x, y| Rgba([x as u8 * 20, y as u8 * 20, 90, 128]));
            if extension == "jpg" {
                DynamicImage::ImageRgba8(source.clone())
                    .to_rgb8()
                    .save(&path)
                    .unwrap();
            } else {
                source.save(&path).unwrap();
            }
            crop_image(
                &path,
                CropRect {
                    x: 2,
                    y: 1,
                    width: 4,
                    height: 3,
                },
            )
            .unwrap();
            let result = image::open(&path).unwrap();
            assert_eq!(result.dimensions(), (4, 3));
            if extension != "jpg" {
                assert_eq!(result.get_pixel(0, 0), *source.get_pixel(2, 1));
            }
            std::fs::remove_file(path).unwrap();
        }
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn crop_uses_display_orientation_and_preserves_colour_profile() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rotated.png");
        let source = RgbaImage::from_fn(8, 6, |x, y| Rgba([x as u8 * 20, y as u8 * 20, 90, 128]));
        let mut encoder = PngEncoder::new(File::create(&path).unwrap());
        // Little-endian TIFF with a single EXIF orientation tag: rotate 90°.
        encoder
            .set_exif_metadata(vec![
                73, 73, 42, 0, 8, 0, 0, 0, 1, 0, 18, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
            ])
            .unwrap();
        let profile = b"test colour profile".to_vec();
        encoder.set_icc_profile(profile.clone()).unwrap();
        source.write_with_encoder(encoder).unwrap();
        let preflight = super::super::metadata_preflight::preflight_image(&path).unwrap();
        assert_eq!(
            preflight.dimensions(),
            Some(super::super::metadata_preflight::ImageDimensions {
                width: 6,
                height: 8
            })
        );
        crop_image(
            &path,
            CropRect {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
            },
        )
        .unwrap();
        let result = image::open(&path).unwrap();
        let expected = image::imageops::rotate90(&source);
        assert_eq!(result.get_pixel(0, 0), *expected.get_pixel(1, 2));
        let (_, mut decoder) = still_decoder(&path).unwrap();
        assert_eq!(decoder.orientation().unwrap(), Orientation::NoTransforms);
        assert_eq!(decoder.icc_profile().unwrap(), Some(profile));
    }

    #[test]
    fn read_only_or_corrupt_source_survives_a_failed_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.png");
        RgbaImage::new(8, 6).save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        let permissions = std::fs::metadata(&path).unwrap().permissions();
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&path, readonly).unwrap();
        let rect = CropRect {
            x: 0,
            y: 0,
            width: 3,
            height: 3,
        };
        assert!(crop_image(&path, rect).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::set_permissions(&path, permissions).unwrap();
        std::fs::write(&path, b"corrupt image").unwrap();
        assert!(crop_image(&path, rect).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"corrupt image");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn invalid_crops_and_animated_formats_leave_source_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.png");
        RgbaImage::new(8, 6).save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        for rect in [
            CropRect {
                x: 0,
                y: 0,
                width: 0,
                height: 2,
            },
            CropRect {
                x: 7,
                y: 0,
                width: 2,
                height: 2,
            },
            CropRect {
                x: u32::MAX,
                y: 0,
                width: 2,
                height: 2,
            },
        ] {
            assert!(matches!(
                crop_image(&path, rect),
                Err(CropError::InvalidBounds)
            ));
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
        let gif = directory.path().join("animation.gif");
        RgbaImage::new(8, 6).save(&gif).unwrap();
        let original = std::fs::read(&gif).unwrap();
        assert!(matches!(
            crop_image(
                &gif,
                CropRect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2
                }
            ),
            Err(CropError::Unsupported)
        ));
        assert_eq!(std::fs::read(&gif).unwrap(), original);
    }
}
