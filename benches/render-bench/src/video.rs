//! The transport tax on video playback.
//!
//! Manzar cannot hand video to the webview over its custom protocol, because
//! WebKitGTK resolves `<video>` URIs through GStreamer, which knows nothing of
//! app-registered schemes. `core/media_server.rs` works around that with a
//! loopback HTTP server, and does so carefully — it streams with `io::copy`, so
//! process memory stays flat regardless of file size.
//!
//! What it cannot avoid is the hop itself: every byte of every frame crosses a
//! TCP socket and an HTTP framing layer before the decoder sees it. This module
//! prices that hop against reading the same bytes directly, which is what a
//! native decoder in the same process would do.
//!
//! The result is a floor, not the whole story: decode dominates video playback,
//! and decode is what a native renderer moves to hardware. But the floor is the
//! part that can be measured here without a media stack installed.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};

/// Matches `image_protocol::VIDEO_CHUNK_BYTES`, the slice size the viewer
/// actually requests.
pub const CHUNK_BYTES: u64 = 4 * 1024 * 1024;

/// Serves byte ranges over loopback with the same response shape as
/// `media_server.rs`, so the measured overhead is the one that ships.
pub struct LoopbackServer {
    pub port: u16,
}

impl LoopbackServer {
    /// `nodelay` disables Nagle on each accepted connection. `media_server.rs`
    /// does not do this today, and measurement shows it is worth roughly a
    /// factor of two on body throughput, so it is a parameter here rather than
    /// a constant: the report compares both settings.
    pub fn start(path: PathBuf, nodelay: bool) -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
        let port = listener.local_addr()?.port();

        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let path = path.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, &path, nodelay);
                });
            }
        });

        Ok(Self { port })
    }
}

fn serve(mut stream: TcpStream, path: &Path, nodelay: bool) -> std::io::Result<()> {
    if nodelay {
        stream.set_nodelay(true)?;
    }
    let mut reader = BufReader::new(stream.try_clone()?);
    let total = std::fs::metadata(path)?.len();

    loop {
        let mut start = 0u64;
        let mut length = total;

        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(range) = line.strip_prefix("Range: bytes=") {
                if let Some((from, to)) = range.split_once('-') {
                    start = from.parse().unwrap_or(0);
                    let end: u64 = to.parse().unwrap_or(total - 1);
                    length = end.saturating_sub(start) + 1;
                }
            }
        }

        let head = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\nConnection: keep-alive\r\n\r\n"
        );
        stream.write_all(head.as_bytes())?;

        let mut file = std::fs::File::open(path)?;
        file.seek(SeekFrom::Start(start))?;
        std::io::copy(&mut Read::take(file, length), &mut stream)?;
        stream.flush()?;
    }
}

/// Pulls one chunk the way the webview does: an HTTP range request over
/// loopback, response head parsed, body drained.
pub fn fetch_chunk_over_http(stream: &mut TcpStream, start: u64, length: u64) -> std::io::Result<u64> {
    write!(
        stream,
        "GET /clip HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes={start}-{}\r\n\r\n",
        start + length - 1
    )?;
    stream.flush()?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut content_length = 0u64;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            content_length = value.parse().unwrap_or(0);
        }
    }

    let mut sink = vec![0u8; content_length as usize];
    reader.read_exact(&mut sink)?;
    Ok(sink.len() as u64)
}

/// Reads the same chunk directly, as an in-process decoder would.
pub fn read_chunk_directly(path: &Path, start: u64, length: u64) -> std::io::Result<u64> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut buffer = vec![0u8; length as usize];
    let read = Read::take(&mut file, length).read(&mut buffer)?;
    Ok(read as u64)
}

/// Writes a file of `bytes` incompressible-ish content to stand in for a clip.
/// Only the transport is under test, so the content need not be a real MP4.
pub fn ensure_clip(directory: &Path, bytes: u64) -> std::io::Result<PathBuf> {
    let path = directory.join(format!("clip-{bytes}.bin"));
    if path.metadata().is_ok_and(|metadata| metadata.len() == bytes) {
        return Ok(path);
    }

    let mut buffer = vec![0u8; 1 << 20];
    for (index, slot) in buffer.iter_mut().enumerate() {
        *slot = (index as u32).wrapping_mul(2_654_435_761).to_le_bytes()[0];
    }

    let file = std::fs::File::create(&path)?;
    let mut writer = std::io::BufWriter::new(file);
    let mut written = 0u64;
    while written < bytes {
        let take = buffer.len().min((bytes - written) as usize);
        writer.write_all(&buffer[..take])?;
        written += take as u64;
    }
    writer.flush()?;
    Ok(path)
}
