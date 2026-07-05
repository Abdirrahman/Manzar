use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::{
    sequence_ordering::{sort_images, OrderedImage, SequenceOrdering},
    supported_image::{is_hidden_dotfile, is_supported_image},
};

#[derive(Debug, Clone)]
pub struct ImageSequence {
    items: Vec<ImageSequenceItem>,
    current_index: usize,
}

#[derive(Debug, Clone)]
pub struct ImageSequenceItem {
    path: PathBuf,
    modified: SystemTime,
    size_bytes: u64,
}

#[derive(Debug)]
pub enum ImageSequenceError {
    HiddenImage,
    NoParentFolder,
    NoSupportedImages,
    UnsupportedImage,
    FileSystem(std::io::Error),
}

impl From<std::io::Error> for ImageSequenceError {
    fn from(error: std::io::Error) -> Self {
        Self::FileSystem(error)
    }
}

impl ImageSequence {
    pub fn from_single_image(
        path: impl AsRef<Path>,
        ordering: SequenceOrdering,
    ) -> Result<Self, ImageSequenceError> {
        let opened_path = path.as_ref();
        if is_hidden_dotfile(opened_path) {
            return Err(ImageSequenceError::HiddenImage);
        }
        if !is_supported_image(opened_path) {
            return Err(ImageSequenceError::UnsupportedImage);
        }

        let opened_path = opened_path.canonicalize()?;
        let parent = opened_path
            .parent()
            .ok_or(ImageSequenceError::NoParentFolder)?;
        let mut items = supported_images_in_folder(parent)?;
        sort_images(&mut items, ordering);

        let current_index = items
            .iter()
            .position(|item| item.path == opened_path)
            .ok_or(ImageSequenceError::UnsupportedImage)?;

        Ok(Self {
            items,
            current_index,
        })
    }

    pub fn from_image_selection(
        paths: impl IntoIterator<Item = impl AsRef<Path>>,
        ordering: SequenceOrdering,
    ) -> Result<Self, ImageSequenceError> {
        let mut seen = HashSet::new();
        let mut items = Vec::new();

        for path in paths {
            let path = path.as_ref();
            if is_hidden_dotfile(path) || !is_supported_image(path) {
                continue;
            }

            let item = match ImageSequenceItem::from_path(path) {
                Ok(item) => item,
                Err(ImageSequenceError::FileSystem(error))
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    continue
                }
                Err(error) => return Err(error),
            };

            if seen.insert(item.path.clone()) {
                items.push(item);
            }
        }

