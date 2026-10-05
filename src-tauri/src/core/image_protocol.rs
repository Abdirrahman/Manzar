use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use super::{
    image_registry::{ApprovedImageRegistry, ImageId},
    metadata_preflight::MAX_SAFE_FILE_SIZE_BYTES,
    render,
    supported_image::{media_kind, media_mime_type, MediaKind},
};

pub const IMAGE_PROTOCOL_SCHEME: &str = "manzar-image";

/// Largest slice of a video served for one request. Playback is a sequence of
/// range requests, so this caps how much video is ever resident in memory.
pub const VIDEO_CHUNK_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolImageResponse {
    mime_type: &'static str,
    bytes: Vec<u8>,
    /// `Some("bytes start-end/total")` when only a slice was served.
    content_range: Option<String>,
    total_bytes: u64,
}

#[derive(Debug)]
pub enum ImageProtocolError {
    Superseded,
    UnknownImageId,
    UnsupportedImage,
    OversizedImage,
    RangeNotSatisfiable { total_bytes: u64 },
    FileSystem(std::io::Error),
}

impl From<std::io::Error> for ImageProtocolError {
    fn from(error: std::io::Error) -> Self {
        Self::FileSystem(error)
    }
}

impl ProtocolImageResponse {
    pub fn mime_type(&self) -> &'static str {
        self.mime_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn content_range(&self) -> Option<&str> {
        self.content_range.as_deref()
    }

    pub fn is_partial(&self) -> bool {
        self.content_range.is_some()
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
}

pub fn image_url(id: &ImageId) -> String {
    format!("{IMAGE_PROTOCOL_SCHEME}://localhost/{}", id.as_str())
}

pub fn image_id_from_protocol_path(path: &str) -> Option<ImageId> {
    let opaque_id = path.strip_prefix('/').unwrap_or(path);

    if opaque_id.is_empty()
        || opaque_id == "."
        || opaque_id == ".."
        || opaque_id.contains('/')
        || opaque_id.contains('\\')
    {
        return None;
    }

    Some(ImageId::from_opaque(opaque_id))
}

pub fn serve_approved_image(
    registry: &ApprovedImageRegistry,
    id: &ImageId,
) -> Result<ProtocolImageResponse, ImageProtocolError> {
    serve_approved_media(registry, id, None, None)
}

/// `viewport` is the window the image will be shown in, in device pixels. When
/// it is given and the image is larger than it, the response is a fitted
/// surface rather than the file; see [`render`].
pub fn serve_approved_media(
    registry: &ApprovedImageRegistry,
    id: &ImageId,
    range_header: Option<&str>,
    viewport: Option<(u32, u32)>,
) -> Result<ProtocolImageResponse, ImageProtocolError> {
    let path = registry
        .path_for(id)
        .ok_or(ImageProtocolError::UnknownImageId)?;
    serve_media_path(path, range_header, viewport)
}

pub fn serve_media_path(
    path: &Path,
    range_header: Option<&str>,
    viewport: Option<(u32, u32)>,
) -> Result<ProtocolImageResponse, ImageProtocolError> {
    serve_media_path_if_current(path, range_header, viewport, &|| true)
}

/// A navigation request can be abandoned without falling back to a full-file
/// read. The predicate is checked between expensive rendering stages.
pub fn serve_media_path_if_current(
    path: &Path,
    range_header: Option<&str>,
    viewport: Option<(u32, u32)>,
    is_current: &impl Fn() -> bool,
) -> Result<ProtocolImageResponse, ImageProtocolError> {
    if !is_current() {
        return Err(ImageProtocolError::Superseded);
    }
    let kind = media_kind(path).ok_or(ImageProtocolError::UnsupportedImage)?;
    let mime_type = media_mime_type(path).ok_or(ImageProtocolError::UnsupportedImage)?;
    let total_bytes = std::fs::metadata(path)?.len();

    // An image is served whole — either fitted to the window or as the original
    // file — and capped by size. Only video is worth streaming.
    if kind == MediaKind::Image {
        if total_bytes > MAX_SAFE_FILE_SIZE_BYTES {
            return Err(ImageProtocolError::OversizedImage);
        }

        // Fitting declines whenever it cannot improve on the file or would
        // change what the user sees, so falling through is always correct.
        let fitted =
            viewport.and_then(|viewport| render::fit_image_if_current(path, viewport, is_current));
        if !is_current() {
            return Err(ImageProtocolError::Superseded);
        }
        if let Some(fitted) = fitted {
            return Ok(ProtocolImageResponse {
                mime_type: fitted.mime_type,
                total_bytes: fitted.bytes.len() as u64,
                bytes: fitted.bytes,
                content_range: None,
            });
        }

        return Ok(ProtocolImageResponse {
            mime_type,
            bytes: std::fs::read(path)?,
            content_range: None,
            total_bytes,
        });
    }

    if total_bytes == 0 {
        return Ok(ProtocolImageResponse {
            mime_type,
            bytes: Vec::new(),
            content_range: None,
            total_bytes,
        });
    }

    let (start, requested_end) = range_header
        .and_then(|header| parse_byte_range(header, total_bytes))
        .unwrap_or((0, None));

    if start >= total_bytes {
        return Err(ImageProtocolError::RangeNotSatisfiable { total_bytes });
    }

    let last_byte = total_bytes - 1;
    let end = requested_end
        .unwrap_or(last_byte)
        .min(last_byte)
        .min(start + VIDEO_CHUNK_BYTES - 1);
    let length = end - start + 1;

    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(length).read_to_end(&mut bytes)?;

    // A short read means the file shrank between the metadata call and the read;
    // report what was actually produced so Content-Range never over-promises.
    if bytes.is_empty() {
        return Err(ImageProtocolError::RangeNotSatisfiable { total_bytes });
    }
    let end = start + bytes.len() as u64 - 1;

    Ok(ProtocolImageResponse {
        mime_type,
        bytes,
        content_range: Some(format!("bytes {start}-{end}/{total_bytes}")),
        total_bytes,
    })
}

/// Parses a single `Range: bytes=…` spec. Multi-range requests are not
/// honoured; returning `None` makes the caller serve from the start instead.
fn parse_byte_range(header: &str, total_bytes: u64) -> Option<(u64, Option<u64>)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let (start, end) = (start.trim(), end.trim());

