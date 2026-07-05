use std::{cmp::Ordering, path::Path, time::SystemTime};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceOrdering {
    NewestModifiedFirst,
    NaturalName,
    SizeLargestFirst,
    SizeSmallestFirst,
}

impl Default for SequenceOrdering {
    fn default() -> Self {
        Self::NewestModifiedFirst
    }
}

pub trait OrderedImage {
    fn path(&self) -> &Path;
    fn modified(&self) -> SystemTime;
    fn size_bytes(&self) -> u64;
}

pub fn sort_images<T: OrderedImage>(images: &mut [T], ordering: SequenceOrdering) {
    images.sort_by(|left, right| compare_images(left, right, ordering));
}

fn compare_images<T: OrderedImage>(left: &T, right: &T, ordering: SequenceOrdering) -> Ordering {
    let primary = match ordering {
        SequenceOrdering::NewestModifiedFirst => right.modified().cmp(&left.modified()),
        SequenceOrdering::NaturalName => natural_path_cmp(left.path(), right.path()),
        SequenceOrdering::SizeLargestFirst => right.size_bytes().cmp(&left.size_bytes()),
        SequenceOrdering::SizeSmallestFirst => left.size_bytes().cmp(&right.size_bytes()),
    };

    primary
        .then_with(|| natural_path_cmp(left.path(), right.path()))
        .then_with(|| stable_path_cmp(left.path(), right.path()))
}

fn natural_path_cmp(left: &Path, right: &Path) -> Ordering {
    let left_name = left
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let right_name = right
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    natural_str_cmp(left_name, right_name)
}

fn stable_path_cmp(left: &Path, right: &Path) -> Ordering {
    left.to_string_lossy().cmp(&right.to_string_lossy())
}

fn natural_str_cmp(left: &str, right: &str) -> Ordering {
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut left_index = 0;
    let mut right_index = 0;

    while left_index < left_chars.len() && right_index < right_chars.len() {
        if left_chars[left_index].is_ascii_digit() && right_chars[right_index].is_ascii_digit() {
            let (left_number, next_left) = take_number(&left_chars, left_index);
            let (right_number, next_right) = take_number(&right_chars, right_index);
            let number_order = left_number.cmp(&right_number);
            if number_order != Ordering::Equal {
                return number_order;
            }
            left_index = next_left;
            right_index = next_right;
            continue;
        }

        let char_order = left_chars[left_index]
            .to_lowercase()
            .to_string()
            .cmp(&right_chars[right_index].to_lowercase().to_string());
        if char_order != Ordering::Equal {
            return char_order;
        }

        left_index += 1;
        right_index += 1;
    }

    left_chars.len().cmp(&right_chars.len())
}

fn take_number(chars: &[char], start: usize) -> (u128, usize) {
    let mut value = 0_u128;
    let mut index = start;

    while index < chars.len() && chars[index].is_ascii_digit() {
        value = value
            .saturating_mul(10)
            .saturating_add(chars[index].to_digit(10).unwrap_or_default() as u128);
        index += 1;
    }

    (value, index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, time::Duration};

    #[derive(Debug)]
    struct TestImage {
        path: PathBuf,
        modified: SystemTime,
        size_bytes: u64,
    }

    impl TestImage {
        fn new(name: &str, modified_seconds: u64, size_bytes: u64) -> Self {
            Self {
                path: PathBuf::from(name),
                modified: SystemTime::UNIX_EPOCH + Duration::from_secs(modified_seconds),
                size_bytes,
            }
        }
    }

    impl OrderedImage for TestImage {
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

    fn names(images: &[TestImage]) -> Vec<String> {
        images
            .iter()
            .map(|image| {
                image
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn natural_name_ordering_is_case_insensitive_and_numeric() {
        let mut images = vec![
            TestImage::new("Image10.png", 1, 1),
            TestImage::new("image2.png", 1, 1),
            TestImage::new("image1.png", 1, 1),
        ];

        sort_images(&mut images, SequenceOrdering::NaturalName);

        assert_eq!(
            names(&images),
            vec!["image1.png", "image2.png", "Image10.png"]
        );
    }

    #[test]
    fn newest_modified_first_ordering_prefers_recent_images() {
        let mut images = vec![
            TestImage::new("older.png", 10, 1),
            TestImage::new("newer.png", 30, 1),
            TestImage::new("middle.png", 20, 1),
        ];

        sort_images(&mut images, SequenceOrdering::NewestModifiedFirst);

        assert_eq!(names(&images), vec!["newer.png", "middle.png", "older.png"]);
    }

    #[test]
    fn size_ordering_supports_largest_and_smallest_first() {
        let images = vec![
            TestImage::new("small.png", 1, 10),
            TestImage::new("large.png", 1, 30),
            TestImage::new("middle.png", 1, 20),
        ];

        let mut largest_first = images;
        sort_images(&mut largest_first, SequenceOrdering::SizeLargestFirst);
        assert_eq!(
            names(&largest_first),
            vec!["large.png", "middle.png", "small.png"]
        );

        let mut smallest_first = largest_first;
        sort_images(&mut smallest_first, SequenceOrdering::SizeSmallestFirst);
        assert_eq!(
            names(&smallest_first),
            vec!["small.png", "middle.png", "large.png"]
        );
    }

    #[test]
    fn ordering_ties_fall_back_to_natural_name_ordering() {
        let mut images = vec![
            TestImage::new("image10.png", 1, 10),
            TestImage::new("image2.png", 1, 10),
            TestImage::new("image1.png", 1, 10),
        ];

        sort_images(&mut images, SequenceOrdering::SizeLargestFirst);

        assert_eq!(
            names(&images),
            vec!["image1.png", "image2.png", "image10.png"]
        );
    }
}
