use std::path::Path;

pub const MAX_SAFE_FILE_SIZE_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_SAFE_DECODED_RGBA_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePreflight {
    file_size_bytes: u64,
    dimensions: Option<ImageDimensions>,
    reasons: Vec<OversizedImageReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OversizedImageReason {
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
pub enum MetadataPreflightError {
    FileSystem(std::io::Error),
}

impl From<std::io::Error> for MetadataPreflightError {
    fn from(error: std::io::Error) -> Self {
        Self::FileSystem(error)
    }
}

impl ImagePreflight {
    pub fn file_size_bytes(&self) -> u64 {
        self.file_size_bytes
    }

    pub fn dimensions(&self) -> Option<ImageDimensions> {
        self.dimensions
    }

    pub fn is_oversized(&self) -> bool {
        !self.reasons.is_empty()
    }

    pub fn reasons(&self) -> &[OversizedImageReason] {
        &self.reasons
    }
}

pub fn preflight_image(path: impl AsRef<Path>) -> Result<ImagePreflight, MetadataPreflightError> {
    let path = path.as_ref();
    let metadata = std::fs::metadata(path)?;
    let file_size_bytes = metadata.len();
    let dimensions = image::image_dimensions(path)
        .ok()
        .map(|(width, height)| ImageDimensions { width, height });
    let mut reasons = Vec::new();

    if file_size_bytes > MAX_SAFE_FILE_SIZE_BYTES {
        reasons.push(OversizedImageReason::FileSize {
            actual_bytes: file_size_bytes,
            threshold_bytes: MAX_SAFE_FILE_SIZE_BYTES,
        });
    }

    if let Some(dimensions) = dimensions {
        let estimated_bytes = u64::from(dimensions.width)
            .saturating_mul(u64::from(dimensions.height))
            .saturating_mul(4);
        if estimated_bytes > MAX_SAFE_DECODED_RGBA_BYTES {
            reasons.push(OversizedImageReason::DecodedRgbaMemory {
                estimated_bytes,
                threshold_bytes: MAX_SAFE_DECODED_RGBA_BYTES,
                width: dimensions.width,
                height: dimensions.height,
            });
        }
    }

    Ok(ImagePreflight {
        file_size_bytes,
        dimensions,
        reasons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn image_larger_than_200_mb_is_oversized() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("large.png");
        std::fs::File::create(&image)
            .expect("test image")
            .set_len(MAX_SAFE_FILE_SIZE_BYTES + 1)
            .expect("large sparse file");

        let preflight = preflight_image(&image).expect("metadata preflight");

        assert!(preflight.is_oversized());
        assert_eq!(
            preflight.reasons(),
            &[OversizedImageReason::FileSize {
                actual_bytes: MAX_SAFE_FILE_SIZE_BYTES + 1,
                threshold_bytes: MAX_SAFE_FILE_SIZE_BYTES,
            }]
        );
    }

    #[test]
    fn decoded_rgba_memory_larger_than_512_mb_is_oversized() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("huge-dimensions.bmp");
        write_bmp_header(&image, 16_384, 8_193);

        let preflight = preflight_image(&image).expect("metadata preflight");

        assert!(preflight.is_oversized());
        assert_eq!(
            preflight.dimensions(),
            Some(ImageDimensions {
                width: 16_384,
                height: 8_193
            })
        );
        assert_eq!(
            preflight.reasons(),
            &[OversizedImageReason::DecodedRgbaMemory {
                estimated_bytes: 536_936_448,
                threshold_bytes: MAX_SAFE_DECODED_RGBA_BYTES,
                width: 16_384,
                height: 8_193,
            }]
        );
    }

    #[test]
    fn threshold_values_are_not_oversized() {
        let directory = tempdir().expect("temp dir");
        let exact_file_threshold = directory.path().join("exact-file-threshold.png");
        std::fs::File::create(&exact_file_threshold)
            .expect("test image")
            .set_len(MAX_SAFE_FILE_SIZE_BYTES)
            .expect("large sparse file");

        let exact_memory_threshold = directory.path().join("exact-memory-threshold.bmp");
        write_bmp_header(&exact_memory_threshold, 16_384, 8_192);

        let file_preflight = preflight_image(&exact_file_threshold).expect("metadata preflight");
        let memory_preflight =
            preflight_image(&exact_memory_threshold).expect("metadata preflight");

        assert!(!file_preflight.is_oversized());
        assert!(file_preflight.reasons().is_empty());
        assert!(!memory_preflight.is_oversized());
        assert!(memory_preflight.reasons().is_empty());
    }

    fn write_bmp_header(path: &std::path::Path, width: i32, height: i32) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&54_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&54_u32.to_le_bytes());
        bytes.extend_from_slice(&40_u32.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&32_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        std::fs::write(path, bytes).expect("bmp header");
    }
}