    if start.is_empty() {
        // Suffix range: `bytes=-500` means the last 500 bytes.
        let suffix: u64 = end.parse().ok()?;
        return Some((total_bytes.saturating_sub(suffix), None));
    }

    let start = start.parse().ok()?;
    let end = if end.is_empty() {
        None
    } else {
        Some(end.parse().ok()?)
    };

    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn approved_image_id_serves_bytes_with_supported_mime_type() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("private-name.png");
        std::fs::write(&image, b"png bytes").expect("image file");

        let mut registry = ApprovedImageRegistry::default();
        let approved = registry.approve_path(&image).expect("approved image");

        let response =
            serve_approved_image(&registry, approved.id()).expect("protocol image response");

        assert_eq!(response.mime_type(), "image/png");
        assert_eq!(response.bytes(), b"png bytes");
        assert!(!response.is_partial());
    }

    #[test]
    fn unknown_image_id_is_rejected() {
        let registry = ApprovedImageRegistry::default();
        let unknown = ImageId::from_opaque("image-404");

        assert!(matches!(
            serve_approved_image(&registry, &unknown),
            Err(ImageProtocolError::UnknownImageId)
        ));
    }

    #[test]
    fn oversized_approved_image_is_rejected_before_serving_bytes() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("large.png");
        std::fs::File::create(&image)
            .expect("test image")
            .set_len(MAX_SAFE_FILE_SIZE_BYTES + 1)
            .expect("large sparse file");

        let mut registry = ApprovedImageRegistry::default();
        let approved = registry.approve_path(&image).expect("approved image");

        assert!(matches!(
            serve_approved_image(&registry, approved.id()),
            Err(ImageProtocolError::OversizedImage)
        ));
    }

    #[test]
    fn image_url_contains_only_protocol_origin_and_opaque_id() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("private-name.png");
        std::fs::write(&image, b"png bytes").expect("image file");

        let mut registry = ApprovedImageRegistry::default();
        let approved = registry.approve_path(&image).expect("approved image");

        let url = image_url(approved.id());

        assert_eq!(
            url,
            format!("manzar-image://localhost/{}", approved.id().as_str())
        );
        assert!(!url.contains("private-name"));
        assert!(!url.contains(directory.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn protocol_path_parses_only_a_single_opaque_id_segment() {
        assert_eq!(
            image_id_from_protocol_path("/image-42").map(|id| id.as_str().to_string()),
            Some("image-42".to_string())
        );
        assert_eq!(image_id_from_protocol_path("/"), None);
        assert_eq!(image_id_from_protocol_path("/../secret"), None);
        assert_eq!(image_id_from_protocol_path("/image-1/extra"), None);
    }

    #[test]
    fn supported_media_mime_types_are_served() {
        for (name, expected_mime_type) in [
            ("image.png", "image/png"),
            ("image.jpg", "image/jpeg"),
            ("image.JPG", "image/jpeg"),
            ("image.jpeg", "image/jpeg"),
            ("image.webp", "image/webp"),
            ("image.gif", "image/gif"),
            ("image.bmp", "image/bmp"),
            ("clip.mp4", "video/mp4"),
            ("clip.mov", "video/quicktime"),
            ("clip.mkv", "video/x-matroska"),
            ("clip.webm", "video/webm"),
        ] {
            let directory = tempdir().expect("temp dir");
            let media = directory.path().join(name);
            std::fs::write(&media, b"media bytes").expect("media file");

            let mut registry = ApprovedImageRegistry::default();
            let approved = registry.approve_path(&media).expect("approved media");
            let response =
                serve_approved_image(&registry, approved.id()).expect("protocol response");

            assert_eq!(response.mime_type(), expected_mime_type, "{name}");
        }
    }

    #[test]
    fn stale_approved_image_id_is_rejected_at_serving_time() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("deleted.png");
        std::fs::write(&image, b"png bytes").expect("image file");

        let mut registry = ApprovedImageRegistry::default();
        let approved = registry.approve_path(&image).expect("approved image");
        std::fs::remove_file(&image).expect("remove approved image");

        assert!(matches!(
            serve_approved_image(&registry, approved.id()),
            Err(ImageProtocolError::FileSystem(error))
                if error.kind() == std::io::ErrorKind::NotFound
        ));
    }

    #[test]
    fn video_larger_than_the_image_size_cap_is_served_instead_of_rejected() {
        let directory = tempdir().expect("temp dir");
        let video = directory.path().join("huge.mkv");
        std::fs::File::create(&video)
            .expect("test video")
            .set_len(MAX_SAFE_FILE_SIZE_BYTES + 1)
            .expect("large sparse file");

        let mut registry = ApprovedImageRegistry::default();
        let approved = registry.approve_path(&video).expect("approved video");

        let response =
            serve_approved_media(&registry, approved.id(), None, None).expect("video response");

        assert!(response.is_partial());
        assert_eq!(response.bytes().len() as u64, VIDEO_CHUNK_BYTES);
        assert_eq!(response.total_bytes(), MAX_SAFE_FILE_SIZE_BYTES + 1);
        assert_eq!(
            response.content_range(),
            Some(
                format!(
                    "bytes 0-{}/{}",
                    VIDEO_CHUNK_BYTES - 1,
                    MAX_SAFE_FILE_SIZE_BYTES + 1
                )
                .as_str()
            )
        );
    }

    #[test]
    fn video_range_request_serves_only_the_requested_slice() {
        let directory = tempdir().expect("temp dir");
        let video = directory.path().join("clip.mp4");
        std::fs::write(&video, b"0123456789").expect("video file");

        let mut registry = ApprovedImageRegistry::default();
        let approved = registry.approve_path(&video).expect("approved video");

        let response = serve_approved_media(&registry, approved.id(), Some("bytes=3-6"), None)
            .expect("range response");

        assert_eq!(response.bytes(), b"3456");
        assert_eq!(response.content_range(), Some("bytes 3-6/10"));
        assert_eq!(response.mime_type(), "video/mp4");
    }

    #[test]
    fn open_ended_and_suffix_video_ranges_are_honoured() {
        let directory = tempdir().expect("temp dir");
        let video = directory.path().join("clip.webm");
        std::fs::write(&video, b"0123456789").expect("video file");

        let mut registry = ApprovedImageRegistry::default();
        let approved = registry.approve_path(&video).expect("approved video");

        let open_ended = serve_approved_media(&registry, approved.id(), Some("bytes=7-"), None)
            .expect("open ended range");
        assert_eq!(open_ended.bytes(), b"789");
        assert_eq!(open_ended.content_range(), Some("bytes 7-9/10"));

        let suffix = serve_approved_media(&registry, approved.id(), Some("bytes=-3"), None)
            .expect("suffix range");
        assert_eq!(suffix.bytes(), b"789");
        assert_eq!(suffix.content_range(), Some("bytes 7-9/10"));
    }

    #[test]
    fn video_range_past_the_end_is_not_satisfiable() {
        let directory = tempdir().expect("temp dir");
        let video = directory.path().join("clip.mov");
        std::fs::write(&video, b"0123456789").expect("video file");

        let mut registry = ApprovedImageRegistry::default();
        let approved = registry.approve_path(&video).expect("approved video");

        assert!(matches!(
            serve_approved_media(&registry, approved.id(), Some("bytes=10-"), None),
            Err(ImageProtocolError::RangeNotSatisfiable { total_bytes: 10 })
        ));
    }

    #[test]
    fn malformed_video_range_falls_back_to_the_start_of_the_file() {
        let directory = tempdir().expect("temp dir");
        let video = directory.path().join("clip.mkv");
        std::fs::write(&video, b"0123456789").expect("video file");

        let mut registry = ApprovedImageRegistry::default();
        let approved = registry.approve_path(&video).expect("approved video");

        for header in ["items=0-1", "bytes=abc-def", "nonsense"] {
            let response = serve_approved_media(&registry, approved.id(), Some(header), None)
                .expect("fallback response");
            assert_eq!(response.bytes(), b"0123456789", "{header}");
            assert_eq!(response.content_range(), Some("bytes 0-9/10"), "{header}");
        }
    }
}
