use std::path::Path;

use serde::Serialize;

pub use super::viewed_image_descriptor::ViewerImage;

/// ponytail: a flat cap with a wholesale clear, matching the preflight cache.
/// Only the image on screen has to stay approved, so a clear costs one
/// re-approval; per-entry eviction would need liveness tracking nothing needs yet.
const MAX_APPROVED_IMAGES: usize = 512;

use super::{
    crop::{crop_image, CropError, CropRect},
    file_actions::{self, FileActionError, TrashDeleter},
    image_registry::{ApprovedImageRegistry, ImageRegistryError},
    image_sequence::{ImageSequence, ImageSequenceError},
    metadata_preflight::MetadataPreflightError,
    sequence_ordering::SequenceOrdering,
    viewed_image_descriptor::{ViewedImageDescriptorError, ViewedImageDescriptors},
};

#[derive(Debug)]
pub struct ViewerSession {
    sequence: Option<ImageSequence>,
    sequence_ordering: SequenceOrdering,
    image_descriptors: ViewedImageDescriptors,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewerSnapshot {
    pub current: Option<ViewerImage>,
    pub current_position: Option<usize>,
    pub count: usize,
    pub sequence_ordering: SequenceOrdering,
}

#[derive(Debug)]
pub enum ViewerSessionError {
    ImageSequence(ImageSequenceError),
    ImageRegistry(ImageRegistryError),
    MetadataPreflight(MetadataPreflightError),
    FileAction(FileActionError),
    NoCurrentImage,
    Crop(CropError),
}

impl Default for ViewerSession {
    fn default() -> Self {
        Self::new(SequenceOrdering::default())
    }
}

impl ViewerSession {
    pub fn new(sequence_ordering: SequenceOrdering) -> Self {
        Self {
            sequence: None,
            sequence_ordering,
            image_descriptors: ViewedImageDescriptors::new(),
        }
    }

    pub fn open_single_image(
        &mut self,
        path: impl AsRef<Path>,
        registry: &mut ApprovedImageRegistry,
    ) -> Result<ViewerSnapshot, ViewerSessionError> {
        let sequence = ImageSequence::from_single_image(path, self.sequence_ordering)
            .map_err(ViewerSessionError::ImageSequence)?;
        self.sequence = Some(sequence);
        self.reset_caches(registry);
        self.snapshot(registry)
    }

    pub fn open_image_selection(
        &mut self,
        paths: impl IntoIterator<Item = impl AsRef<Path>>,
        registry: &mut ApprovedImageRegistry,
    ) -> Result<ViewerSnapshot, ViewerSessionError> {
        let sequence = ImageSequence::from_image_selection(paths, self.sequence_ordering)
            .map_err(ViewerSessionError::ImageSequence)?;
        self.sequence = Some(sequence);
        self.reset_caches(registry);
        self.snapshot(registry)
    }

    pub fn open_folder(
        &mut self,
        folder: impl AsRef<Path>,
        registry: &mut ApprovedImageRegistry,
    ) -> Result<ViewerSnapshot, ViewerSessionError> {
        let sequence = ImageSequence::from_folder(folder, self.sequence_ordering)
            .map_err(ViewerSessionError::ImageSequence)?;
        self.sequence = Some(sequence);
        self.reset_caches(registry);
        self.snapshot(registry)
    }

    pub fn next(
        &mut self,
        registry: &mut ApprovedImageRegistry,
    ) -> Result<ViewerSnapshot, ViewerSessionError> {
        if let Some(sequence) = &mut self.sequence {
            sequence.next();
        }
        self.snapshot(registry)
    }

    pub fn previous(
        &mut self,
        registry: &mut ApprovedImageRegistry,
    ) -> Result<ViewerSnapshot, ViewerSessionError> {
        if let Some(sequence) = &mut self.sequence {
            sequence.previous();
        }
        self.snapshot(registry)
    }

    pub fn set_sequence_ordering(
        &mut self,
        ordering: SequenceOrdering,
        registry: &mut ApprovedImageRegistry,
    ) -> Result<ViewerSnapshot, ViewerSessionError> {
        self.sequence_ordering = ordering;

        if let Some(sequence) = &mut self.sequence {
            sequence.reorder(ordering);
        }

        self.snapshot(registry)
    }

