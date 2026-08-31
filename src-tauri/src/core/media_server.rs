//! A loopback HTTP server used only for video playback.
//!
//! WebKitGTK cannot play `<video>` from an app-registered custom URI scheme:
//! the media pipeline resolves the URI through GStreamer, which has no knowledge
//! of the scheme, so the element fails with MEDIA_ERR_SRC_NOT_SUPPORTED. Images
//! keep using the custom protocol; only video needs an origin GStreamer accepts.
//!
//! The authorization boundary from ADR 0001 is preserved: the URL carries an
//! opaque id and a per-run token, never a filesystem path, and the id must still
//! resolve through the approved image registry.

use std::{
    io::{self, BufRead, BufReader, Seek, SeekFrom, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::OnceLock,
    time::Duration,
};

use uuid::Uuid;

use super::{
    image_registry::ImageId,
    supported_image::{is_video, media_mime_type},
};
use crate::SharedImageRegistry;

/// Caps a request head so a misbehaving client cannot make us buffer without end.
const MAX_REQUEST_HEAD_BYTES: usize = 8 * 1024;
const MAX_REQUEST_HEAD_LINES: usize = 64;
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

static MEDIA_SERVER: OnceLock<MediaServer> = OnceLock::new();

#[derive(Debug)]
pub struct MediaServer {
    port: u16,
    token: String,
}

impl MediaServer {
    pub fn origin(&self) -> String {
        format!("http://{}:{}", Ipv4Addr::LOCALHOST, self.port)
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Binds a random loopback port and serves approved video. Called once at
/// startup; if it fails, video falls back to the custom protocol URL, which
/// cannot play but still fails safely.
pub fn start(registry: SharedImageRegistry) -> io::Result<&'static MediaServer> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
    let port = listener.local_addr()?.port();
    let token = Uuid::new_v4().to_string();
    let server = MediaServer {
        port,
        token: token.clone(),
    };

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            // Without this the engine's sequential range requests pay Nagle
            // meeting a delayed ACK on every hop. Measured at 1.5x on body
            // throughput in benches/render-bench.
            let _ = stream.set_nodelay(true);
            let registry = registry.clone();
            let token = token.clone();
            // ponytail: one thread per connection. The webview opens a handful,
            // and reaching the server at all requires the token; swap in a pool
            // if that ever stops being true.
            std::thread::spawn(move || {
                let _ = serve_connection(stream, &registry, &token, port);
            });
        }
    });

    Ok(MEDIA_SERVER.get_or_init(|| server))
}

/// The URL a `<video>` element should load for an approved id.
pub fn video_url(id: &ImageId) -> Option<String> {
    let server = MEDIA_SERVER.get()?;
    Some(format!(
        "{}/{}/{}",
        server.origin(),
        server.token,
        id.as_str()
    ))
}

#[derive(Debug, Default)]
struct Request {
    method: String,
    path: String,
    host: Option<String>,
    range: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    BadRequest,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    NotSatisfiable { total_bytes: u64 },
    ServerError,
}

impl Status {
    fn line(self) -> &'static str {
        match self {
            Status::BadRequest => "400 Bad Request",
            Status::Forbidden => "403 Forbidden",
            Status::NotFound => "404 Not Found",
            Status::MethodNotAllowed => "405 Method Not Allowed",
            Status::NotSatisfiable { .. } => "416 Range Not Satisfiable",
            Status::ServerError => "500 Internal Server Error",
        }
    }
}

#[derive(Debug)]
struct MediaTarget {
    path: PathBuf,
    mime_type: &'static str,
    total_bytes: u64,
}

