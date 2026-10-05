use std::fs::File;

use image::{codecs::png::PngEncoder, DynamicImage, ImageEncoder};
use manzar_lib::core::{
    crop::{crop_image, CropRect},
    image_protocol::serve_media_path,
};

fn write_oriented_png(path: &std::path::Path, image: &DynamicImage, orientation: u8) {
    let mut encoder = PngEncoder::new(File::create(path).unwrap());
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
    image.write_with_encoder(encoder).unwrap();
}

#[test]
fn grayscale_fit_matches_equal_rgb_channels_for_all_display_orientations() {
    let directory = tempfile::tempdir().unwrap();
    let gray = DynamicImage::ImageLuma8(image::GrayImage::from_fn(129, 97, |x, y| {
        image::Luma([((x * 73 + y * 19 + x * y) % 256) as u8])
    }));
    let rgb = DynamicImage::ImageRgb8(gray.to_rgb8());
    for orientation in 1..=8 {
        let gray_path = directory.path().join("gray.png");
        let rgb_path = directory.path().join("rgb.png");
        write_oriented_png(&gray_path, &gray, orientation);
        write_oriented_png(&rgb_path, &rgb, orientation);
        for viewport in [(19, 17), (64, 51), (128, 96)] {
            let fitted_gray = serve_media_path(&gray_path, None, Some(viewport)).unwrap();
            let fitted_rgb = serve_media_path(&rgb_path, None, Some(viewport)).unwrap();
            // Equal channels must produce the same pixels through the unchanged
            // RGB path, including edge clipping, padding and convolution rounding.
            assert_eq!(
                fitted_gray.bytes(),
                fitted_rgb.bytes(),
                "EXIF {orientation}, {viewport:?}"
            );
        }
    }
}

#[test]
fn a_display_crop_preserves_exact_8_and_16_bit_pixels_for_every_exif_orientation() {
    let directory = tempfile::tempdir().unwrap();
    // Source pixels, numbered left to right in a 5 x 4 grid:
    // 1..5 / 6..10 / 11..15 / 16..20. These are worked display-space
    // selections at (1,1), size 2 x 3, independent of the crop implementation.
    let expected = [
        [7, 8, 12, 13, 17, 18],
        [9, 8, 14, 13, 19, 18],
        [14, 13, 9, 8, 4, 3],
        [12, 13, 7, 8, 2, 3],
        [7, 12, 8, 13, 9, 14],
        [12, 7, 13, 8, 14, 9],
        [14, 9, 13, 8, 12, 7],
        [9, 14, 8, 13, 7, 12],
    ];
    for sixteen_bit in [false, true] {
        let source = if sixteen_bit {
            DynamicImage::ImageRgba16(image::ImageBuffer::from_fn(5, 4, |x, y| {
                let id = (y * 5 + x + 1) as u16;
                image::Rgba([id * 3000, 1000, 2000, id * 100])
            }))
        } else {
            DynamicImage::ImageRgba8(image::RgbaImage::from_fn(5, 4, |x, y| {
                let id = (y * 5 + x + 1) as u8;
                image::Rgba([id * 10, 100, 200, id * 5])
            }))
        };
        for orientation in 1..=8 {
            let path = directory.path().join("crop.png");
            write_oriented_png(&path, &source, orientation);
            crop_image(
                &path,
                CropRect {
                    x: 1,
                    y: 1,
                    width: 2,
                    height: 3,
                },
            )
            .unwrap();
            let cropped = image::open(path).unwrap();
            assert_eq!(cropped.color(), source.color());
            assert_eq!((cropped.width(), cropped.height()), (2, 3));
            if sixteen_bit {
                let expected: Vec<_> = expected[orientation as usize - 1]
                    .iter()
                    .map(|id| image::Rgba([id * 3000, 1000, 2000, id * 100]))
                    .collect();
                assert_eq!(
                    cropped.into_rgba16().pixels().copied().collect::<Vec<_>>(),
                    expected
                );
            } else {
                let expected: Vec<_> = expected[orientation as usize - 1]
                    .iter()
                    .map(|id| image::Rgba([(id * 10) as u8, 100, 200, (id * 5) as u8]))
                    .collect();
                assert_eq!(
                    cropped.into_rgba8().pixels().copied().collect::<Vec<_>>(),
                    expected
                );
            }
        }
    }
}
