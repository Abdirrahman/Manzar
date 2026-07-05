use std::path::Path;

use super::{
    image_registry::{ApprovedImageRegistry, ImageId},
    metadata_preflight::MAX_SAFE_FILE_SIZE_BYTES,
};

pub const IMAGE_PROTOCOL_SCHEME: &str = "manzar-image";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolImageResponse {
    mime_type: &'static str,
    bytes: Vec<u8>,
}

#[derive(Debug)]
pub enum ImageProtocolError {
    UnknownImageId,
    UnsupportedImage,
    OversizedImage,
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
    let path = registry
        .path_for(id)
        .ok_or(ImageProtocolError::UnknownImageId)?;
    let mime_type = supported_image_mime_type(path).ok_or(ImageProtocolError::UnsupportedImage)?;
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_SAFE_FILE_SIZE_BYTES {
        return Err(ImageProtocolError::OversizedImage);
    }
    let bytes = std::fs::read(path)?;

    Ok(ProtocolImageResponse { mime_type, bytes })
}

fn supported_image_mime_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
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
    fn supported_image_mime_types_are_served() {
        for (name, expected_mime_type) in [
            ("image.png", "image/png"),
            ("image.jpg", "image/jpeg"),
            ("image.JPG", "image/jpeg"),
            ("image.jpeg", "image/jpeg"),
            ("image.webp", "image/webp"),
            ("image.gif", "image/gif"),
            ("image.bmp", "image/bmp"),
        ] {
            let directory = tempdir().expect("temp dir");
            let image = directory.path().join(name);
            std::fs::write(&image, b"image bytes").expect("image file");

            let mut registry = ApprovedImageRegistry::default();
            let approved = registry.approve_path(&image).expect("approved image");
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
}