fn serve_connection(
    mut stream: TcpStream,
    registry: &SharedImageRegistry,
    token: &str,
    port: u16,
) -> io::Result<()> {
    stream.set_read_timeout(Some(IDLE_TIMEOUT))?;
    stream.set_write_timeout(Some(IDLE_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    // Keep-alive: seeking issues many range requests, and reusing the connection
    // avoids a TCP handshake for each one.
    loop {
        let Some(request) = read_request_head(&mut reader)? else {
            return Ok(());
        };

        match resolve_request(&request, registry, token, port) {
            Ok(target) => {
                if !write_media_response(&mut stream, &request, &target)? {
                    return Ok(());
                }
            }
            Err(status) => {
                write_status_response(&mut stream, status)?;
                return Ok(());
            }
        }
    }
}

fn read_request_head(reader: &mut BufReader<TcpStream>) -> io::Result<Option<Request>> {
    let mut request_line = String::new();
    let mut consumed = match reader.read_line(&mut request_line) {
        Ok(0) => return Ok(None),
        Ok(bytes) => bytes,
        // A closed or idle connection is normal, not an error worth reporting.
        Err(_) => return Ok(None),
    };

    let mut parts = request_line.split_whitespace();
    let mut request = Request {
        method: parts.next().unwrap_or_default().to_string(),
        path: parts.next().unwrap_or_default().to_string(),
        ..Request::default()
    };

    for _ in 0..MAX_REQUEST_HEAD_LINES {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(None),
            Ok(bytes) => consumed += bytes,
            Err(_) => return Ok(None),
        }

        if consumed > MAX_REQUEST_HEAD_BYTES {
            return Ok(None);
        }

        let line = line.trim_end();
        if line.is_empty() {
            return Ok(Some(request));
        }

        if let Some((name, value)) = line.split_once(':') {
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("host") {
                request.host = Some(value);
            } else if name.eq_ignore_ascii_case("range") {
                request.range = Some(value);
            }
        }
    }

    Ok(None)
}

fn resolve_request(
    request: &Request,
    registry: &SharedImageRegistry,
    token: &str,
    port: u16,
) -> Result<MediaTarget, Status> {
    if request.method != "GET" && request.method != "HEAD" {
        return Err(Status::MethodNotAllowed);
    }

    // Blocks DNS rebinding: a page that resolves its own hostname to 127.0.0.1
    // still sends that hostname in Host, which will not match the loopback pair.
    let host = request.host.as_deref().ok_or(Status::BadRequest)?;
    if host != format!("{}:{}", Ipv4Addr::LOCALHOST, port) && host != format!("localhost:{port}") {
        return Err(Status::Forbidden);
    }

    let (request_token, id) = request
        .path
        .strip_prefix('/')
        .and_then(|path| path.split_once('/'))
        .ok_or(Status::NotFound)?;

    if request_token != token {
        return Err(Status::Forbidden);
    }

    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err(Status::NotFound);
    }

    let registry = registry.lock().map_err(|_| Status::ServerError)?;
    let path = registry
        .path_for(&ImageId::from_opaque(id))
        .ok_or(Status::NotFound)?;

    // Only video is served from this origin; images stay on the custom protocol
    // so the surface here stays as small as possible.
    if !is_video(path) {
        return Err(Status::Forbidden);
    }

    let mime_type = media_mime_type(path).ok_or(Status::Forbidden)?;
    let total_bytes = std::fs::metadata(path).map_err(|_| Status::NotFound)?.len();

    Ok(MediaTarget {
        path: path.to_path_buf(),
        mime_type,
        total_bytes,
    })
}

