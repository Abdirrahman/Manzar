//! Runs every comparison and writes `report.json`, the input to the write-up.
//!
//! Usage: `cargo run --release --bin report -- [output.json]`

use std::time::Duration;

use manzar_render_bench::corpus::{ensure_corpus, Format, Fixture, FIXTURES};
use manzar_render_bench::memory::{peak_bytes_of, TrackingAllocator};
use manzar_render_bench::timing::{measure, Sample};
use manzar_render_bench::{paths, video, VIEWPORT};
use serde::Serialize;

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

const WARMUP: Duration = Duration::from_millis(250);
const MIN_SAMPLES: usize = 7;
const BUDGET: Duration = Duration::from_secs(6);

#[derive(Serialize)]
struct Report {
    machine: Machine,
    viewport: [u32; 2],
    fixtures: Vec<FixtureReport>,
    comparisons: Vec<Comparison>,
}

#[derive(Serialize)]
struct Machine {
    cpu_cores: usize,
    target: &'static str,
    rustc: &'static str,
    simd: Vec<&'static str>,
}

#[derive(Serialize)]
struct FixtureReport {
    name: &'static str,
    width: u32,
    height: u32,
    megapixels: f64,
    /// Encoded size on disk, per format.
    file_bytes: Vec<(String, u64)>,
    /// What the engine must hold once it has decoded the whole thing.
    decoded_rgba_bytes: u64,
    /// What the native path hands over instead: the window, and nothing more.
    viewport_rgb_bytes: u64,
    /// The DCT scale the native decoder picks for this fixture.
    dct_scaled_to: [u32; 2],
}

/// One head-to-head measurement. `baseline` is always the path that ships
/// today; `candidate` is what the plan proposes.
#[derive(Serialize)]
struct Comparison {
    group: &'static str,
    fixture: &'static str,
    detail: String,
    baseline_label: &'static str,
    candidate_label: &'static str,
    baseline: Sample,
    candidate: Sample,
    speedup: f64,
    baseline_peak_bytes: Option<u64>,
    candidate_peak_bytes: Option<u64>,
}

