use std::{cmp::Ordering, iter::Peekable, path::Path, str::Chars, time::SystemTime};

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
    // Unstable is safe here: compare_images breaks every tie on the full path,
    // so the order is total and never depends on the sort preserving input order.
    images.sort_unstable_by(|left, right| compare_images(left, right, ordering));
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

/// Walks both names in lockstep without allocating. The previous version built a
/// `Vec<char>` for each name on every comparison and a `String` for every
/// character compared, which dominated the cost of sorting a large folder.
fn natural_str_cmp(left: &str, right: &str) -> Ordering {
    let mut left_chars = left.chars().peekable();
    let mut right_chars = right.chars().peekable();

    loop {
        let (Some(left_char), Some(right_char)) =
            (left_chars.peek().copied(), right_chars.peek().copied())
        else {
            break;
        };

        if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
            let number_order = take_number(&mut left_chars).cmp(&take_number(&mut right_chars));
            if number_order != Ordering::Equal {
                return number_order;
            }
            continue;
        }

        // `char::to_lowercase` yields an iterator, so comparing the iterators
        // gives the same case-insensitive result the old `String` compare did.
        let char_order = left_char.to_lowercase().cmp(right_char.to_lowercase());
        if char_order != Ordering::Equal {
            return char_order;
        }

        left_chars.next();
        right_chars.next();
    }

    // Same final tie-break as before: the shorter name sorts first. Reached only
    // when the names are otherwise equal, so the extra counts are not hot.
    match (left_chars.peek(), right_chars.peek()) {
        (None, None) => left.chars().count().cmp(&right.chars().count()),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(_), Some(_)) => unreachable!("loop only breaks when a side is exhausted"),
    }
}

fn take_number(chars: &mut Peekable<Chars<'_>>) -> u128 {
    let mut value = 0_u128;

    while let Some(digit) = chars.peek().and_then(|digit| digit.to_digit(10)) {
        value = value.saturating_mul(10).saturating_add(u128::from(digit));
        chars.next();
    }

    value
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
    fn natural_name_ordering_handles_digit_runs_padding_and_unicode() {
        // Numeric runs compare by value, not digit-by-digit.
        assert_eq!(natural_str_cmp("image9.png", "image10.png"), Ordering::Less);
        assert_eq!(
            natural_str_cmp("image100.png", "image99.png"),
            Ordering::Greater
        );
        // Equal values, different padding: the shorter name wins, as before.
        assert_eq!(natural_str_cmp("img01.png", "img1.png"), Ordering::Greater);
        assert_eq!(natural_str_cmp("img1.png", "img1.png"), Ordering::Equal);
        // Case-insensitive, including outside ASCII.
        assert_eq!(natural_str_cmp("Photo.png", "photo.png"), Ordering::Equal);
        assert_eq!(natural_str_cmp("Ärger.png", "ärger.png"), Ordering::Equal);
        // A prefix sorts before the longer name that extends it.
        assert_eq!(natural_str_cmp("clip.mp4", "clip2.mp4"), Ordering::Less);
        // Numbers sort ahead of letters at the same position.
        assert_eq!(natural_str_cmp("2clip.mkv", "aclip.mkv"), Ordering::Less);
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
