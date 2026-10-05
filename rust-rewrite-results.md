# Rust image rewrite: implementation and measured results

Implemented 5 October 2026 against baseline `da19ceef6e47de3af6d3eee39b7e502fb860158c`.

## Result

The confirmed publishing gate is met: complete Rust grayscale fitting improved by **60.9% at the median and 60.2% at p95**, while retaining byte-identical output. The rotated crop save improved by **14.8% at the median**. The other measured median changes are within 0.5%, which is noise rather than evidence of a gain or regression.

These figures describe the shipping Rust image-serving and crop APIs. They do not measure the webview, command scheduling, cold storage, or overall viewer latency. The crop gain is below 25%; the grayscale operation is the targeted operation that meets the agreed 25% gate.

| Complete Rust operation | Median, baseline → candidate | Median reduction | p95, baseline → candidate |
| --- | ---: | ---: | ---: |
| 12 MP RGB JPEG fit | 52.65 → 52.69 ms | -0.1% | 54.27 → 53.86 ms |
| 12 MP grayscale JPEG fit | 109.65 → 42.93 ms | 60.9% | 117.75 → 46.89 ms |
| 12 MP RGB PNG fit | 79.14 → 78.74 ms | 0.5% | 89.38 → 84.69 ms |
| 24 MP rotated PNG crop | 150.96 → 128.67 ms | 14.8% | 154.97 → 142.59 ms |
| 24 MP upright PNG crop | 129.53 → 129.79 ms | -0.2% | 137.93 → 137.37 ms |

## What changed

- Grayscale JPEGs no longer undergo a scaled decode that is discarded. The pixel format is checked before scaled decoding.
- 8-bit grayscale images stay single-channel through orientation and SIMD resizing. Only the fitted pixels are expanded to RGB. This avoids expanding and convolving three identical full-resolution channels.
- Full-resolution fallback decoding consumes the existing prepared decoder. PNG, WebP, BMP, and full-resolution JPEG paths avoid constructing a second decoder and copying the compressed input through `load_from_memory`.
- Crops inverse-map the display-space rectangle through EXIF orientation, copy only the selected source pixels, and orient only that region. The full decoded image is released before transforming the crop.
- Source frames and resizer scratch are released before fitted BMP encoding. Full decoding retains the 512 MiB allocation budget; scaled JPEG output now has an explicit 512 MiB decoder buffer limit.

Orientation still precedes fitted convolution: moving it after resizing can change rounding and clipped pixel values. Existing codecs, SIMD dispatch, BMP transport, React components, release portability, ICC fallback, animation/alpha handling, and atomic crop replacement remain in use. No dependency or lockfile was changed.

## TDD evidence and correctness

The user confirmed the public image-serving and crop APIs as test seams. The grayscale performance check failed on the original implementation (`rewrite-red.json`). Early rejection alone improved median latency by 19.9%, so the same 25% check still failed (`rewrite-gray-early-rejection.json`). Single-channel fitting passed the gate (`rewrite-gray-green.json`), followed by the final five-case paired comparison.

The crop performance check also failed before its rewrite (`rewrite-crop-red.json`) and passed afterward (`rewrite-crop-green.json`). Its secondary internal target was a 10% median improvement; it does not replace the primary 25% grayscale publishing gate.

Each benchmark sample compares the full encoded response or crop file byte for byte against a frozen executable compiled from the shipping baseline. The final report contains output SHA-256 hashes, all raw timings, compiler/CPU details, source and Cargo.lock hashes, and fixture hashes. Both executables use the application's locked dependencies and release profile (LTO, one codegen unit). No engine decode model is used.

The new public-interface tests compare grayscale fitting to equal RGB channels across all eight EXIF orientations, odd dimensions, and three viewports. A separate crop test uses literal, worked pixel selections for all eight orientations and both RGBA8 and RGBA16, checking exact pixels, alpha, dimensions, and color depth. Existing tests cover profile preservation, readonly/corrupt inputs, invalid bounds, atomic-save cleanup, stale IDs, registry authorization, animation/alpha fallback, and video playback boundaries.

Validation: **101 Rust tests passed**, including loopback video-server tests; the frontend crop test passed all 2,491 assertions; the frontend production build and changed-file formatting checks passed. Performance assertions live in the explicit release benchmark, not in timing-sensitive ordinary unit tests.

The Tauri production build also passed. Strict Clippy encountered three pre-existing warnings in unchanged `image_sequence.rs` and `settings.rs`: `should_implement_trait`, `if_same_then_else`, and `derivable_impls`. These are outside the image rewrite. The follow-up Clippy check passed while allowing only those baseline warning categories and denying other warnings.

## Reproduction

The frozen executable used for this run is `/tmp/manzar-rewrite-baseline/rewrite_bench`. For a fresh comparison, build the same harness at the old commit and the new checkout:

```sh
# Run from the repository root. Use the same rustc for both builds.
git worktree add --detach /tmp/manzar-rewrite-old da19ceef6e47de3af6d3eee39b7e502fb860158c
cp src-tauri/examples/rewrite_bench.rs /tmp/manzar-rewrite-old/src-tauri/examples/
cargo build --release --locked --manifest-path /tmp/manzar-rewrite-old/src-tauri/Cargo.toml --example rewrite_bench
cargo build --release --locked --manifest-path src-tauri/Cargo.toml --example rewrite_bench
src-tauri/target/release/examples/rewrite_bench prepare /tmp/manzar-rewrite-corpus
python3 benches/compare_rewrite.py \
  /tmp/manzar-rewrite-old/src-tauri/target/release/examples/rewrite_bench \
  src-tauri/target/release/examples/rewrite_bench \
  /tmp/manzar-rewrite-corpus /tmp/manzar-rewrite-results.json --samples 31
# After inspecting the report, delete the copied harness and remove the worktree.
rm /tmp/manzar-rewrite-old/src-tauri/examples/rewrite_bench.rs
git worktree remove /tmp/manzar-rewrite-old
```

The harness creates deterministic grain/gradient fixtures, uses a 1920 × 1440 fit viewport, and crops a 700 × 600 selection from a 6000 × 4000 PNG. Fixture generation, crop-source reset, and output capture are outside the timer. Crop timing includes decode, encode, encoded-size verification, disk sync, and atomic replacement. Two warmups precede 31 samples per variant; process order alternates within each pair. The source files are on a warm filesystem cache. p95 is the nearest-rank percentile. No confidence interval, cold-disk result, whole-app memory result, real-photograph corpus, or cross-platform performance result is claimed.

## Remaining opportunities

Command scheduling/lock shortening, preflight header I/O, cancellation, bounded caching, decoder backend changes, and native GPU presentation remain separate work. RGB scaled JPEG still needs its compressed input for the scaled decoder in addition to the metadata decoder's input. These changes need their own profiles, correctness contracts, and measurements; none is required for the demonstrated grayscale gain.
