use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
}

/// The single source of truth for what Manzar accepts and how it is served.
/// Adding a format means adding one row here.
const SUPPORTED_MEDIA: &[(&str, MediaKind, &str)] = &[
    ("png", MediaKind::Image, "image/png"),
    ("jpg", MediaKind::Image, "image/jpeg"),
    ("jpeg", MediaKind::Image, "image/jpeg"),
    ("webp", MediaKind::Image, "image/webp"),
    ("gif", MediaKind::Image, "image/gif"),
    ("bmp", MediaKind::Image, "image/bmp"),
    ("mp4", MediaKind::Video, "video/mp4"),
    ("mov", MediaKind::Video, "video/quicktime"),
    ("mkv", MediaKind::Video, "video/x-matroska"),
    ("webm", MediaKind::Video, "video/webm"),
];

fn entry_for(path: &Path) -> Option<&'static (&'static str, MediaKind, &'static str)> {
    let extension = path.extension()?.to_str()?;
    SUPPORTED_MEDIA
        .iter()
        .find(|(candidate, _, _)| candidate.eq_ignore_ascii_case(extension))
}

pub fn media_kind(path: &Path) -> Option<MediaKind> {
    entry_for(path).map(|(_, kind, _)| *kind)
}

pub fn media_mime_type(path: &Path) -> Option<&'static str> {
    entry_for(path).map(|(_, _, mime_type)| *mime_type)
}

pub fn is_supported_media(path: &Path) -> bool {
    entry_for(path).is_some()
}

pub fn is_video(path: &Path) -> bool {
    media_kind(path) == Some(MediaKind::Video)
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
            assert_eq!(
                media_kind(Path::new(path)),
                Some(MediaKind::Image),
                "{path} should be a supported image"
            );
        }
    }

    #[test]
    fn supported_video_formats_are_mp4_mov_mkv_and_webm() {
        for path in [
            "clip.mp4",
            "clip.mov",
            "clip.mkv",
            "clip.webm",
            "CLIP.MP4",
            "CLIP.MKV",
        ] {
            assert_eq!(
                media_kind(Path::new(path)),
                Some(MediaKind::Video),
                "{path} should be a supported video"
            );
            assert!(is_video(Path::new(path)));
        }
    }

    #[test]
    fn every_supported_extension_has_a_mime_type() {
        for (extension, _, mime_type) in SUPPORTED_MEDIA {
            let path = format!("media.{extension}");
            assert_eq!(media_mime_type(Path::new(&path)), Some(*mime_type));
        }
    }

    #[test]
    fn unsupported_formats_are_rejected() {
        for path in [
            "vector.svg",
            "scan.tiff",
            "raw.avif",
            "movie.avi",
            "movie.wmv",
            "notes.txt",
            "no-extension",
        ] {
            assert!(
                !is_supported_media(Path::new(path)),
                "{path} should be unsupported"
            );
            assert_eq!(media_mime_type(Path::new(path)), None);
        }
    }

    #[test]
    fn hidden_dotfiles_are_detected_by_file_name() {
        assert!(is_hidden_dotfile(Path::new("/tmp/.hidden.png")));
        assert!(is_hidden_dotfile(Path::new("/tmp/.hidden.mp4")));
        assert!(!is_hidden_dotfile(Path::new("/tmp/visible.png")));
    }
}
