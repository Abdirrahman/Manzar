use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::{
    core::{
        file_actions::DesktopTrashDeleter,
        sequence_ordering::SequenceOrdering,
        settings::{SettingsError, UserSettings},
        viewer_session::{ViewerSessionError, ViewerSnapshot},
    },
    SettingsFilePath, SharedImageRegistry, SharedViewerSession,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandError {
    pub message: &'static str,
}

#[tauri::command]
pub fn get_viewer_snapshot(
    session: State<'_, SharedViewerSession>,
    registry: State<'_, SharedImageRegistry>,
) -> Result<ViewerSnapshot, CommandError> {
    with_session_and_registry(&session, &registry, |session, registry| {
        session.snapshot(registry)
    })
}

#[tauri::command]
pub fn open_single_image(
    path: String,
    session: State<'_, SharedViewerSession>,
    registry: State<'_, SharedImageRegistry>,
) -> Result<ViewerSnapshot, CommandError> {
    with_session_and_registry(&session, &registry, |session, registry| {
        session.open_single_image(PathBuf::from(path), registry)
    })
}

#[tauri::command]
pub fn open_image_selection(
    paths: Vec<String>,
    session: State<'_, SharedViewerSession>,
    registry: State<'_, SharedImageRegistry>,
) -> Result<ViewerSnapshot, CommandError> {
    let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    with_session_and_registry(&session, &registry, |session, registry| {
        session.open_image_selection(&paths, registry)
    })
}

#[tauri::command]
pub fn open_folder(
    path: String,
    session: State<'_, SharedViewerSession>,
    registry: State<'_, SharedImageRegistry>,
) -> Result<ViewerSnapshot, CommandError> {
    with_session_and_registry(&session, &registry, |session, registry| {
        session.open_folder(PathBuf::from(path), registry)
    })
}

#[tauri::command]
pub fn navigate_next(
    session: State<'_, SharedViewerSession>,
    registry: State<'_, SharedImageRegistry>,
) -> Result<ViewerSnapshot, CommandError> {
    with_session_and_registry(&session, &registry, |session, registry| {
        session.next(registry)
    })
}

#[tauri::command]
pub fn navigate_previous(
    session: State<'_, SharedViewerSession>,
    registry: State<'_, SharedImageRegistry>,
) -> Result<ViewerSnapshot, CommandError> {
    with_session_and_registry(&session, &registry, |session, registry| {
        session.previous(registry)
    })
}

#[tauri::command]
pub fn set_sequence_ordering(
    ordering: SequenceOrdering,
    session: State<'_, SharedViewerSession>,
    registry: State<'_, SharedImageRegistry>,
    settings_path: State<'_, SettingsFilePath>,
) -> Result<ViewerSnapshot, CommandError> {
    persist_sequence_ordering(ordering, settings_path.as_path()).map_err(CommandError::from)?;

    with_session_and_registry(&session, &registry, |session, registry| {
        session.set_sequence_ordering(ordering, registry)
    })
}

#[tauri::command]
pub fn rename_current_image(
    new_stem: String,
    session: State<'_, SharedViewerSession>,
    registry: State<'_, SharedImageRegistry>,
) -> Result<ViewerSnapshot, CommandError> {
    with_session_and_registry(&session, &registry, |session, registry| {
        session.rename_current_image(&new_stem, registry)
    })
}

#[tauri::command]
pub fn trash_current_image(
    session: State<'_, SharedViewerSession>,
    registry: State<'_, SharedImageRegistry>,
) -> Result<ViewerSnapshot, CommandError> {
    let deleter = DesktopTrashDeleter;
    with_session_and_registry(&session, &registry, |session, registry| {
        session.trash_current_image(registry, &deleter)
    })
}

fn persist_sequence_ordering(
    ordering: SequenceOrdering,
    settings_path: &Path,
) -> Result<(), SettingsError> {
    UserSettings::new(ordering).save(settings_path)
}

fn with_session_and_registry(
    session: &SharedViewerSession,
    registry: &SharedImageRegistry,
    operation: impl FnOnce(
        &mut crate::core::viewer_session::ViewerSession,
        &mut crate::core::image_registry::ApprovedImageRegistry,
    ) -> Result<ViewerSnapshot, ViewerSessionError>,
) -> Result<ViewerSnapshot, CommandError> {
    let mut session = session.lock().map_err(|_| CommandError {
        message: "viewer session unavailable",
    })?;
    let mut registry = registry.lock().map_err(|_| CommandError {
        message: "image registry unavailable",
    })?;

    operation(&mut session, &mut registry).map_err(CommandError::from)
}

impl From<ViewerSessionError> for CommandError {
    fn from(error: ViewerSessionError) -> Self {
        Self {
            message: error.frontend_safe_message(),
        }
    }
}

impl From<SettingsError> for CommandError {
    fn from(error: SettingsError) -> Self {
        let message = match error {
            SettingsError::FileSystem(_) => "failed to save settings",
            SettingsError::Parse(_) => "failed to serialize settings",
        };

        Self { message }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sequence_ordering_persistence_writes_only_sequence_ordering() {
        let directory = tempdir().expect("temp dir");
        let settings_path = directory.path().join("settings.json");

        persist_sequence_ordering(SequenceOrdering::SizeLargestFirst, &settings_path)
            .expect("persist ordering");

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