    pub fn rename_current_image(
        &mut self,
        new_stem: &str,
        registry: &mut ApprovedImageRegistry,
    ) -> Result<ViewerSnapshot, ViewerSessionError> {
        let current_path = self.current_path_buf()?;
        let renamed_path = file_actions::rename_current_image(&current_path, new_stem)
            .map_err(ViewerSessionError::FileAction)?;
        registry.revoke_path(&current_path);
        self.image_descriptors.forget_path(&current_path);

        let sequence = self
            .sequence
            .as_mut()
            .ok_or(ViewerSessionError::NoCurrentImage)?;

        sequence
            .replace_current_path(&renamed_path)
            .map_err(ViewerSessionError::ImageSequence)?;
        sequence.reorder(self.sequence_ordering);

        self.snapshot(registry)
    }

    pub fn crop_current_image(
        &mut self,
        image_id: &str,
        rect: CropRect,
        registry: &mut ApprovedImageRegistry,
    ) -> Result<ViewerSnapshot, ViewerSessionError> {
        let path = self.current_path_buf()?;
        let current = registry
            .approve_path(&path)
            .map_err(ViewerSessionError::ImageRegistry)?;
        if current.id().as_str() != image_id {
            return Err(ViewerSessionError::Crop(CropError::SourceChanged));
        }
        crop_image(&path, rect).map_err(ViewerSessionError::Crop)?;
        registry.revoke_path(&path);
        self.image_descriptors.forget_path(&path);
        if let Some(sequence) = &mut self.sequence {
            sequence
                .replace_current_path(&path)
                .map_err(ViewerSessionError::ImageSequence)?;
            sequence.reorder(self.sequence_ordering);
        }
        self.snapshot(registry)
    }

    pub fn trash_current_image(
        &mut self,
        registry: &mut ApprovedImageRegistry,
        deleter: &impl TrashDeleter,
    ) -> Result<ViewerSnapshot, ViewerSessionError> {
        let current_path = self.current_path_buf()?;
        file_actions::trash_current_image(&current_path, deleter)
            .map_err(ViewerSessionError::FileAction)?;
        registry.revoke_path(&current_path);
        self.image_descriptors.forget_path(&current_path);

        if let Some(sequence) = &mut self.sequence {
            sequence.remove_current();
            if sequence.is_empty() {
                self.sequence = None;
            }
        }

        self.snapshot(registry)
    }

    fn reset_caches(&mut self, registry: &mut ApprovedImageRegistry) {
        registry.clear();
        self.image_descriptors.clear();
    }

    pub fn snapshot(
        &mut self,
        registry: &mut ApprovedImageRegistry,
    ) -> Result<ViewerSnapshot, ViewerSessionError> {
        // Only the current image needs to stay approved, so bound the caches
        // before adding to them; walking a large folder would otherwise retain
        // an entry for every image ever displayed.
        if registry.len() >= MAX_APPROVED_IMAGES {
            self.reset_caches(registry);
        }

        let Some(sequence) = &self.sequence else {
            return Ok(ViewerSnapshot {
                current: None,
                current_position: None,
                count: 0,
                sequence_ordering: self.sequence_ordering,
            });
        };

        let current_path = sequence
            .current_path()
            .ok_or(ViewerSessionError::NoCurrentImage)?;
        let current = self
            .image_descriptors
            .descriptor_for_path(current_path, registry)
            .map_err(ViewerSessionError::from)?;

        Ok(ViewerSnapshot {
            current: Some(current),
            current_position: sequence.current_position(),
            count: sequence.len(),
            sequence_ordering: self.sequence_ordering,
        })
    }

