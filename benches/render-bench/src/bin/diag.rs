//! One-off diagnostic: where does the loopback hop actually spend its time?
//! Kept in the tree because the answer changed a recommendation.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::Instant;

use manzar_render_bench::video;

fn main() -> std::io::Result<()> {
    let dir = std::env::temp_dir();
    let clip = video::ensure_clip(&dir, 256 * 1024 * 1024)?;
    let total = 64u64 * 1024 * 1024;

    // A: raw loopback throughput with a large buffer and no HTTP at all.
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let buffer = vec![0u8; 1 << 20];
            let mut sent = 0u64;
            while sent < total {
                let take = (total - sent).min(buffer.len() as u64) as usize;
                if stream.write_all(&buffer[..take]).is_err() {
                    break;
                }
                sent += take as u64;
            }
        }
    });
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    let started = Instant::now();
    let mut sink = vec![0u8; 1 << 20];
    let mut got = 0u64;
    while got < total {
        let n = stream.read(&mut sink)?;
        if n == 0 { break; }
        got += n as u64;
    }
    let raw = started.elapsed();
    println!("A raw loopback, 1 MB writes : {:>8.1} ms  {:>7.0} MB/s", raw.as_secs_f64()*1000.0, total as f64/1e6/raw.as_secs_f64());

    // B: the shipping shape - io::copy from file to socket, 16 range requests.
    let server = video::LoopbackServer::start(clip.clone(), std::env::var_os("BENCH_TCP_NODELAY").is_some())?;
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port))?;
    for _ in 0..2 { // warm the page cache
        for i in 0..16u64 { video::fetch_chunk_over_http(&mut stream, i*video::CHUNK_BYTES, video::CHUNK_BYTES)?; }
    }
    let started = Instant::now();
    for i in 0..16u64 { video::fetch_chunk_over_http(&mut stream, i*video::CHUNK_BYTES, video::CHUNK_BYTES)?; }
    let http = started.elapsed();
    println!("B io::copy + HTTP, 16x4 MB  : {:>8.1} ms  {:>7.0} MB/s", http.as_secs_f64()*1000.0, total as f64/1e6/http.as_secs_f64());

    // C: one 64 MB request instead of 16 - isolates per-request overhead.
    let started = Instant::now();
    video::fetch_chunk_over_http(&mut stream, 0, total)?;
    let single = started.elapsed();
    println!("C io::copy + HTTP, 1x64 MB  : {:>8.1} ms  {:>7.0} MB/s", single.as_secs_f64()*1000.0, total as f64/1e6/single.as_secs_f64());

    // B2: split one request into "wait for response head" and "drain body", to
    // see whether the per-request cost is a server-side stall or transfer ramp.
    {
        use std::io::BufRead;
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port))?;
        if std::env::var_os("BENCH_CLIENT_NODELAY").is_some() {
            s.set_nodelay(true)?;
        }
        let mut head_total = std::time::Duration::ZERO;
        let mut body_total = std::time::Duration::ZERO;
        let mut reader = std::io::BufReader::new(s.try_clone()?);
        for i in 0..16u64 {
            let start = i * video::CHUNK_BYTES;
            write!(s, "GET /clip HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes={}-{}\r\n\r\n", start, start + video::CHUNK_BYTES - 1)?;
            s.flush()?;

            let t0 = Instant::now();
            let mut length = 0u64;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line)?;
                let line = line.trim_end();
                if line.is_empty() { break; }
                if let Some(v) = line.strip_prefix("Content-Length: ") { length = v.parse().unwrap_or(0); }
            }
            head_total += t0.elapsed();

            let t1 = Instant::now();
            let mut sink = vec![0u8; length as usize];
            reader.read_exact(&mut sink)?;
            body_total += t1.elapsed();
        }
        println!("B2 head wait                : {:>8.1} ms  ({:.1} ms/request)", head_total.as_secs_f64()*1000.0, head_total.as_secs_f64()*1000.0/16.0);
        println!("B2 body drain               : {:>8.1} ms  ({:>7.0} MB/s)", body_total.as_secs_f64()*1000.0, total as f64/1e6/body_total.as_secs_f64());
    }

    // D: in-process read of the same bytes.
    let started = Instant::now();
    for i in 0..16u64 { video::read_chunk_directly(&clip, i*video::CHUNK_BYTES, video::CHUNK_BYTES)?; }
    let direct = started.elapsed();
    println!("D in-process read, 16x4 MB  : {:>8.1} ms  {:>7.0} MB/s", direct.as_secs_f64()*1000.0, total as f64/1e6/direct.as_secs_f64());
    Ok(())
}
