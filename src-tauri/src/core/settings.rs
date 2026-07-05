use std::path::Path;

use serde::{Deserialize, Serialize};

use super::sequence_ordering::SequenceOrdering;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSettings {
    sequence_ordering: SequenceOrdering,
}

#[derive(Debug)]
pub enum SettingsError {
    FileSystem(std::io::Error),
    Parse(serde_json::Error),
}

impl From<std::io::Error> for SettingsError {
    fn from(error: std::io::Error) -> Self {
        Self::FileSystem(error)
    }
}

impl From<serde_json::Error> for SettingsError {
    fn from(error: serde_json::Error) -> Self {
        Self::Parse(error)
    }
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            sequence_ordering: SequenceOrdering::default(),
        }
    }
}

impl UserSettings {
    pub fn new(sequence_ordering: SequenceOrdering) -> Self {
        Self { sequence_ordering }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, SettingsError> {
        match std::fs::read_to_string(path.as_ref()) {
            Ok(contents) => Ok(serde_json::from_str(&contents)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(SettingsError::FileSystem(error)),
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SettingsError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let contents = serde_json::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    pub fn sequence_ordering(&self) -> SequenceOrdering {
        self.sequence_ordering
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sequence_ordering::SequenceOrdering;
    use tempfile::tempdir;

    #[test]
    fn missing_settings_file_uses_default_sequence_ordering() {
        let directory = tempdir().expect("temp dir");
        let settings_path = directory.path().join("settings.json");

        let settings = UserSettings::load(&settings_path).expect("user settings");

        assert_eq!(
            settings.sequence_ordering(),
            SequenceOrdering::NewestModifiedFirst
        );
    }

    #[test]
    fn sequence_ordering_is_saved_and_loaded() {
        let directory = tempdir().expect("temp dir");
        let settings_path = directory.path().join("settings.json");

        UserSettings::new(SequenceOrdering::NaturalName)
            .save(&settings_path)
            .expect("save settings");
        let loaded = UserSettings::load(&settings_path).expect("load settings");

        assert_eq!(loaded.sequence_ordering(), SequenceOrdering::NaturalName);
    }

    #[test]
    fn only_sequence_ordering_is_persisted() {
        let directory = tempdir().expect("temp dir");
        let settings_path = directory.path().join("settings.json");

        UserSettings::new(SequenceOrdering::SizeLargestFirst)
            .save(&settings_path)
            .expect("save settings");
        let saved_json = std::fs::read_to_string(&settings_path).expect("settings json");
        let saved_value: serde_json::Value = serde_json::from_str(&saved_json).expect("json value");
        let saved_object = saved_value.as_object().expect("settings object");

        assert_eq!(saved_object.len(), 1);
        assert_eq!(
            saved_object.get("sequence_ordering"),
            Some(&serde_json::Value::String("size_largest_first".to_string()))
        );
    }
}
