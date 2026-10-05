use std::{fs::File, io::BufReader, path::Path};

use image::{
    codecs::{
        bmp::BmpEncoder, jpeg::JpegEncoder, png::PngDecoder, png::PngEncoder, webp::WebPDecoder,
        webp::WebPEncoder,
    },
    metadata::Orientation,
    DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader,
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
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let cropped = image.crop_imm(rect.x, rect.y, rect.width, rect.height);
    drop(image);

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
    image: &DynamicImage,
    mut encoder: impl ImageEncoder,
    profile: Option<Vec<u8>>,
) -> Result<(), CropError> {
    if let Some(profile) = profile {
        encoder
            .set_icc_profile(profile)
            .map_err(image::ImageError::Unsupported)?;
    }
    image.write_with_encoder(encoder)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, Rgba, RgbaImage};

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
