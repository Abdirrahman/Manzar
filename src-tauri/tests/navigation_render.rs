use manzar_lib::core::image_protocol::serve_media_path;

#[test]
fn a_superseded_image_request_never_falls_back_to_reading_the_original() {
    use manzar_lib::core::image_protocol::{serve_media_path_if_current, ImageProtocolError};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("image.png");
    image::RgbaImage::new(256, 192).save(&path).unwrap();
    assert!(matches!(
        serve_media_path_if_current(&path, None, Some((64, 64)), &|| false),
        Err(ImageProtocolError::Superseded)
    ));
}

#[test]
fn revisiting_a_fitted_image_preserves_pixels_and_tracks_source_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("image.bmp");
    image::RgbImage::from_pixel(256, 192, image::Rgb([255, 0, 0]))
        .save(&path)
        .unwrap();
    let first = serve_media_path(&path, None, Some((64, 64))).unwrap();
    assert_eq!(
        first.bytes(),
        serve_media_path(&path, None, Some((64, 64)))
            .unwrap()
            .bytes()
    );
    let larger = serve_media_path(&path, None, Some((128, 128))).unwrap();
    assert_eq!(
        image::load_from_memory(larger.bytes()).unwrap().width(),
        128
    );

    // On Unix, also preserve mtime to exercise inode/change-time invalidation.
    #[cfg(unix)]
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let replacement = directory.path().join("replacement.bmp");
    image::RgbImage::from_pixel(256, 192, image::Rgb([0, 255, 0]))
        .save(&replacement)
        .unwrap();
    #[cfg(unix)]
    std::fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    std::fs::rename(replacement, &path).unwrap();
    let changed = serve_media_path(&path, None, Some((64, 64))).unwrap();
    assert_ne!(changed.bytes(), first.bytes());
    assert_eq!(
        image::load_from_memory(changed.bytes())
            .unwrap()
            .into_rgb8()
            .get_pixel(32, 24)
            .0,
        [0, 255, 0]
    );
    assert_eq!(
        serve_media_path(&path, None, None).unwrap().bytes(),
        std::fs::read(&path).unwrap()
    );
    std::fs::remove_file(&path).unwrap();
    assert!(serve_media_path(&path, None, Some((64, 64))).is_err());
}
