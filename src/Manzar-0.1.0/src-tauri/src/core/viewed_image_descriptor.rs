use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::Serialize;

use super::{
    image_protocol::image_url,
    image_registry::{ApprovedImageRegistry, ImageRegistryError},
    metadata_preflight::{
        preflight_image, ImageDimensions, ImagePreflight, MetadataPreflightError,
        OversizedImageReason,
    },
};

#[derive(Debug, Default)]
pub struct ViewedImageDescriptors<P = FilesystemImagePreflightReader> {
    preflight_reader: P,
    preflight_by_path: HashMap<PathBuf, CachedImagePreflight>,
}

#[derive(Debug, Clone)]
struct CachedImagePreflight {
    fingerprint: FileFingerprint,
    preflight: ImagePreflight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    size_bytes: u64,
    modified: SystemTime,
}

#[derive(Debug, Default)]
pub struct FilesystemImagePreflightReader;

pub trait ImagePreflightReader {
    fn preflight_image(&mut self, path: &Path) -> Result<ImagePreflight, MetadataPreflightError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewerImage {
    pub id: String,
    pub url: String,
    pub preflight: ImagePreflightDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImagePreflightDto {
    pub file_size_bytes: u64,
    pub dimensions: Option<ImageDimensionsDto>,
    pub oversized: bool,
    pub reasons: Vec<OversizedImageReasonDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ImageDimensionsDto {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum OversizedImageReasonDto {
    FileSize {
        actual_bytes: u64,
        threshold_bytes: u64,
    },
    DecodedRgbaMemory {
        estimated_bytes: u64,
        threshold_bytes: u64,
        width: u32,
        height: u32,
    },
}

#[derive(Debug)]
pub enum ViewedImageDescriptorError {
    ImageRegistry(ImageRegistryError),
    MetadataPreflight(MetadataPreflightError),
}

impl ViewedImageDescriptors<FilesystemImagePreflightReader> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<P: ImagePreflightReader> ViewedImageDescriptors<P> {
    pub fn forget_path(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.preflight_by_path.remove(&path);
    }

    pub fn descriptor_for_path(
        &mut self,
        path: impl AsRef<Path>,
        registry: &mut ApprovedImageRegistry,
    ) -> Result<ViewerImage, ViewedImageDescriptorError> {
        let approved = registry
            .approve_path(path)
            .map_err(ViewedImageDescriptorError::ImageRegistry)?;
        let canonical_path = approved.path().to_path_buf();
        let preflight = self.preflight_for_path(&canonical_path)?;
        let id = approved.id().as_str().to_string();
        let url = image_url(approved.id());

        Ok(ViewerImage {
            id,
            url,
            preflight: ImagePreflightDto::from_preflight(preflight),
        })
    }

    fn preflight_for_path(
        &mut self,
        path: &Path,
    ) -> Result<ImagePreflight, ViewedImageDescriptorError> {
        let fingerprint = FileFingerprint::from_path(path)
            .map_err(ViewedImageDescriptorError::MetadataPreflight)?;

        if let Some(cached) = self.preflight_by_path.get(path) {
            if cached.fingerprint == fingerprint {
                return Ok(cached.preflight.clone());
            }
        }

        let preflight = self
            .preflight_reader
            .preflight_image(path)
            .map_err(ViewedImageDescriptorError::MetadataPreflight)?;
        self.preflight_by_path.insert(
            path.to_path_buf(),
            CachedImagePreflight {
                fingerprint,
                preflight: preflight.clone(),
            },
        );
        Ok(preflight)
    }

    #[cfg(test)]
    fn with_preflight_reader(preflight_reader: P) -> Self {
        Self {
            preflight_reader,
            preflight_by_path: HashMap::new(),
        }
    }
}

impl ImagePreflightReader for FilesystemImagePreflightReader {
    fn preflight_image(&mut self, path: &Path) -> Result<ImagePreflight, MetadataPreflightError> {
        preflight_image(path)
    }
}

impl FileFingerprint {
    fn from_path(path: &Path) -> Result<Self, MetadataPreflightError> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            size_bytes: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        })
    }
}

impl ImagePreflightDto {
    fn from_preflight(preflight: ImagePreflight) -> Self {
        Self {
            file_size_bytes: preflight.file_size_bytes(),
            dimensions: preflight.dimensions().map(ImageDimensionsDto::from),
            oversized: preflight.is_oversized(),
            reasons: preflight
                .reasons()
                .iter()
                .cloned()
                .map(OversizedImageReasonDto::from)
                .collect(),
        }
    }
}

impl From<ImageDimensions> for ImageDimensionsDto {
    fn from(dimensions: ImageDimensions) -> Self {
        Self {
            width: dimensions.width,
            height: dimensions.height,
        }
    }
}

impl From<OversizedImageReason> for OversizedImageReasonDto {
    fn from(reason: OversizedImageReason) -> Self {
        match reason {
            OversizedImageReason::FileSize {
                actual_bytes,
                threshold_bytes,
            } => Self::FileSize {
                actual_bytes,
                threshold_bytes,
            },
            OversizedImageReason::DecodedRgbaMemory {
                estimated_bytes,
                threshold_bytes,
                width,
                height,
            } => Self::DecodedRgbaMemory {
                estimated_bytes,
                threshold_bytes,
                width,
                height,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::image_registry::{ApprovedImageRegistry, ImageRegistryError};
    use tempfile::tempdir;

    #[derive(Debug, Default)]
    struct CountingFilesystemPreflight {
        calls: usize,
    }

    impl ImagePreflightReader for CountingFilesystemPreflight {
        fn preflight_image(
            &mut self,
            path: &Path,
        ) -> Result<ImagePreflight, MetadataPreflightError> {
            self.calls += 1;
            preflight_image(path)
        }
    }

    #[test]
    fn unchanged_supported_image_reuses_cached_preflight() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("image.png");
        std::fs::write(&image, b"image bytes").expect("image file");

        let mut registry = ApprovedImageRegistry::default();
        let mut descriptors =
            ViewedImageDescriptors::with_preflight_reader(CountingFilesystemPreflight::default());

        descriptors
            .descriptor_for_path(&image, &mut registry)
            .expect("first descriptor");
        descriptors
            .descriptor_for_path(&image, &mut registry)
            .expect("cached descriptor");

        assert_eq!(descriptors.preflight_reader.calls, 1);
    }

    #[test]
    fn changed_file_metadata_recomputes_preflight_without_changing_opaque_descriptor() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("image.png");
        std::fs::write(&image, b"small").expect("image file");

        let mut registry = ApprovedImageRegistry::default();
        let mut descriptors =
            ViewedImageDescriptors::with_preflight_reader(CountingFilesystemPreflight::default());

        let first = descriptors
            .descriptor_for_path(&image, &mut registry)
            .expect("first descriptor");
        std::fs::write(&image, b"larger image bytes").expect("changed image file");
        let changed = descriptors
            .descriptor_for_path(&image, &mut registry)
            .expect("changed descriptor");

        assert_eq!(changed.id, first.id);
        assert_eq!(changed.url, first.url);
        assert_eq!(changed.preflight.file_size_bytes, 18);
        assert_eq!(descriptors.preflight_reader.calls, 2);
    }

    #[test]
    fn hidden_unsupported_and_deleted_files_are_not_described_from_cache() {
        let directory = tempdir().expect("temp dir");
        let supported = directory.path().join("image.png");
        let hidden = directory.path().join(".hidden.png");
        let unsupported = directory.path().join("notes.txt");
        std::fs::write(&supported, b"image bytes").expect("supported image");
        std::fs::write(&hidden, b"hidden image").expect("hidden image");
        std::fs::write(&unsupported, b"notes").expect("unsupported file");

        let mut registry = ApprovedImageRegistry::default();
        let mut descriptors =
            ViewedImageDescriptors::with_preflight_reader(CountingFilesystemPreflight::default());

        descriptors
            .descriptor_for_path(&supported, &mut registry)
            .expect("cached supported descriptor");
        std::fs::remove_file(&supported).expect("deleted supported image");

        assert!(matches!(
            descriptors.descriptor_for_path(&hidden, &mut registry),
            Err(ViewedImageDescriptorError::ImageRegistry(
                ImageRegistryError::HiddenImage
            ))
        ));
        assert!(matches!(
            descriptors.descriptor_for_path(&unsupported, &mut registry),
            Err(ViewedImageDescriptorError::ImageRegistry(
                ImageRegistryError::UnsupportedImage
            ))
        ));
        assert!(matches!(
            descriptors.descriptor_for_path(&supported, &mut registry),
            Err(ViewedImageDescriptorError::ImageRegistry(
                ImageRegistryError::FileSystem(error)
            )) if error.kind() == std::io::ErrorKind::NotFound
        ));
        assert_eq!(descriptors.preflight_reader.calls, 1);
    }
}