    fn current_path_buf(&self) -> Result<std::path::PathBuf, ViewerSessionError> {
        let sequence = self
            .sequence
            .as_ref()
            .ok_or(ViewerSessionError::NoCurrentImage)?;
        sequence
            .current_path()
            .map(Path::to_path_buf)
            .ok_or(ViewerSessionError::NoCurrentImage)
    }
}

impl ViewerSessionError {
    pub fn frontend_safe_message(&self) -> &'static str {
        match self {
            Self::ImageSequence(ImageSequenceError::HiddenImage) => {
                "hidden files are not supported"
            }
            Self::ImageSequence(ImageSequenceError::NoParentFolder) => "file has no parent folder",
            Self::ImageSequence(ImageSequenceError::NoSupportedImages) => {
                "no supported images or videos were found"
            }
            Self::ImageSequence(ImageSequenceError::UnsupportedImage) => "unsupported file format",
            Self::ImageSequence(ImageSequenceError::FileSystem(error))
            | Self::ImageRegistry(ImageRegistryError::FileSystem(error))
            | Self::MetadataPreflight(MetadataPreflightError::FileSystem(error)) => {
                if error.kind() == std::io::ErrorKind::NotFound {
                    "file was not found"
                } else {
                    "failed to access file"
                }
            }
            Self::ImageRegistry(ImageRegistryError::HiddenImage) => {
                "hidden files are not supported"
            }
            Self::ImageRegistry(ImageRegistryError::UnsupportedImage) => "unsupported file format",
            Self::FileAction(FileActionError::EmptyStem) => "rename name cannot be empty",
            Self::FileAction(FileActionError::HiddenTarget) => {
                "rename name cannot start with a dot"
            }
            Self::FileAction(FileActionError::PathSeparator) => {
                "rename name cannot contain path separators"
            }
            Self::FileAction(FileActionError::TargetAlreadyExists) => {
                "a file with that name already exists"
            }
            Self::FileAction(FileActionError::TrashDeletionFailed) => {
                "failed to move file to trash"
            }
            Self::FileAction(FileActionError::NoParentFolder) => "file has no parent folder",
            Self::FileAction(FileActionError::MissingExtension) => "unsupported file format",
            Self::FileAction(FileActionError::FileSystem(error)) => {
                if error.kind() == std::io::ErrorKind::NotFound {
                    "file was not found"
                } else {
                    "failed to update file"
                }
            }
            Self::Crop(CropError::Unsupported) => "Crop supports still PNG, JPEG, WebP and BMP images. Animated images are left unchanged.",
            Self::Crop(CropError::InvalidBounds) => "Select a crop inside the image with a width and height of at least one pixel.",
            Self::Crop(CropError::ImageTooLarge) => "This image is too large to crop safely.",
            Self::Crop(CropError::SourceChanged) => "The source image changed. Reopen it before cropping.",
            Self::Crop(CropError::Image(_)) => "Could not encode this crop. The original image is unchanged.",
            Self::Crop(CropError::FileSystem(_)) => "Could not save the crop. Check available space and folder permissions. The original image is unchanged.",
            Self::NoCurrentImage => "nothing is currently open",
        }
    }
}