/// Streams the body straight from the file to the socket, so memory stays flat
/// no matter how large the video or the requested range is. Returns whether the
/// connection can be reused.
fn write_media_response(
    stream: &mut TcpStream,
    request: &Request,
    target: &MediaTarget,
) -> io::Result<bool> {
    let total = target.total_bytes;
    let requested = request
        .range
        .as_deref()
        .and_then(|header| parse_byte_range(header, total));

    let (status_line, start, length) = match requested {
        Some((start, _)) if start >= total => {
            write_status_response(stream, Status::NotSatisfiable { total_bytes: total })?;
            return Ok(false);
        }
        Some((start, end)) => {
            let last = total.saturating_sub(1);
            let end = end.unwrap_or(last).min(last);
            if end < start {
                write_status_response(stream, Status::NotSatisfiable { total_bytes: total })?;
                return Ok(false);
            }
            ("206 Partial Content", start, end - start + 1)
        }
        None => ("200 OK", 0, total),
    };

    let mut head = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: {}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n",
        target.mime_type
    );
    if status_line.starts_with("206") {
        let end = start + length - 1;
        head.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;

    if request.method == "HEAD" {
        stream.flush()?;
        return Ok(true);
    }

    let mut file = std::fs::File::open(&target.path)?;
    file.seek(SeekFrom::Start(start))?;
    // io::copy streams through a small internal buffer; the whole video is never
    // held in memory, which is the point of serving it over this origin.
    let copied = io::copy(&mut io::Read::take(file, length), stream)?;
    stream.flush()?;

    // A short read means the file changed underneath us and the promised
    // Content-Length is now a lie, so the connection must not be reused.
    Ok(copied == length)
}

fn write_status_response(stream: &mut TcpStream, status: Status) -> io::Result<()> {
    let body = status.line();
    let mut head = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n",
        status.line(),
        body.len()
    );
    if let Status::NotSatisfiable { total_bytes } = status {
        head.push_str(&format!("Content-Range: bytes */{total_bytes}\r\n"));
    }
    head.push_str("\r\n");
    head.push_str(body);
    stream.write_all(head.as_bytes())?;
    stream.flush()
}

