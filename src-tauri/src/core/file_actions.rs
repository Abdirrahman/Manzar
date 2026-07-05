use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum FileActionError {
    EmptyStem,
    HiddenTarget,
    PathSeparator,
    NoParentFolder,
    MissingExtension,
    TargetAlreadyExists,
    TrashDeletionFailed,
    FileSystem(std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrashDeleteError;

pub trait TrashDeleter {
    fn trash(&self, path: &Path) -> Result<(), TrashDeleteError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DesktopTrashDeleter;

impl TrashDeleter for DesktopTrashDeleter {
    fn trash(&self, path: &Path) -> Result<(), TrashDeleteError> {
        trash::delete(path).map_err(|_| TrashDeleteError)
    }
}

impl From<std::io::Error> for FileActionError {
    fn from(error: std::io::Error) -> Self {
        Self::FileSystem(error)
    }
}

pub fn rename_current_image(
    current_path: impl AsRef<Path>,
    new_stem: &str,
) -> Result<PathBuf, FileActionError> {
    let current_path = current_path.as_ref().canonicalize()?;
    let parent = current_path
        .parent()
        .ok_or(FileActionError::NoParentFolder)?;
    let extension = current_path
        .extension()
        .ok_or(FileActionError::MissingExtension)?;
    let new_stem = validate_new_stem(new_stem)?;

    let mut file_name = OsString::from(new_stem);
    file_name.push(".");
    file_name.push(extension);
    let target_path = parent.join(file_name);

    if target_path == current_path {
        return Ok(current_path);
    }

    if target_path.exists() {
        return Err(FileActionError::TargetAlreadyExists);
    }

    std::fs::rename(&current_path, &target_path)?;
    Ok(target_path.canonicalize()?)
}

pub fn trash_current_image(
    current_path: impl AsRef<Path>,
    deleter: &impl TrashDeleter,
) -> Result<(), FileActionError> {
    let current_path = current_path.as_ref().canonicalize()?;
    deleter
        .trash(&current_path)
        .map_err(|_| FileActionError::TrashDeletionFailed)
}

fn validate_new_stem(new_stem: &str) -> Result<&str, FileActionError> {
    let new_stem = new_stem.trim();

    if new_stem.is_empty() {
        return Err(FileActionError::EmptyStem);
    }

    if new_stem.starts_with('.') {
        return Err(FileActionError::HiddenTarget);
    }

    if new_stem.contains('/') || new_stem.contains('\\') {
        return Err(FileActionError::PathSeparator);
    }

    Ok(new_stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[derive(Debug, Default)]
    struct RecordingTrashDeleter {
        trashed: std::cell::RefCell<Vec<PathBuf>>,
        fail: bool,
    }

    impl TrashDeleter for RecordingTrashDeleter {
        fn trash(&self, path: &Path) -> Result<(), TrashDeleteError> {
            if self.fail {
                return Err(TrashDeleteError);
            }

            self.trashed.borrow_mut().push(path.to_path_buf());
            Ok(())
        }
    }

    #[test]
    fn rename_is_stem_only_and_preserves_extension_in_same_folder() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("old-name.JPG");
        std::fs::write(&image, b"image").expect("image file");

        let renamed = rename_current_image(&image, "new-name").expect("renamed image");

        assert_eq!(
            renamed,
            directory
                .path()
                .join("new-name.JPG")
                .canonicalize()
                .expect("canonical renamed")
        );
        assert!(!image.exists());
        assert_eq!(std::fs::read(renamed).expect("renamed bytes"), b"image");
    }

    #[test]
    fn rename_rejects_empty_hidden_separator_and_collision_targets() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("current.png");
        let collision = directory.path().join("existing.png");
        std::fs::write(&image, b"image").expect("image file");
        std::fs::write(&collision, b"collision").expect("collision file");

        assert!(matches!(
            rename_current_image(&image, "   "),
            Err(FileActionError::EmptyStem)
        ));
        assert!(matches!(
            rename_current_image(&image, ".hidden"),
            Err(FileActionError::HiddenTarget)
        ));
        assert!(matches!(
            rename_current_image(&image, "other/folder"),
            Err(FileActionError::PathSeparator)
        ));
        assert!(matches!(
            rename_current_image(&image, "other\\folder"),
            Err(FileActionError::PathSeparator)
        ));
        assert!(matches!(
            rename_current_image(&image, "existing"),
            Err(FileActionError::TargetAlreadyExists)
        ));
    }

    #[test]
    fn trash_deletion_uses_the_current_image_path() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("current.png");
        std::fs::write(&image, b"image").expect("image file");
        let canonical = image.canonicalize().expect("canonical image");
        let deleter = RecordingTrashDeleter::default();

        trash_current_image(&image, &deleter).expect("trash deletion");

        assert_eq!(deleter.trashed.borrow().as_slice(), &[canonical]);
    }

    #[test]
    fn trash_deletion_reports_deleter_failure() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("current.png");
        std::fs::write(&image, b"image").expect("image file");
        let deleter = RecordingTrashDeleter {
            fail: true,
            ..RecordingTrashDeleter::default()
        };

        assert!(matches!(
            trash_current_image(&image, &deleter),
            Err(FileActionError::TrashDeletionFailed)
        ));
    }
}
