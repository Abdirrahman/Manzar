//! Criterion benchmarks over the same pipelines the report binary measures.
//!
//! `cargo run --release --bin report` produces the figures for the write-up;
//! this file is the statistically rigorous version for anyone who wants
//! confidence intervals and regression tracking across commits.
//!
//! Run with `cargo bench --bench pipeline`.

use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use manzar_render_bench::corpus::{ensure_corpus, Format, FIXTURES};
use manzar_render_bench::paths;
use manzar_render_bench::VIEWPORT;

fn decode(criterion: &mut Criterion) {
    let directory = ensure_corpus().expect("corpus");
    let mut group = criterion.benchmark_group("decode");
    group.sample_size(10).measurement_time(Duration::from_secs(12));

    for fixture in FIXTURES {
        let Some(path) = fixture
            .formats
            .contains(&Format::Jpeg)
            .then(|| fixture.path(&directory, Format::Jpeg))
        else {
            continue;
        };
        let bytes = std::fs::read(&path).expect("fixture readable");
        group.throughput(Throughput::Elements(
            u64::from(fixture.width) * u64::from(fixture.height),
        ));

        group.bench_with_input(
            BenchmarkId::new("full/image-crate", fixture.name),
            &bytes,
            |bencher, bytes| bencher.iter(|| paths::decode_full_image_crate(bytes)),
        );
        group.bench_with_input(
            BenchmarkId::new("full/zune-jpeg", fixture.name),
            &bytes,
            |bencher, bytes| bencher.iter(|| paths::decode_full_zune(bytes)),
        );
        group.bench_with_input(
            BenchmarkId::new("dct-scaled/jpeg-decoder", fixture.name),
            &bytes,
            |bencher, bytes| bencher.iter(|| paths::decode_scaled_jpeg(bytes, VIEWPORT)),
        );
    }

    group.finish();
}

fn resize(criterion: &mut Criterion) {
    let directory = ensure_corpus().expect("corpus");
    let mut group = criterion.benchmark_group("resize-to-viewport");
    group.sample_size(10).measurement_time(Duration::from_secs(12));

    for fixture in FIXTURES {
        let path = fixture.path(&directory, Format::Jpeg);
        if !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path).expect("fixture readable");
        let frame = paths::decode_full_image_crate(&bytes);

        group.bench_with_input(
            BenchmarkId::new("image-crate/lanczos3", fixture.name),
            &frame,
            |bencher, frame| bencher.iter(|| paths::resize_image_crate(frame, VIEWPORT)),
        );
        group.bench_with_input(
            BenchmarkId::new("fast-image-resize/lanczos3", fixture.name),
            &frame,
            |bencher, frame| bencher.iter(|| paths::resize_fir(frame, VIEWPORT)),
        );
    }

    group.finish();
}

fn zoom(criterion: &mut Criterion) {
    let directory = ensure_corpus().expect("corpus");
    let mut group = criterion.benchmark_group("zoom-step");
    group.sample_size(10).measurement_time(Duration::from_secs(12));

    for fixture in FIXTURES {
        let path = fixture.path(&directory, Format::Jpeg);
        if !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path).expect("fixture readable");
        let frame = paths::decode_full_image_crate(&bytes);

        for scale in [1.0_f64, 2.0, 4.0] {
            let id = format!("{}@{}x", fixture.name, scale);
            group.bench_with_input(
                BenchmarkId::new("full-reraster", &id),
                &frame,
                |bencher, frame| bencher.iter(|| paths::zoom_full_reraster(frame, scale)),
            );
            group.bench_with_input(
                BenchmarkId::new("viewport-crop", &id),
                &frame,
                |bencher, frame| {
                    bencher.iter(|| paths::zoom_viewport_crop(frame, scale, VIEWPORT))
                },
            );
        }
    }

    group.finish();
}

fn first_pixel(criterion: &mut Criterion) {
    let directory = ensure_corpus().expect("corpus");
    let mut group = criterion.benchmark_group("first-pixel");
    group.sample_size(10).measurement_time(Duration::from_secs(15));

    for fixture in FIXTURES {
        let path = fixture.path(&directory, Format::Jpeg);
        if !path.exists() {
            continue;
        }

        group.bench_with_input(
            BenchmarkId::new("webview-model", fixture.name),
            &path,
            |bencher, path| bencher.iter(|| paths::first_pixel_webview(path, VIEWPORT)),
        );
        group.bench_with_input(
            BenchmarkId::new("native", fixture.name),
            &path,
            |bencher, path| bencher.iter(|| paths::first_pixel_recommended(path, VIEWPORT)),
        );
    }

    group.finish();
}

criterion_group!(benches, decode, resize, zoom, first_pixel);
criterion_main!(benches);