impl From<ViewedImageDescriptorError> for ViewerSessionError {
    fn from(error: ViewedImageDescriptorError) -> Self {
        match error {
            ViewedImageDescriptorError::ImageRegistry(error) => Self::ImageRegistry(error),
            ViewedImageDescriptorError::MetadataPreflight(error) => Self::MetadataPreflight(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        file_actions::{TrashDeleteError, TrashDeleter},
        image_protocol::{serve_approved_image, ImageProtocolError},
        image_registry::{ApprovedImageRegistry, ImageId},
        sequence_ordering::SequenceOrdering,
    };
    use std::{cell::RefCell, path::PathBuf};
    use tempfile::tempdir;

    #[derive(Debug, Default)]
    struct TestTrashDeleter {
        trashed: RefCell<Vec<PathBuf>>,
        fail: bool,
    }

    impl TrashDeleter for TestTrashDeleter {
        fn trash(&self, path: &Path) -> Result<(), TrashDeleteError> {
            if self.fail {
                return Err(TrashDeleteError);
            }

            self.trashed.borrow_mut().push(path.to_path_buf());
            std::fs::remove_file(path).map_err(|_| TrashDeleteError)?;
            Ok(())
        }
    }

    #[test]
    fn crop_refreshes_the_current_file_and_rejects_stale_selections() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("source.png");
        let sibling = directory.path().join("sibling.png");
        image::RgbaImage::new(16, 12).save(&path).unwrap();
        image::RgbaImage::new(5, 5).save(&sibling).unwrap();
        let sibling_bytes = std::fs::read(&sibling).unwrap();
        let mut session = ViewerSession::default();
        let mut registry = ApprovedImageRegistry::default();
        let before = session
            .open_single_image(&path, &mut registry)
            .unwrap()
            .current
            .unwrap();
        let rect = CropRect {
            x: 2,
            y: 3,
            width: 8,
            height: 6,
        };
        let after = session
            .crop_current_image(&before.id, rect, &mut registry)
            .unwrap();
        let current = after.current.unwrap();
        assert_eq!(after.count, 2);
        assert_eq!(current.filename, "source.png");
        assert_ne!(current.id, before.id);
        assert_ne!(current.url, before.url);
        assert_eq!(image::image_dimensions(&path).unwrap(), (8, 6));
        assert_eq!(current.preflight.dimensions.unwrap().width, 8);
        assert!(registry
            .path_for(&ImageId::from_opaque(before.id.clone()))
            .is_none());
        assert!(session
            .crop_current_image(&before.id, rect, &mut registry)
            .is_err());
        session.next(&mut registry).unwrap();
        let revisited = session.previous(&mut registry).unwrap().current.unwrap();
        assert_eq!(revisited.id, current.id);
        assert_eq!(std::fs::read(sibling).unwrap(), sibling_bytes);
    }

    #[test]
    fn opening_single_image_returns_frontend_safe_current_image_snapshot() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("image1.png");
        let opened = directory.path().join("private-image2.png");
        std::fs::write(&first, b"first image").expect("first image");
        std::fs::write(&opened, b"opened image").expect("opened image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);

        let snapshot = session
            .open_single_image(&opened, &mut registry)
            .expect("viewer snapshot");

        let current = snapshot.current.expect("current image");
        assert_eq!(snapshot.count, 2);
        assert_eq!(snapshot.current_position, Some(2));
        assert_eq!(snapshot.sequence_ordering, SequenceOrdering::NaturalName);
        assert_eq!(
            current.url,
            format!("manzar-image://localhost/{}", current.id)
        );
        assert!(!current.id.contains("private-image2"));
        assert!(!current.url.contains("private-image2"));
        assert!(!current
            .url
            .contains(directory.path().to_string_lossy().as_ref()));
        assert!(!current.preflight.oversized);
    }

    #[test]
    fn walking_a_large_sequence_keeps_the_approved_registry_bounded() {
        let directory = tempdir().expect("temp dir");
        for index in 0..(MAX_APPROVED_IMAGES + 20) {
            std::fs::write(
                directory.path().join(format!("image{index:04}.png")),
                b"image bytes",
            )
            .expect("image file");
        }

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        session
            .open_folder(directory.path(), &mut registry)
            .expect("open folder");

        for _ in 0..(MAX_APPROVED_IMAGES + 20) {
            session.next(&mut registry).expect("navigate next");
        }

        assert!(registry.len() <= MAX_APPROVED_IMAGES);
        // The image on screen is still servable after any bounding clear.
        let snapshot = session.snapshot(&mut registry).expect("snapshot");
        let current = snapshot.current.expect("current image");
        assert!(serve_approved_image(&registry, &ImageId::from_opaque(current.id)).is_ok());
    }

    #[test]
    fn navigation_wraps_and_returns_updated_current_image_snapshot() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("image1.png");
        let second = directory.path().join("private-image2.png");
        std::fs::write(&first, b"first image").expect("first image");
        std::fs::write(&second, b"second image").expect("second image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        session
            .open_single_image(&first, &mut registry)
            .expect("opened first image");

        let second_snapshot = session.next(&mut registry).expect("next image");
        let wrapped_snapshot = session.next(&mut registry).expect("wrapped image");
        let previous_snapshot = session.previous(&mut registry).expect("previous image");

        assert_eq!(second_snapshot.current_position, Some(2));
        assert_eq!(wrapped_snapshot.current_position, Some(1));
        assert_eq!(previous_snapshot.current_position, Some(2));

        let second_current = second_snapshot.current.expect("second current image");
        assert_eq!(
            second_current.url,
            format!("manzar-image://localhost/{}", second_current.id)
        );
        assert!(!second_current.url.contains("private-image2"));
        assert!(!second_current
            .url
            .contains(directory.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn opening_new_single_image_revokes_previous_approved_image_id() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("first.png");
        let second = directory.path().join("second.png");
        std::fs::write(&first, b"first image").expect("first image");
        std::fs::write(&second, b"second image").expect("second image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        let opened = session
            .open_single_image(&first, &mut registry)
            .expect("opened first image")
            .current
            .expect("first current image");
        let previous_id = ImageId::from_opaque(opened.id);

        session
            .open_single_image(&second, &mut registry)
            .expect("opened second image");

        assert!(matches!(
            serve_approved_image(&registry, &previous_id),
            Err(ImageProtocolError::UnknownImageId)
        ));
    }

    #[test]
    fn opening_new_image_selection_revokes_previous_approved_image_id() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("first.png");
        let second = directory.path().join("second.png");
        std::fs::write(&first, b"first image").expect("first image");
        std::fs::write(&second, b"second image").expect("second image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        let opened = session
            .open_single_image(&first, &mut registry)
            .expect("opened first image")
            .current
            .expect("first current image");
        let previous_id = ImageId::from_opaque(opened.id);

        session
            .open_image_selection([&second], &mut registry)
            .expect("opened image selection");

        assert!(matches!(
            serve_approved_image(&registry, &previous_id),
            Err(ImageProtocolError::UnknownImageId)
        ));
    }

    #[test]
    fn opening_new_folder_revokes_previous_approved_image_id() {
        let first_directory = tempdir().expect("first temp dir");
        let second_directory = tempdir().expect("second temp dir");
        let first = first_directory.path().join("first.png");
        let second = second_directory.path().join("second.png");
        std::fs::write(&first, b"first image").expect("first image");
        std::fs::write(&second, b"second image").expect("second image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        let opened = session
            .open_single_image(&first, &mut registry)
            .expect("opened first image")
            .current
            .expect("first current image");
        let previous_id = ImageId::from_opaque(opened.id);

        session
            .open_folder(second_directory.path(), &mut registry)
            .expect("opened folder");

        assert!(matches!(
            serve_approved_image(&registry, &previous_id),
            Err(ImageProtocolError::UnknownImageId)
        ));
    }

    #[test]
    fn repeated_navigation_to_the_same_supported_image_reuses_its_opaque_descriptor() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("image1.png");
        let second = directory.path().join("image2.png");
        std::fs::write(&first, b"first image").expect("first image");
        std::fs::write(&second, b"second image").expect("second image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);

        let opened = session
            .open_single_image(&first, &mut registry)
            .expect("opened first image")
            .current
            .expect("opened current image");
        session.next(&mut registry).expect("next image");
        let wrapped = session
            .next(&mut registry)
            .expect("wrapped to first image")
            .current
            .expect("wrapped current image");

        assert_eq!(wrapped.id, opened.id);
        assert_eq!(wrapped.url, opened.url);
    }

    #[test]
    fn opening_image_selection_uses_only_explicit_supported_images() {
        let directory = tempdir().expect("temp dir");
        let selected_first = directory.path().join("image1.png");
        let selected_second = directory.path().join("private-image2.gif");
        let unselected_sibling = directory.path().join("image3.jpg");
        let unsupported = directory.path().join("notes.txt");
        std::fs::write(&selected_first, b"selected first").expect("selected first");
        std::fs::write(&selected_second, b"selected second").expect("selected second");
        std::fs::write(&unselected_sibling, b"unselected sibling").expect("unselected sibling");
        std::fs::write(&unsupported, b"unsupported").expect("unsupported");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);

        let snapshot = session
            .open_image_selection(
                [&selected_second, &unsupported, &selected_first],
                &mut registry,
            )
            .expect("opened selection");

        assert_eq!(snapshot.count, 2);
        assert_eq!(snapshot.current_position, Some(1));
        let current = snapshot.current.expect("current image");
        assert!(!current.url.contains("private-image2"));
        assert!(!current
            .url
            .contains(directory.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn opening_folder_uses_supported_non_hidden_images_in_that_folder() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("image1.png");
        let second = directory.path().join("private-image2.webp");
        let hidden = directory.path().join(".hidden.jpg");
        let unsupported = directory.path().join("notes.txt");
        std::fs::write(&first, b"first").expect("first");
        std::fs::write(&second, b"second").expect("second");
        std::fs::write(&hidden, b"hidden").expect("hidden");
        std::fs::write(&unsupported, b"unsupported").expect("unsupported");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);

        let snapshot = session
            .open_folder(directory.path(), &mut registry)
            .expect("opened folder");

        assert_eq!(snapshot.count, 2);
        assert_eq!(snapshot.current_position, Some(1));
        let current = snapshot.current.expect("current image");
        assert!(!current
            .url
            .contains(directory.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn unsupported_single_image_error_is_frontend_safe() {
        let directory = tempdir().expect("temp dir");
        let unsupported = directory.path().join("private-notes.txt");
        std::fs::write(&unsupported, b"notes").expect("unsupported file");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);

        let error = session
            .open_single_image(&unsupported, &mut registry)
            .expect_err("unsupported image");
        let message = error.frontend_safe_message();

        assert_eq!(message, "unsupported file format");
        assert!(!message.contains("private-notes"));
        assert!(!message.contains(directory.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn changing_sequence_ordering_without_an_open_image_updates_the_snapshot() {
        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NewestModifiedFirst);

        let snapshot = session
            .set_sequence_ordering(SequenceOrdering::NaturalName, &mut registry)
            .expect("updated ordering snapshot");

        assert_eq!(snapshot.current, None);
        assert_eq!(snapshot.current_position, None);
        assert_eq!(snapshot.count, 0);
        assert_eq!(snapshot.sequence_ordering, SequenceOrdering::NaturalName);
    }

    #[test]
    fn opening_sequence_uses_the_active_sequence_ordering() {
        let directory = tempdir().expect("temp dir");
        let large_name_first = directory.path().join("a-large.png");
        let small_name_second = directory.path().join("z-small.png");
        std::fs::write(&large_name_first, b"large image contents").expect("large image");
        std::fs::write(&small_name_second, b"s").expect("small image");

        let canonical_small = small_name_second.canonicalize().expect("canonical small");
        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::SizeSmallestFirst);

        let snapshot = session
            .open_folder(directory.path(), &mut registry)
            .expect("opened folder");
        let current = snapshot.current.expect("current image");

        assert_eq!(
            snapshot.sequence_ordering,
            SequenceOrdering::SizeSmallestFirst
        );
        assert_eq!(snapshot.current_position, Some(1));
        assert_eq!(
            registry.path_for(&crate::core::image_registry::ImageId::from_opaque(
                current.id
            )),
            Some(canonical_small.as_path())
        );
    }

    #[test]
    fn changing_sequence_ordering_preserves_current_image_in_reordered_sequence() {
        let directory = tempdir().expect("temp dir");
        let large_name_first = directory.path().join("a-large.png");
        let small_name_second = directory.path().join("z-small.png");
        std::fs::write(&large_name_first, b"large image contents").expect("large image");
        std::fs::write(&small_name_second, b"s").expect("small image");
        let canonical_large = large_name_first.canonicalize().expect("canonical large");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        let opened_snapshot = session
            .open_folder(directory.path(), &mut registry)
            .expect("opened folder");
        let opened_current = opened_snapshot.current.expect("opened current image");

        assert_eq!(
            registry.path_for(&crate::core::image_registry::ImageId::from_opaque(
                opened_current.id
            )),
            Some(canonical_large.as_path())
        );
        assert_eq!(opened_snapshot.current_position, Some(1));

        let reordered_snapshot = session
            .set_sequence_ordering(SequenceOrdering::SizeSmallestFirst, &mut registry)
            .expect("reordered snapshot");
        let reordered_current = reordered_snapshot.current.expect("reordered current image");

        assert_eq!(
            registry.path_for(&crate::core::image_registry::ImageId::from_opaque(
                reordered_current.id
            )),
            Some(canonical_large.as_path())
        );
        assert_eq!(
            reordered_snapshot.sequence_ordering,
            SequenceOrdering::SizeSmallestFirst
        );
        assert_eq!(reordered_snapshot.current_position, Some(2));
        assert_eq!(reordered_snapshot.count, 2);
    }

    #[test]
    fn renaming_current_image_updates_the_current_descriptor_and_reorders_sequence() {
        let directory = tempdir().expect("temp dir");
        let original = directory.path().join("a-current.png");
        let sibling = directory.path().join("b-sibling.png");
        let renamed = directory.path().join("z-renamed.png");
        std::fs::write(&original, b"current").expect("current image");
        std::fs::write(&sibling, b"sibling").expect("sibling image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        session
            .open_single_image(&original, &mut registry)
            .expect("opened image");

        let snapshot = session
            .rename_current_image("z-renamed", &mut registry)
            .expect("renamed current image");
        let current = snapshot.current.expect("current image");
        let canonical_renamed = renamed.canonicalize().expect("canonical renamed");

        assert!(!original.exists());
        assert_eq!(snapshot.count, 2);
        assert_eq!(snapshot.current_position, Some(2));
        assert_eq!(
            registry.path_for(&crate::core::image_registry::ImageId::from_opaque(
                current.id
            )),
            Some(canonical_renamed.as_path())
        );
    }

    #[test]
    fn renamed_current_image_revokes_the_original_opaque_id() {
        let directory = tempdir().expect("temp dir");
        let original = directory.path().join("current.png");
        let sibling = directory.path().join("sibling.png");
        std::fs::write(&original, b"current").expect("current image");
        std::fs::write(&sibling, b"sibling").expect("sibling image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        let opened = session
            .open_single_image(&original, &mut registry)
            .expect("opened image")
            .current
            .expect("opened current image");
        let original_id = ImageId::from_opaque(opened.id);

        session
            .rename_current_image("renamed", &mut registry)
            .expect("renamed current image");

        assert!(matches!(
            serve_approved_image(&registry, &original_id),
            Err(ImageProtocolError::UnknownImageId)
        ));
    }

    #[test]
    fn invalid_rename_error_is_frontend_safe() {
        let directory = tempdir().expect("temp dir");
        let current = directory.path().join("private-current.png");
        std::fs::write(&current, b"current").expect("current image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        session
            .open_single_image(&current, &mut registry)
            .expect("opened image");

        let error = session
            .rename_current_image("bad/name", &mut registry)
            .expect_err("invalid rename");
        let message = error.frontend_safe_message();

        assert_eq!(message, "rename name cannot contain path separators");
        assert!(!message.contains("private-current"));
        assert!(!message.contains(directory.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn trashing_current_image_shows_next_image() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("image1.png");
        let second = directory.path().join("private-image2.png");
        std::fs::write(&first, b"first").expect("first image");
        std::fs::write(&second, b"second").expect("second image");
        let canonical_first = first.canonicalize().expect("canonical first");
        let canonical_second = second.canonicalize().expect("canonical second");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        session
            .open_single_image(&first, &mut registry)
            .expect("opened image");
        let deleter = TestTrashDeleter::default();

        let snapshot = session
            .trash_current_image(&mut registry, &deleter)
            .expect("trashed current image");
        let current = snapshot.current.expect("current image");

        assert_eq!(deleter.trashed.borrow().as_slice(), &[canonical_first]);
        assert_eq!(snapshot.count, 1);
        assert_eq!(snapshot.current_position, Some(1));
        assert_eq!(
            registry.path_for(&crate::core::image_registry::ImageId::from_opaque(
                current.id
            )),
            Some(canonical_second.as_path())
        );
    }

    #[test]
    fn trashed_current_image_revokes_the_original_opaque_id() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("image1.png");
        let second = directory.path().join("image2.png");
        std::fs::write(&first, b"first").expect("first image");
        std::fs::write(&second, b"second").expect("second image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        let opened = session
            .open_single_image(&first, &mut registry)
            .expect("opened image")
            .current
            .expect("opened current image");
        let original_id = ImageId::from_opaque(opened.id);
        let deleter = TestTrashDeleter::default();

        session
            .trash_current_image(&mut registry, &deleter)
            .expect("trashed current image");

        assert!(matches!(
            serve_approved_image(&registry, &original_id),
            Err(ImageProtocolError::UnknownImageId)
        ));
    }

    #[test]
    fn trashing_the_only_image_returns_empty_snapshot() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("image1.png");
        std::fs::write(&image, b"image").expect("image file");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        session
            .open_single_image(&image, &mut registry)
            .expect("opened image");
        let deleter = TestTrashDeleter::default();

        let snapshot = session
            .trash_current_image(&mut registry, &deleter)
            .expect("trashed current image");

        assert_eq!(snapshot.current, None);
        assert_eq!(snapshot.current_position, None);
        assert_eq!(snapshot.count, 0);
    }

    #[test]
    fn trash_failure_keeps_current_image_visible() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("image1.png");
        let second = directory.path().join("image2.png");
        std::fs::write(&first, b"first").expect("first image");
        std::fs::write(&second, b"second").expect("second image");

        let mut registry = ApprovedImageRegistry::default();
        let mut session = ViewerSession::new(SequenceOrdering::NaturalName);
        session
            .open_single_image(&first, &mut registry)
            .expect("opened image");
        let deleter = TestTrashDeleter {
            fail: true,
            ..TestTrashDeleter::default()
        };

        let error = session
            .trash_current_image(&mut registry, &deleter)
            .expect_err("trash failed");
        let snapshot = session.snapshot(&mut registry).expect("current snapshot");

        assert_eq!(
            error.frontend_safe_message(),
            "failed to move file to trash"
        );
        assert_eq!(snapshot.count, 2);
        assert_eq!(snapshot.current_position, Some(1));
    }
}
