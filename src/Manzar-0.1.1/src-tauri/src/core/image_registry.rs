use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use uuid::Uuid;

use super::supported_image::{is_hidden_dotfile, is_supported_image};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageId(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedImage {
    id: ImageId,
    path: PathBuf,
}

#[derive(Debug, Default)]
pub struct ApprovedImageRegistry {
    paths_by_id: HashMap<ImageId, PathBuf>,
    ids_by_path: HashMap<PathBuf, ImageId>,
}

#[derive(Debug)]
pub enum ImageRegistryError {
    HiddenImage,
    UnsupportedImage,
    FileSystem(std::io::Error),
}

impl From<std::io::Error> for ImageRegistryError {
    fn from(error: std::io::Error) -> Self {
        Self::FileSystem(error)
    }
}

impl ImageId {
    pub fn from_opaque(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ApprovedImage {
    pub fn id(&self) -> &ImageId {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ApprovedImageRegistry {
    pub fn approve_path(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<ApprovedImage, ImageRegistryError> {
        let path = path.as_ref();
        if is_hidden_dotfile(path) {
            return Err(ImageRegistryError::HiddenImage);
        }
        if !is_supported_image(path) {
            return Err(ImageRegistryError::UnsupportedImage);
        }

        let path = path.canonicalize()?;
        let metadata = std::fs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(ImageRegistryError::UnsupportedImage);
        }

        if let Some(id) = self.ids_by_path.get(&path) {
            return Ok(ApprovedImage {
                id: id.clone(),
                path,
            });
        }

        let id = self.new_image_id();
        self.paths_by_id.insert(id.clone(), path.clone());
        self.ids_by_path.insert(path.clone(), id.clone());
        Ok(ApprovedImage { id, path })
    }

    pub fn path_for(&self, id: &ImageId) -> Option<&Path> {
        self.paths_by_id.get(id).map(PathBuf::as_path)
    }

    pub fn clear(&mut self) {
        self.paths_by_id.clear();
        self.ids_by_path.clear();
    }

    fn new_image_id(&self) -> ImageId {
        loop {
            let id = ImageId(Uuid::new_v4().to_string());
            if !self.paths_by_id.contains_key(&id) {
                return id;
            }
        }
    }

    pub fn revoke_path(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        if let Some(id) = self.ids_by_path.remove(&path) {
            self.paths_by_id.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn approved_supported_image_gets_opaque_id_resolvable_only_by_registry() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("private-name.png");
        std::fs::write(&image, b"image bytes").expect("image file");
        let canonical_image = image.canonicalize().expect("canonical image");

        let mut registry = ApprovedImageRegistry::default();
        let approved = registry.approve_path(&image).expect("approved image");

        assert!(!approved.id().as_str().contains("private-name"));
        assert!(!approved
            .id()
            .as_str()
            .contains(directory.path().to_string_lossy().as_ref()));
        assert_eq!(
            registry.path_for(approved.id()),
            Some(canonical_image.as_path())
        );
    }

    #[test]
    fn approved_image_ids_are_not_predictable_sequence_values() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("first.png");
        let second = directory.path().join("second.png");
        std::fs::write(&first, b"first image").expect("first image");
        std::fs::write(&second, b"second image").expect("second image");

        let mut registry = ApprovedImageRegistry::default();
        let first = registry.approve_path(&first).expect("first approved image");
        let second = registry
            .approve_path(&second)
            .expect("second approved image");

        assert_ne!(first.id().as_str(), "image-1");
        assert_ne!(second.id().as_str(), "image-2");
        assert_ne!(first.id(), second.id());
    }

    #[test]
    fn hidden_and_unsupported_images_are_not_approved() {
        let directory = tempdir().expect("temp dir");
        let hidden = directory.path().join(".hidden.png");
        let unsupported = directory.path().join("notes.txt");
        std::fs::write(&hidden, b"hidden image").expect("hidden image");
        std::fs::write(&unsupported, b"notes").expect("unsupported file");

        let mut registry = ApprovedImageRegistry::default();

        assert!(matches!(
            registry.approve_path(&hidden),
            Err(ImageRegistryError::HiddenImage)
        ));
        assert!(matches!(
            registry.approve_path(&unsupported),
            Err(ImageRegistryError::UnsupportedImage)
        ));
    }

    #[test]
    fn unknown_opaque_id_does_not_resolve_to_a_path() {
        let registry = ApprovedImageRegistry::default();

        assert_eq!(registry.path_for(&ImageId("image-404".to_string())), None);
    }
}
