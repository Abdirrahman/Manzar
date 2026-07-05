use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tauri::{
    http::{header, Response, StatusCode},
    Manager,
};

use crate::core::{
    image_protocol::{
        image_id_from_protocol_path, serve_approved_image, ImageProtocolError,
        IMAGE_PROTOCOL_SCHEME,
    },
    image_registry::ApprovedImageRegistry,
    settings::UserSettings,
    viewer_session::ViewerSession,
};

mod commands;
pub mod core;

pub type SharedImageRegistry = Arc<Mutex<ApprovedImageRegistry>>;
pub type SharedViewerSession = Arc<Mutex<ViewerSession>>;

#[derive(Debug, Clone)]
pub struct SettingsFilePath(PathBuf);

impl SettingsFilePath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let image_registry = SharedImageRegistry::default();
    let viewer_session = SharedViewerSession::default();
    let protocol_registry = Arc::clone(&image_registry);
    let setup_viewer_session = Arc::clone(&viewer_session);

    tauri::Builder::default()
        .manage(image_registry)
        .manage(viewer_session)
        .setup(move |app| {
            let settings_path = settings_file_path(app);
            let settings = load_user_settings(&settings_path);

            if let Ok(mut session) = setup_viewer_session.lock() {
                *session = ViewerSession::new(settings.sequence_ordering());
            }

            app.manage(SettingsFilePath(settings_path));
            Ok(())
        })
        .register_uri_scheme_protocol(IMAGE_PROTOCOL_SCHEME, move |_ctx, request| {
            image_protocol_response(&protocol_registry, request.uri().path())
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_viewer_snapshot,
            commands::open_single_image,
            commands::open_image_selection,
            commands::open_folder,
            commands::navigate_next,
            commands::navigate_previous,
            commands::set_sequence_ordering,
            commands::rename_current_image,
            commands::trash_current_image
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn settings_file_path(app: &tauri::App) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("manzar"))
        .join("settings.json")
}

fn load_user_settings(settings_path: &Path) -> UserSettings {
    UserSettings::load(settings_path).unwrap_or_else(|error| {
        eprintln!("failed to load Manzar settings: {error:?}");
        UserSettings::default()
    })
}

fn image_protocol_response(registry: &SharedImageRegistry, path: &str) -> Response<Vec<u8>> {
    let Some(image_id) = image_id_from_protocol_path(path) else {
        return plain_text_response(StatusCode::BAD_REQUEST, "invalid image id");
    };

    let Ok(registry) = registry.lock() else {
        return plain_text_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "image registry unavailable",
        );
    };

    match serve_approved_image(&registry, &image_id) {
        Ok(image) => {
            let mime_type = image.mime_type();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime_type)
                .body(image.into_bytes())
                .expect("valid image protocol response")
        }
        Err(ImageProtocolError::UnknownImageId) => {
            plain_text_response(StatusCode::NOT_FOUND, "image not found")
        }
        Err(ImageProtocolError::UnsupportedImage) => {
            plain_text_response(StatusCode::FORBIDDEN, "unsupported image")
        }
        Err(ImageProtocolError::OversizedImage) => {
            plain_text_response(StatusCode::PAYLOAD_TOO_LARGE, "image too large")
        }
        Err(ImageProtocolError::FileSystem(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            plain_text_response(StatusCode::NOT_FOUND, "image not found")
        }
        Err(ImageProtocolError::FileSystem(_)) => {
            plain_text_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to read image")
        }
    }
}

fn plain_text_response(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(message.as_bytes().to_vec())
        .expect("valid plain-text protocol response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::metadata_preflight::MAX_SAFE_FILE_SIZE_BYTES;
    use tempfile::tempdir;

    #[test]
    fn oversized_protocol_image_returns_frontend_safe_payload_too_large_response() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("large.png");
        std::fs::File::create(&image)
            .expect("test image")
            .set_len(MAX_SAFE_FILE_SIZE_BYTES + 1)
            .expect("large sparse file");

        let registry = SharedImageRegistry::default();
        let id = {
            let mut registry = registry.lock().expect("image registry");
            registry
                .approve_path(&image)
                .expect("approved image")
                .id()
                .as_str()
                .to_string()
        };

        let response = image_protocol_response(&registry, &format!("/{id}"));

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(response.body(), b"image too large");
    }
}
