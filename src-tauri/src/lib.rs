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
        image_id_from_protocol_path, serve_approved_media, ImageProtocolError,
        IMAGE_PROTOCOL_SCHEME,
    },
    image_registry::ApprovedImageRegistry,
    media_server,
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
    let setup_image_registry = Arc::clone(&image_registry);

    tauri::Builder::default()
        .manage(image_registry)
        .manage(viewer_session)
        .setup(move |app| {
            // Started before any snapshot so video descriptors can carry a
            // playable URL. A failure is non-fatal: images still work.
            if let Err(error) = media_server::start(Arc::clone(&setup_image_registry)) {
                eprintln!("failed to start Manzar media server: {error}");
            }

            let settings_path = settings_file_path(app);
            let settings = load_user_settings(&settings_path);

            if let Ok(mut session) = setup_viewer_session.lock() {
                *session = ViewerSession::new(settings.sequence_ordering());
            }

            app.manage(SettingsFilePath(settings_path));

            if let Err(error) = open_startup_file_arguments(
                std::env::args_os()
                    .skip(1)
                    .filter(|argument| !argument.to_string_lossy().starts_with('-')),
                &setup_viewer_session,
                &setup_image_registry,
            ) {
                eprintln!("failed to open startup image argument: {}", error.message);
            }

            Ok(())
        })
        .register_uri_scheme_protocol(IMAGE_PROTOCOL_SCHEME, move |_ctx, request| {
            let range = request
                .headers()
                .get(header::RANGE)
                .and_then(|value| value.to_str().ok());
            media_protocol_response(&protocol_registry, request.uri().path(), range)
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

fn open_startup_file_arguments(
    paths: impl IntoIterator<Item = impl Into<PathBuf>>,
    session: &SharedViewerSession,
    registry: &SharedImageRegistry,
) -> Result<Option<crate::core::viewer_session::ViewerSnapshot>, commands::CommandError> {
    let paths = paths.into_iter().map(Into::into).collect::<Vec<_>>();

    if paths.is_empty() {
        return Ok(None);
    }

    let mut session = session.lock().map_err(|_| commands::CommandError {
        message: "viewer session unavailable",
    })?;
    let mut registry = registry.lock().map_err(|_| commands::CommandError {
        message: "image registry unavailable",
    })?;

    let opened = if paths.len() == 1 {
        session.open_single_image(&paths[0], &mut registry)
    } else {
        session.open_image_selection(&paths, &mut registry)
    };

    opened.map(Some).map_err(commands::CommandError::from)
}

fn media_protocol_response(
    registry: &SharedImageRegistry,
    path: &str,
    range: Option<&str>,
) -> Response<Vec<u8>> {
    let Some(image_id) = image_id_from_protocol_path(path) else {
        return plain_text_response(StatusCode::BAD_REQUEST, "invalid image id");
    };

    let Ok(registry) = registry.lock() else {
        return plain_text_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "image registry unavailable",
        );
    };

    match serve_approved_media(&registry, &image_id, range) {
        Ok(media) => {
            let status = if media.is_partial() {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            };
            let mut response = Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, media.mime_type())
                .header(header::ACCEPT_RANGES, "bytes");

            if let Some(content_range) = media.content_range() {
                response = response.header(header::CONTENT_RANGE, content_range);
            }

            response
                .body(media.into_bytes())
                .expect("valid media protocol response")
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
        Err(ImageProtocolError::RangeNotSatisfiable { total_bytes }) => Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(header::CONTENT_RANGE, format!("bytes */{total_bytes}"))
            .body(b"requested range not satisfiable".to_vec())
            .expect("valid range error response"),
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
    fn single_startup_image_argument_opens_as_single_image_session() {
        let directory = tempdir().expect("temp dir");
        let opened = directory.path().join("a-opened.png");
        let sibling = directory.path().join("z-sibling.jpg");
        // The sibling is written first so the opened image is the newer of the
        // two. The default ordering is newest-modified-first, so the opened
        // image leads whether or not both writes land on the same mtime tick:
        // by modification time if they differ, by name if they tie.
        std::fs::write(&sibling, b"sibling image").expect("sibling image");
        std::fs::write(&opened, b"opened image").expect("opened image");

        let session = SharedViewerSession::default();
        let registry = SharedImageRegistry::default();

        let snapshot = open_startup_file_arguments([opened], &session, &registry)
            .expect("startup arguments")
            .expect("opened snapshot");

        assert_eq!(snapshot.count, 2);
        assert_eq!(snapshot.current_position, Some(1));
    }

    #[test]
    fn multiple_startup_image_arguments_open_as_explicit_selection() {
        let directory = tempdir().expect("temp dir");
        let selected_first = directory.path().join("a-selected.png");
        let selected_second = directory.path().join("b-selected.gif");
        let unselected_sibling = directory.path().join("c-unselected.jpg");
        std::fs::write(&selected_first, b"selected first").expect("selected first");
        std::fs::write(&selected_second, b"selected second").expect("selected second");
        std::fs::write(&unselected_sibling, b"unselected sibling").expect("unselected sibling");

        let session = SharedViewerSession::default();
        let registry = SharedImageRegistry::default();

        let snapshot =
            open_startup_file_arguments([selected_second, selected_first], &session, &registry)
                .expect("startup arguments")
                .expect("opened snapshot");

        assert_eq!(snapshot.count, 2);
    }

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

        let response = media_protocol_response(&registry, &format!("/{id}"), None);

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(response.body(), b"image too large");
    }

    #[test]
    fn video_range_request_returns_partial_content_with_a_content_range_header() {
        let directory = tempdir().expect("temp dir");
        let video = directory.path().join("clip.mp4");
        std::fs::write(&video, b"0123456789").expect("test video");

        let registry = SharedImageRegistry::default();
        let id = {
            let mut registry = registry.lock().expect("image registry");
            registry
                .approve_path(&video)
                .expect("approved video")
                .id()
                .as_str()
                .to_string()
        };

        let response = media_protocol_response(&registry, &format!("/{id}"), Some("bytes=2-5"));

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body(), b"2345");
        assert_eq!(
            response.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes 2-5/10"
        );
        assert_eq!(
            response.headers().get(header::ACCEPT_RANGES).unwrap(),
            "bytes"
        );
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "video/mp4"
        );
    }
}