        Self::from_items(items, ordering)
    }

    pub fn from_folder(
        folder: impl AsRef<Path>,
        ordering: SequenceOrdering,
    ) -> Result<Self, ImageSequenceError> {
        let items = supported_images_in_folder(folder.as_ref())?;
        Self::from_items(items, ordering)
    }

    pub fn current_path(&self) -> Option<&Path> {
        self.items
            .get(self.current_index)
            .map(ImageSequenceItem::path)
    }

    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.items.iter().map(ImageSequenceItem::path)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn current_position(&self) -> Option<usize> {
        if self.items.is_empty() {
            None
        } else {
            Some(self.current_index + 1)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn next(&mut self) -> Option<&Path> {
        if self.items.is_empty() {
            return None;
        }

        self.current_index = (self.current_index + 1) % self.items.len();
        self.current_path()
    }

    pub fn previous(&mut self) -> Option<&Path> {
        if self.items.is_empty() {
            return None;
        }

        self.current_index = if self.current_index == 0 {
            self.items.len() - 1
        } else {
            self.current_index - 1
        };
        self.current_path()
    }

    pub fn reorder(&mut self, ordering: SequenceOrdering) {
        let current_path = self.current_path().map(Path::to_path_buf);
        sort_images(&mut self.items, ordering);

        if let Some(current_path) = current_path {
            if let Some(current_index) =
                self.items.iter().position(|item| item.path == current_path)
            {
                self.current_index = current_index;
            }
        }
    }

    pub fn replace_current_path(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<(), ImageSequenceError> {
        if self.items.is_empty() {
            return Err(ImageSequenceError::NoSupportedImages);
        }

        self.items[self.current_index] = ImageSequenceItem::from_path(path)?;
        Ok(())
    }

    pub fn remove_current(&mut self) {
        if self.items.is_empty() {
            return;
        }

        self.items.remove(self.current_index);

        if self.items.is_empty() {
            self.current_index = 0;
        } else if self.current_index >= self.items.len() {
            self.current_index = 0;
        }
    }

    fn from_items(
        mut items: Vec<ImageSequenceItem>,
        ordering: SequenceOrdering,
    ) -> Result<Self, ImageSequenceError> {
        if items.is_empty() {
            return Err(ImageSequenceError::NoSupportedImages);
        }

        sort_images(&mut items, ordering);
        Ok(Self {
            items,
            current_index: 0,
        })
    }
}

impl ImageSequenceItem {
    fn from_path(path: impl AsRef<Path>) -> Result<Self, ImageSequenceError> {
        let path = path.as_ref().canonicalize()?;
        let metadata = std::fs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(ImageSequenceError::UnsupportedImage);
        }

        Ok(Self {
            path,
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            size_bytes: metadata.len(),
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl OrderedImage for ImageSequenceItem {
    fn path(&self) -> &Path {
        &self.path
    }

    fn modified(&self) -> SystemTime {
        self.modified
    }

    fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
}

fn supported_images_in_folder(folder: &Path) -> Result<Vec<ImageSequenceItem>, ImageSequenceError> {
    let mut items = Vec::new();

    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        if is_hidden_dotfile(&path) || !is_supported_image(&path) {
            continue;
        }

        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }

        items.push(ImageSequenceItem {
            path: path.canonicalize()?,
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            size_bytes: metadata.len(),
        });
    }

    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &[u8]) {
        std::fs::write(path, contents).expect("test file");
    }

    fn canonical(path: &Path) -> PathBuf {
        path.canonicalize().expect("canonical path")
    }

    fn sequence_paths(sequence: &ImageSequence) -> Vec<PathBuf> {
        sequence.paths().map(Path::to_path_buf).collect()
    }

    #[test]
    fn opening_one_supported_image_discovers_non_hidden_supported_sibling_images() {
        let directory = tempdir().expect("temp dir");
        let opened = directory.path().join("opened.png");
        let sibling = directory.path().join("sibling.JPG");
        let hidden = directory.path().join(".hidden.png");
        let unsupported = directory.path().join("notes.txt");

        write_file(&opened, b"opened");
        write_file(&sibling, b"sibling image file");
        write_file(&hidden, b"hidden image file");
        write_file(&unsupported, b"unsupported file");

        let sequence = ImageSequence::from_single_image(&opened, SequenceOrdering::NaturalName)
            .expect("Image Sequence");

        let opened = canonical(&opened);
        let sibling = canonical(&sibling);
        assert_eq!(sequence_paths(&sequence), vec![opened.clone(), sibling]);
        assert_eq!(sequence.current_path(), Some(opened.as_path()));
    }

    #[test]
    fn explicit_image_selection_uses_only_selected_supported_images() {
        let directory = tempdir().expect("temp dir");
        let selected_first = directory.path().join("image1.png");
        let selected_second = directory.path().join("image2.gif");
        let unselected_sibling = directory.path().join("image3.jpg");
        let unsupported = directory.path().join("notes.txt");
        let hidden = directory.path().join(".hidden.png");

        write_file(&selected_first, b"selected first");
        write_file(&selected_second, b"selected second");
        write_file(&unselected_sibling, b"unselected sibling");
        write_file(&unsupported, b"unsupported");
        write_file(&hidden, b"hidden");

        let sequence = ImageSequence::from_image_selection(
            [&selected_second, &unsupported, &selected_first, &hidden],
            SequenceOrdering::NaturalName,
        )
        .expect("Image Sequence");

        assert_eq!(
            sequence_paths(&sequence),
            vec![canonical(&selected_first), canonical(&selected_second)]
        );
    }

    #[test]
    fn opening_a_folder_uses_supported_non_hidden_images_in_that_folder() {
        let directory = tempdir().expect("temp dir");
        let image1 = directory.path().join("image1.png");
        let image2 = directory.path().join("image2.webp");
        let hidden = directory.path().join(".hidden.jpg");
        let unsupported = directory.path().join("notes.txt");

        write_file(&image2, b"image2");
        write_file(&image1, b"image1");
        write_file(&hidden, b"hidden");
        write_file(&unsupported, b"unsupported");

        let sequence = ImageSequence::from_folder(directory.path(), SequenceOrdering::NaturalName)
            .expect("Image Sequence");

        assert_eq!(
            sequence_paths(&sequence),
            vec![canonical(&image1), canonical(&image2)]
        );
    }

    #[test]
    fn sequence_navigation_wraps_next_and_previous() {
        let directory = tempdir().expect("temp dir");
        let image1 = directory.path().join("image1.png");
        let image2 = directory.path().join("image2.png");

        write_file(&image1, b"image1");
        write_file(&image2, b"image2");

        let mut sequence =
            ImageSequence::from_folder(directory.path(), SequenceOrdering::NaturalName)
                .expect("Image Sequence");
        let image1 = canonical(&image1);
        let image2 = canonical(&image2);

        assert_eq!(sequence.current_path(), Some(image1.as_path()));
        assert_eq!(sequence.next(), Some(image2.as_path()));
        assert_eq!(sequence.next(), Some(image1.as_path()));
        assert_eq!(sequence.previous(), Some(image2.as_path()));
    }

    #[test]
    fn reordering_preserves_the_current_image() {
        let directory = tempdir().expect("temp dir");
        let large_name_first = directory.path().join("a-large.png");
        let small_name_second = directory.path().join("z-small.png");

        write_file(&large_name_first, b"large image contents");
        write_file(&small_name_second, b"s");

        let mut sequence =
            ImageSequence::from_folder(directory.path(), SequenceOrdering::NaturalName)
                .expect("Image Sequence");
        let large_name_first = canonical(&large_name_first);
        let small_name_second = canonical(&small_name_second);

        assert_eq!(sequence.current_path(), Some(large_name_first.as_path()));

        sequence.reorder(SequenceOrdering::SizeSmallestFirst);

        assert_eq!(sequence.current_path(), Some(large_name_first.as_path()));
        assert_eq!(sequence.current_position(), Some(2));
        assert_eq!(
            sequence_paths(&sequence),
            vec![small_name_second, large_name_first]
        );
    }

    #[test]
    fn replacing_current_path_updates_the_current_item() {
        let directory = tempdir().expect("temp dir");
        let old = directory.path().join("old.png");
        let sibling = directory.path().join("sibling.png");
        let renamed = directory.path().join("renamed.png");
        write_file(&old, b"old");
        write_file(&sibling, b"sibling");

        let mut sequence = ImageSequence::from_single_image(&old, SequenceOrdering::NaturalName)
            .expect("Image Sequence");
        std::fs::rename(&old, &renamed).expect("rename image");

        sequence
            .replace_current_path(&renamed)
            .expect("replace current path");

        assert_eq!(sequence.current_path(), Some(canonical(&renamed).as_path()));
        assert_eq!(
            sequence_paths(&sequence),
            vec![canonical(&renamed), canonical(&sibling)]
        );
    }

    #[test]
    fn removing_current_selects_the_next_image_and_wraps_from_the_end() {
        let directory = tempdir().expect("temp dir");
        let first = directory.path().join("image1.png");
        let second = directory.path().join("image2.png");
        write_file(&first, b"first");
        write_file(&second, b"second");

        let mut sequence =
            ImageSequence::from_folder(directory.path(), SequenceOrdering::NaturalName)
                .expect("Image Sequence");

        sequence.remove_current();
        assert_eq!(sequence.current_path(), Some(canonical(&second).as_path()));
        assert_eq!(sequence.current_position(), Some(1));

        sequence.remove_current();
        assert!(sequence.is_empty());
        assert_eq!(sequence.current_position(), None);
    }
}