impl Comparison {
    fn new(
        group: &'static str,
        fixture: &'static str,
        detail: impl Into<String>,
        (baseline_label, baseline): (&'static str, Sample),
        (candidate_label, candidate): (&'static str, Sample),
    ) -> Self {
        Self {
            group,
            fixture,
            detail: detail.into(),
            baseline_label,
            candidate_label,
            speedup: candidate.speedup_over(&baseline),
            baseline,
            candidate,
            baseline_peak_bytes: None,
            candidate_peak_bytes: None,
        }
    }

    fn with_peaks(mut self, baseline: u64, candidate: u64) -> Self {
        self.baseline_peak_bytes = Some(baseline);
        self.candidate_peak_bytes = Some(candidate);
        self
    }
}

fn main() -> std::io::Result<()> {
    let directory = ensure_corpus()?;
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "report.json".to_string());

    let mut comparisons = Vec::new();
    let mut fixtures = Vec::new();

    for fixture in FIXTURES {
        fixtures.push(describe(fixture, &directory));

        let jpeg = fixture.path(&directory, Format::Jpeg);
        if !jpeg.exists() {
            continue;
        }
        let bytes = std::fs::read(&jpeg)?;

        comparisons.extend(decode_comparisons(fixture, &bytes));
        comparisons.extend(dct_curve(fixture, &bytes));

        // Every later stage starts from a decoded frame, so decode once here
        // rather than inside each measurement.
        let frame = paths::decode_full_image_crate(&bytes);
        comparisons.push(resize_comparison(fixture, &frame));
        comparisons.extend(zoom_comparisons(fixture, &frame));
        drop(frame);

        comparisons.push(first_pixel_comparison(fixture, &jpeg));

        let png = fixture.path(&directory, Format::Png);
        if png.exists() {
            comparisons.push(png_resize_comparison(fixture, &png)?);
        }
    }

    match video_transport_comparisons(&directory) {
        Ok(measured) => comparisons.extend(measured),
        Err(error) => eprintln!("  video   skipped: {error}"),
    }

    let report = Report {
        machine: Machine {
            cpu_cores: std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(0),
            target: std::env::consts::ARCH,
            rustc: env!("BENCH_RUSTC_VERSION"),
            simd: detected_simd(),
        },
        viewport: [VIEWPORT.0, VIEWPORT.1],
        fixtures,
        comparisons,
    };

    std::fs::write(&output, serde_json::to_string_pretty(&report)?)?;
    eprintln!("wrote {output}");
    Ok(())
}

fn describe(fixture: &Fixture, directory: &std::path::Path) -> FixtureReport {
    let file_bytes = fixture
        .formats
        .iter()
        .filter_map(|format| {
            let path = fixture.path(directory, *format);
            std::fs::metadata(&path)
                .ok()
                .map(|metadata| (format.label().to_string(), metadata.len()))
        })
        .collect();

    let scaled = paths::required_source_size(fixture.width, fixture.height, VIEWPORT);
    let fitted = paths::fit_within(fixture.width, fixture.height, VIEWPORT);

    FixtureReport {
        name: fixture.name,
        width: fixture.width,
        height: fixture.height,
        megapixels: fixture.megapixels(),
        file_bytes,
        decoded_rgba_bytes: fixture.decoded_rgba_bytes(),
        viewport_rgb_bytes: u64::from(fitted.0) * u64::from(fitted.1) * 3,
        dct_scaled_to: [scaled.0, scaled.1],
    }
}

fn decode_comparisons(fixture: &Fixture, bytes: &[u8]) -> Vec<Comparison> {
    eprintln!("  decode  {}", fixture.name);
    let baseline = run(|| paths::decode_full_image_crate(bytes));
    let zune = run(|| paths::decode_full_zune(bytes));
    let scaled = run(|| paths::decode_scaled_jpeg(bytes, VIEWPORT));

    // The scale a viewer would actually pick for this fixture, recorded so the
    // decode-scaled row can say which divisor produced its number.
    let picked = paths::required_source_size(fixture.width, fixture.height, VIEWPORT);
    let divisor = fixture.width.div_ceil(picked.0.max(1)).max(1);

    vec![
        Comparison::new(
            "decode",
            fixture.name,
            "full-resolution decode, same pixels out",
            ("image crate (today)", baseline),
            ("zune-jpeg", zune),
        ),
        Comparison::new(
            "decode-scaled",
            fixture.name,
            format!(
                "decode at 1/{divisor} scale: {}x{} instead of {}x{}",
                picked.0, picked.1, fixture.width, fixture.height
            ),
            ("image crate, full decode (today)", baseline),
            ("jpeg-decoder, DCT-scaled", scaled),
        ),
    ]
}

/// Decode cost at each DCT divisor, each measured against the full decode. The
/// point is the shape of the curve: cost falls roughly with the pixel count,
/// which is why choosing the right scale matters more than choosing a decoder.
fn dct_curve(fixture: &Fixture, bytes: &[u8]) -> Vec<Comparison> {
    eprintln!("  dct     {}", fixture.name);
    let full = run(|| paths::decode_jpeg_at_divisor(bytes, 1));

    [2u32, 4, 8]
        .into_iter()
        .map(|divisor| {
            let scaled = run(|| paths::decode_jpeg_at_divisor(bytes, divisor));
            Comparison::new(
                "dct-curve",
                fixture.name,
                format!(
                    "1/{divisor} scale: {}x{}",
                    fixture.width.div_ceil(divisor),
                    fixture.height.div_ceil(divisor)
                ),
                ("jpeg-decoder, full scale", full),
                ("jpeg-decoder, DCT-scaled", scaled),
            )
        })
        .collect()
}

fn resize_comparison(fixture: &Fixture, frame: &paths::Frame) -> Comparison {
    eprintln!("  resize  {}", fixture.name);
    let baseline = run(|| paths::resize_image_crate(frame, VIEWPORT));
    let candidate = run(|| paths::resize_fir(frame, VIEWPORT));

    Comparison::new(
        "resize",
        fixture.name,
        "fit-to-window, Lanczos3, from a decoded frame",
        ("image::imageops (scalar)", baseline),
        ("fast_image_resize (SIMD)", candidate),
    )
}

fn png_resize_comparison(
    fixture: &Fixture,
    path: &std::path::Path,
) -> std::io::Result<Comparison> {
    eprintln!("  png     {}", fixture.name);
    let bytes = std::fs::read(path)?;
    let frame = paths::decode_full_image_crate(&bytes);

    // PNG has no DCT scale to exploit, so the gain here is the resampler alone.
    // Including it keeps the report honest about which formats benefit and how
    // much of the win is decode versus scale.
    let baseline = run(|| paths::resize_image_crate(&frame, VIEWPORT));
    let candidate = run(|| paths::resize_fir(&frame, VIEWPORT));

    Ok(Comparison::new(
        "resize-png",
        fixture.name,
        "PNG has no DCT scale: this is the resampler gain alone",
        ("image::imageops (scalar)", baseline),
        ("fast_image_resize (SIMD)", candidate),
    ))
}

fn zoom_comparisons(fixture: &Fixture, frame: &paths::Frame) -> Vec<Comparison> {
    eprintln!("  zoom    {}", fixture.name);
    [1.0_f64, 2.0, 4.0]
        .into_iter()
        .map(|scale| {
            let baseline = run(|| paths::zoom_full_reraster(frame, scale));
            let candidate = run(|| paths::zoom_viewport_crop(frame, scale, VIEWPORT));

            Comparison::new(
                "zoom",
                fixture.name,
                format!("one zoom step to {scale:.0}x"),
                ("re-raster the whole image", baseline),
                ("resample the visible rectangle", candidate),
            )
        })
        .collect()
}

fn first_pixel_comparison(fixture: &Fixture, path: &std::path::Path) -> Comparison {
    eprintln!("  open    {}", fixture.name);
    let baseline = run(|| paths::first_pixel_webview(path, VIEWPORT));
    let candidate = run(|| paths::first_pixel_recommended(path, VIEWPORT));

    // Peaks are taken outside the timing loop: the allocator counters are cheap
    // but the measurement should not be charged for them.
    let (_, baseline_peak) = peak_bytes_of(|| paths::first_pixel_webview(path, VIEWPORT));
    let (_, candidate_peak) = peak_bytes_of(|| paths::first_pixel_recommended(path, VIEWPORT));

    Comparison::new(
        "first-pixel",
        fixture.name,
        "file on disk to window-ready pixels",
        ("read whole file, decode all, scale all", baseline),
        ("scale-aware decode, SIMD fit", candidate),
    )
    .with_peaks(baseline_peak, candidate_peak)
}

/// Prices the loopback HTTP hop that video playback goes through today.
///
/// Video cannot use the custom protocol at all — WebKitGTK resolves `<video>`
/// URIs through GStreamer, which does not know the scheme — so every byte
/// crosses a TCP socket. Decode dominates playback and is not measured here;
/// this is the floor under the transport alone.
///
/// Two comparisons come back. The first is the hop as configured today against
/// an in-process read. The second isolates a one-line change to
/// `media_server.rs`: neither end disables Nagle, and the server side of that
/// is the app's to fix.
fn video_transport_comparisons(directory: &std::path::Path) -> std::io::Result<Vec<Comparison>> {
    eprintln!("  video   transport");
    let clip = video::ensure_clip(directory, 256 * 1024 * 1024)?;
    let chunks = 16u64;
    let detail = format!(
        "{chunks} sequential {} MB range requests",
        video::CHUNK_BYTES / (1024 * 1024)
    );

    let drain = |nodelay: bool| -> std::io::Result<Sample> {
        let server = video::LoopbackServer::start(clip.clone(), nodelay)?;
        let mut stream =
            std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, server.port))?;
        Ok(run(|| {
            for index in 0..chunks {
                video::fetch_chunk_over_http(
                    &mut stream,
                    index * video::CHUNK_BYTES,
                    video::CHUNK_BYTES,
                )
                .expect("chunk served");
            }
        }))
    };

    let shipping = drain(false)?;
    let tuned = drain(true)?;
    let direct = run(|| {
        for index in 0..chunks {
            video::read_chunk_directly(&clip, index * video::CHUNK_BYTES, video::CHUNK_BYTES)
                .expect("chunk read");
        }
    });

    Ok(vec![
        Comparison::new(
            "video-transport",
            "256mb-clip",
            detail.clone(),
            ("loopback HTTP, as shipped", shipping),
            ("in-process read", direct),
        ),
        Comparison::new(
            "video-transport-nodelay",
            "256mb-clip",
            format!("{detail}; server-side TCP_NODELAY only"),
            ("loopback HTTP, as shipped", shipping),
            ("loopback HTTP, Nagle disabled", tuned),
        ),
    ])
}

fn run<T>(body: impl FnMut() -> T) -> Sample {
    measure(WARMUP, MIN_SAMPLES, BUDGET, body)
}

/// Reports the widest SIMD the build can actually dispatch to, so a reader can
/// tell whether these numbers came from an AVX2 machine or a scalar fallback.
fn detected_simd() -> Vec<&'static str> {
    let mut detected = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse4.1") {
            detected.push("sse4.1");
        }
        if std::arch::is_x86_feature_detected!("avx2") {
            detected.push("avx2");
        }
        if std::arch::is_x86_feature_detected!("avx512f") {
            detected.push("avx512f");
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if std::arch::is_aarch64_feature_detected!("neon") {
            detected.push("neon");
        }
    }
    detected
}