/// Parses a single `Range: bytes=…` spec. Multi-range is not honoured; `None`
/// makes the caller serve the whole resource.
fn parse_byte_range(header: &str, total_bytes: u64) -> Option<(u64, Option<u64>)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let (start, end) = (start.trim(), end.trim());

    if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        return Some((total_bytes.saturating_sub(suffix), None));
    }

    let start = start.parse().ok()?;
    let end = if end.is_empty() {
        None
    } else {
        Some(end.parse().ok()?)
    };

    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::image_registry::ApprovedImageRegistry;
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    /// Spins a server on its own listener so tests do not depend on the process
    /// wide OnceLock, and returns the port plus token.
    fn spawn_test_server(registry: SharedImageRegistry) -> (u16, String) {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = "test-token".to_string();
        let served_token = token.clone();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let registry = registry.clone();
                let token = served_token.clone();
                std::thread::spawn(move || {
                    let _ = serve_connection(stream, &registry, &token, port);
                });
            }
        });

        (port, token)
    }

    fn request(port: u16, raw: &str) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream.write_all(raw.as_bytes()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response);
        String::from_utf8_lossy(&response).to_string()
    }

    fn approved_video() -> (tempfile::TempDir, SharedImageRegistry, String) {
        let directory = tempdir().expect("temp dir");
        let video = directory.path().join("clip.mp4");
        std::fs::write(&video, b"0123456789").expect("video file");

        let registry: SharedImageRegistry = Arc::new(Mutex::new(ApprovedImageRegistry::default()));
        let id = registry
            .lock()
            .unwrap()
            .approve_path(&video)
            .expect("approved video")
            .id()
            .as_str()
            .to_string();

        (directory, registry, id)
    }

    #[test]
    fn approved_video_is_served_with_range_support() {
        let (_dir, registry, id) = approved_video();
        let (port, token) = spawn_test_server(registry);

        let response = request(
            port,
            &format!(
                "GET /{token}/{id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=2-5\r\nConnection: close\r\n\r\n"
            ),
        );

        assert!(
            response.starts_with("HTTP/1.1 206 Partial Content"),
            "{response}"
        );
        assert!(
            response.contains("Content-Range: bytes 2-5/10"),
            "{response}"
        );
        assert!(response.contains("Content-Type: video/mp4"), "{response}");
        assert!(response.ends_with("2345"), "{response}");
    }

    #[test]
    fn a_request_without_a_range_serves_the_whole_video() {
        let (_dir, registry, id) = approved_video();
        let (port, token) = spawn_test_server(registry);

        let response = request(
            port,
            &format!("GET /{token}/{id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        );

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains("Content-Length: 10"), "{response}");
        assert!(response.ends_with("0123456789"), "{response}");
    }

    #[test]
    fn the_wrong_token_is_refused() {
        let (_dir, registry, id) = approved_video();
        let (port, _token) = spawn_test_server(registry);

        let response = request(
            port,
            &format!("GET /not-the-token/{id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        );

        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    }

    #[test]
    fn a_foreign_host_header_is_refused_to_block_dns_rebinding() {
        let (_dir, registry, id) = approved_video();
        let (port, token) = spawn_test_server(registry);

        let response = request(
            port,
            &format!("GET /{token}/{id} HTTP/1.1\r\nHost: evil.example.com\r\n\r\n"),
        );

        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    }

    #[test]
    fn unapproved_and_non_video_ids_are_refused() {
        let directory = tempdir().expect("temp dir");
        let image = directory.path().join("photo.png");
        std::fs::write(&image, b"png bytes").expect("image file");

        let registry: SharedImageRegistry = Arc::new(Mutex::new(ApprovedImageRegistry::default()));
        let image_id = registry
            .lock()
            .unwrap()
            .approve_path(&image)
            .expect("approved image")
            .id()
            .as_str()
            .to_string();

        let (port, token) = spawn_test_server(registry);

        // An approved *image* must not be reachable from the media origin.
        let image_response = request(
            port,
            &format!("GET /{token}/{image_id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        );
        assert!(
            image_response.starts_with("HTTP/1.1 403"),
            "{image_response}"
        );

        // An id that was never approved is unknown.
        let unknown = request(
            port,
            &format!("GET /{token}/never-approved HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        );
        assert!(unknown.starts_with("HTTP/1.1 404"), "{unknown}");
    }

    #[test]
    fn write_methods_are_rejected() {
        let (_dir, registry, id) = approved_video();
        let (port, token) = spawn_test_server(registry);

        let response = request(
            port,
            &format!("DELETE /{token}/{id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        );

        assert!(response.starts_with("HTTP/1.1 405"), "{response}");
    }

    #[test]
    fn a_range_past_the_end_is_not_satisfiable() {
        let (_dir, registry, id) = approved_video();
        let (port, token) = spawn_test_server(registry);

        let response = request(
            port,
            &format!(
                "GET /{token}/{id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=10-\r\n\r\n"
            ),
        );

        assert!(response.starts_with("HTTP/1.1 416"), "{response}");
        assert!(response.contains("Content-Range: bytes */10"), "{response}");
    }

    #[test]
    fn suffix_and_open_ended_ranges_are_honoured() {
        let (_dir, registry, id) = approved_video();
        let (port, token) = spawn_test_server(registry);

        let suffix = request(
            port,
            &format!(
                "GET /{token}/{id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=-3\r\nConnection: close\r\n\r\n"
            ),
        );
        assert!(suffix.contains("Content-Range: bytes 7-9/10"), "{suffix}");
        assert!(suffix.ends_with("789"), "{suffix}");

        let open_ended = request(
            port,
            &format!(
                "GET /{token}/{id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=7-\r\nConnection: close\r\n\r\n"
            ),
        );
        assert!(
            open_ended.contains("Content-Range: bytes 7-9/10"),
            "{open_ended}"
        );
    }

    #[test]
    fn head_requests_return_headers_without_a_body() {
        let (_dir, registry, id) = approved_video();
        let (port, token) = spawn_test_server(registry);

        let response = request(
            port,
            &format!("HEAD /{token}/{id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        );

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains("Content-Length: 10"), "{response}");
        assert!(response.ends_with("\r\n\r\n"), "{response}");
    }
}
