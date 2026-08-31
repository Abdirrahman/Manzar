//! Measures the shipping protocol handler with and without a viewport hint.
//!
//! Unlike `benches/render-bench`, which models the pipeline, this calls the
//! function the application actually serves images through — so the Rust side
//! of every figure here is the real code path.
//!
//! The engine's side still has to be modelled: WebKitGTK cannot be driven from
//! a benchmark. It is modelled the same conservative way the plan uses — a
//! decode of whatever bytes the engine is handed, plus a scale to the window
//! when those bytes are larger than it — which charges the engine for the
//! decode and the scale and nothing else. The real engine additionally copies
//! across the protocol boundary and keeps a source surface, so the baseline
//! costs at least this much.
//!
//! One figure is neither measured nor modelled but exact: `decodedBytes`, the
//! RGBA surface the engine must hold for the bytes it was given. That is
//! arithmetic on the dimensions it receives.
//!
//! Run with:
//!     cargo run --release --example fit_bench

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use manzar_lib::core::{
    image_protocol::serve_approved_media, image_registry::ApprovedImageRegistry,
};

/// The window a desktop viewer is realistically asked to fill.
const VIEWPORT: (u32, u32) = (2560, 1440);

struct Fixture {
    name: &'static str,
    width: u32,
    height: u32,
    png: bool,
}

const FIXTURES: &[Fixture] = &[
    Fixture { name: "12mp", width: 4000, height: 3000, png: false },
    Fixture { name: "24mp", width: 6000, height: 4000, png: false },
    Fixture { name: "61mp", width: 9504, height: 6336, png: false },
    Fixture { name: "12mp-png", width: 4000, height: 3000, png: true },
];

fn main() {
    let corpus = std::env::var_os("MANZAR_BENCH_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("manzar-fit-bench"));
    std::fs::create_dir_all(&corpus).expect("corpus directory");

    let mut rows = Vec::new();

    for fixture in FIXTURES {
        let path = synthesise(&corpus, fixture);
        let runs = if fixture.width > 8000 { 3 } else { 7 };

        let mut registry = ApprovedImageRegistry::default();
        let id = registry
            .approve_path(&path)
            .expect("fixture is a supported image")
            .id()
            .clone();

        // Today: the whole file is served, and the engine decodes every pixel
        // of it and scales the full surface down to the window.
        let baseline = median(runs, || {
            let response =
                serve_approved_media(&registry, &id, None, None).expect("baseline response");
            let decoded = image::load_from_memory(response.bytes()).expect("decodes");
            let (width, height) = fit_within(decoded.width(), decoded.height(), VIEWPORT);
            let _ = image::imageops::resize(
                &decoded.into_rgb8(),
                width,
                height,
                image::imageops::FilterType::Lanczos3,
            );
        });

        // Now: Rust decodes no larger than the window needs and hands over one
        // window-sized surface, which the engine only has to decode.
        let candidate = median(runs, || {
            let response = serve_approved_media(&registry, &id, None, Some(VIEWPORT))
                .expect("fitted response");
            let _ = image::load_from_memory(response.bytes()).expect("decodes");
        });

        let baseline_bytes = serve_approved_media(&registry, &id, None, None)
            .expect("baseline response")
            .bytes()
            .len();
        let fitted = serve_approved_media(&registry, &id, None, Some(VIEWPORT))
            .expect("fitted response");
        let fitted_bytes = fitted.bytes().len();
        let fitted_mime = fitted.mime_type();

        let displayed = fit_within(fixture.width, fixture.height, VIEWPORT);
        let baseline_decoded = u64::from(fixture.width) * u64::from(fixture.height) * 4;
        let fitted_decoded = u64::from(displayed.0) * u64::from(displayed.1) * 4;

        println!(
            "{:9} {:8.1} ms -> {:8.1} ms  {:5.1}x   engine decodes {:>6} MB -> {:>5} MB   \
             transport {:>6} KB -> {:>6} KB ({})",
            fixture.name,
            baseline,
            candidate,
            baseline / candidate,
            baseline_decoded / 1_048_576,
            fitted_decoded / 1_048_576,
            baseline_bytes / 1024,
            fitted_bytes / 1024,
            fitted_mime,
        );

        rows.push(format!(
            r#"{{"fixture":"{}","w":{},"h":{},"baselineMs":{:.1},"candidateMs":{:.1},"speedup":{:.2},"baselineDecodedBytes":{},"fittedDecodedBytes":{},"baselineTransportBytes":{},"fittedTransportBytes":{},"fittedMime":"{}","runs":{}}}"#,
            fixture.name,
            fixture.width,
            fixture.height,
            baseline,
            candidate,
            baseline / candidate,
            baseline_decoded,
            fitted_decoded,
            baseline_bytes,
            fitted_bytes,
            fitted_mime,
            runs,
        ));
    }

    let report = format!(
        r#"{{"viewport":[{},{}],"rows":[{}]}}"#,
        VIEWPORT.0,
        VIEWPORT.1,
        rows.join(",")
    );
    let out = Path::new("fit-bench-report.json");
    std::fs::write(out, &report).expect("report written");
    println!("\nwrote {}", out.display());
}

fn median(runs: usize, mut body: impl FnMut()) -> f64 {
    let mut samples: Vec<f64> = (0..runs)
        .map(|_| {
            let started = Instant::now();
            body();
            started.elapsed().as_secs_f64() * 1000.0
        })
        .collect();

    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn fit_within(width: u32, height: u32, target: (u32, u32)) -> (u32, u32) {
    let scale = (f64::from(target.0) / f64::from(width))
        .min(f64::from(target.1) / f64::from(height))
        .min(1.0);

    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

/// A flat test pattern would be dishonest: it compresses to nearly nothing, so
/// a JPEG decode over it would measure the file header rather than the entropy
/// decode. This builds a lighting ramp, a mid-frequency subject and per-pixel
/// grain, which lands quality-85 output in a photographic bits-per-pixel range.
fn synthesise(corpus: &Path, fixture: &Fixture) -> PathBuf {
    let extension = if fixture.png { "png" } else { "jpg" };
    let path = corpus.join(format!("{}.{extension}", fixture.name));

    if path.exists() {
        return path;
    }

    println!("generating {} ...", path.display());

    let mut noise = 0x2545_F491_4F6C_DD1Du64;
    let frame = image::RgbImage::from_fn(fixture.width, fixture.height, |x, y| {
        // xorshift, inlined so the corpus needs no dependency and stays
        // deterministic across machines.
        noise ^= noise << 13;
        noise ^= noise >> 7;
        noise ^= noise << 17;
        let grain = (noise >> 56) as u8 / 8;

        let fx = f64::from(x) / f64::from(fixture.width);
        let fy = f64::from(y) / f64::from(fixture.height);
        let subject = ((fx * 24.0).sin() * (fy * 18.0).cos() * 60.0 + 128.0) as u8;

        image::Rgb([
            subject.saturating_add(grain),
            ((fy * 200.0) as u8).saturating_add(grain),
            ((fx * 180.0) as u8).saturating_add(grain),
        ])
    });

    frame.save(&path).expect("fixture written");
    path
}
