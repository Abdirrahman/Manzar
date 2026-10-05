//! Shipping image-serving latency for a six-image working set at viewer size.
//! No webview or cold-disk timing is inferred. Run with a generated corpus:
//! cargo run --release --locked --example navigation_bench -- /tmp/manzar-rewrite-corpus /tmp/navigation.json
use std::{path::PathBuf, time::Instant};

use manzar_lib::core::image_protocol::serve_media_path;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let source = PathBuf::from(&args[1]);
    let directory = tempfile::tempdir().unwrap();
    let viewport = Some((1280, 768));
    let paths: Vec<_> = (0..6)
        .map(|index| {
            let extension = if index % 3 == 2 { "png" } else { "jpg" };
            let path = directory.path().join(format!("photo{index}.{extension}"));
            std::fs::copy(source.join(format!("rgb.{extension}")), &path).unwrap();
            path
        })
        .collect();
    let mut cold = Vec::new();
    let reference: Vec<_> = paths
        .iter()
        .map(|path| {
            let start = Instant::now();
            let response = serve_media_path(path, None, viewport).unwrap();
            cold.push(start.elapsed().as_secs_f64() * 1000.0);
            response.into_bytes()
        })
        .collect();
    let mut warm = Vec::new();
    for index in 0..120 {
        let position = index % paths.len();
        let start = Instant::now();
        let response = serve_media_path(&paths[position], None, viewport).unwrap();
        warm.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(response.bytes(), reference[position]);
    }
    let mut sorted = warm.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let p95 = sorted[(sorted.len() * 95).div_ceil(100) - 1];
    let report = serde_json::json!({
        "viewport": [1280, 768], "working_set": 6,
        "scope": "shipping Rust image-serving API, warm page cache, no webview timing",
        "cold_ms": cold, "warm_ms": warm, "warm_median_ms": median, "warm_p95_ms": p95,
        "correctness": "every response matches its first-render bytes",
    });
    std::fs::write(&args[2], serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!("Repeated switching: median {median:.2} ms, p95 {p95:.2} ms");
    assert!(
        median < 16.0 && p95 < 25.0,
        "repeated switching exceeds frame budget: median {median:.2} ms, p95 {p95:.2} ms"
    );
}
