use std::path::Path;

pub fn is_supported_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpeg" | "jpg" | "webp" | "gif" | "bmp")
    )
}

pub fn is_hidden_dotfile(path: &Path) -> bool {
    path.file_name()
        .and_then(|file_name| file_name.to_str())
        .is_some_and(|file_name| file_name.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn supported_image_formats_are_png_jpeg_jpg_webp_gif_and_bmp() {
        for path in [
            "photo.png",
            "photo.jpeg",
            "photo.jpg",
            "photo.webp",
            "photo.gif",
            "photo.bmp",
            "PHOTO.PNG",
            "PHOTO.JPEG",
            "PHOTO.JPG",
        ] {
            assert!(
                is_supported_image(Path::new(path)),
                "{path} should be supported"
            );
        }
    }

    #[test]
    fn unsupported_image_formats_are_rejected() {
        for path in [
            "vector.svg",
            "scan.tiff",
            "raw.avif",
            "movie.mp4",
            "notes.txt",
            "no-extension",
        ] {
            assert!(
                !is_supported_image(Path::new(path)),
                "{path} should be unsupported"
            );
        }
    }

    #[test]
    fn hidden_dotfiles_are_detected_by_file_name() {
        assert!(is_hidden_dotfile(Path::new("/tmp/.hidden.png")));
        assert!(!is_hidden_dotfile(Path::new("/tmp/visible.png")));
    }
}
