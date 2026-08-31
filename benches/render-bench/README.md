# render-bench

Measures the image pipeline Manzar runs today against the Rust-native renderer
proposed in [`rust-renderer-plan.html`](../../rust-renderer-plan.html). Every
figure in that write-up comes from here.

The crate is deliberately standalone — it declares its own empty `[workspace]`
and is not referenced by `src-tauri` — so nothing it depends on can reach a
release build of the app.

## Running it

```sh
cd benches/render-bench

cargo run --release --bin report   # every figure, written to report.json
cargo bench --bench pipeline       # the same comparisons under criterion
cargo test --release               # the invariants the comparison rests on
cargo run --release --bin diag     # where the loopback video hop spends its time
```

A release build is not optional. Every SIMD path measured here degrades to
scalar without optimisation, so a debug run reports fiction.

## The corpus

Fixtures are generated on first run and cached in the system temp directory, or
in `$MANZAR_BENCH_CORPUS` if that is set. Generation is deterministic, so a
cached corpus and a fresh one produce identical measurements. The 61 MP fixture
takes about a minute to build the first time.

Photographs cannot be committed to the repository, and a flat test pattern would
be dishonest — it compresses to nearly nothing, so a JPEG decode benchmark over
it would measure the file header rather than the entropy decode.
`corpus::synthesise` therefore builds lighting ramps, a mid-frequency subject
and per-pixel grain, which lands quality-85 JPEG output in the same
bits-per-pixel range as a camera file.

## What is being compared

Two pipelines that put the same pixels on the same screen:

- **`webview`** — what ships today. The whole file is read into memory and handed
  over the custom protocol; the engine decodes every pixel and re-rasters the
  full surface on each zoom step.
- **`native`** — what the plan proposes. Rust decodes at the smallest DCT scale
  that still covers the window, resizes with SIMD, and hands over only the pixels
  that will be lit.

WebKitGTK cannot be driven from a benchmark harness, so the engine's side is
*modelled*: a full-resolution decode with a production Rust decoder, then a
full-surface rescale. That model charges the webview for the decode and the
scale and nothing else — not the protocol copy, not the source surface it keeps
alongside the decoded one, not the per-frame texture upload. The engine's own
decoders are in the same performance class as the ones used here, so the model
is conservative: the shipping path costs at least this much.

`paths.rs` carries the same caveat at the top, next to the code it applies to.

## Reading the output

`report.json` holds one record per comparison. `baseline` is always the path
that ships today and `candidate` is what the plan proposes, so `speedup` is
always "how much better the proposal is". Timings are medians of up to seven
runs after a warmup, capped by a per-measurement time budget — the large
fixtures would otherwise dominate a run.

`baseline_peak_bytes` and `candidate_peak_bytes` are high-water marks of live
allocations, recorded by the tracking `GlobalAlloc` in `memory.rs`. They appear
only on the `first-pixel` rows, where holding fewer bytes is the point.

## A note on feature surface

`image`'s `rayon` feature pulls `exr`, `ravif` and `zune-inflate` into the
dependency graph — `zune-inflate` 0.2.54 carries RUSTSEC-2023-0080 — and made no
difference to anything measured here, because `imageops::resize` is
single-threaded either way. This crate therefore declares the same `image`
feature set `src-tauri` does. Worth remembering when the app takes on
`fast_image_resize`: its optional `image` feature is interop this code never
uses, and enabling it re-enables the image crate's defaults.
