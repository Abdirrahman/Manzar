# Rust rendering performance review

Reviewed 5 October 2026, application commit `da19ceef6e47de3af6d3eee39b7e502fb860158c`.

## Verdict

Yes: focused rewrites of the existing Rust pipeline can remove substantial unnecessary work. The strongest targets are its preparation/decoding flow, command scheduling, and the order of image transformations. Reimplementing codecs or SIMD resizing is not the first step.

This review inspected the actual sources of the pinned dependencies and ran new optimized CPU-stage measurements. It does not claim measured whole-application speedups. At the time of this review, runtime source and dependencies were unchanged; its additions were this report, an isolated benchmark binary, and its JSON results. The subsequent implementation and complete-operation comparisons are recorded in [Rust rewrite results](rust-rewrite-results.md).

## Findings, ordered by implementation priority

### 1. JPEG preparation repeatedly reads the full compressed file

Locations: `core/metadata_preflight.rs:87`, `core/crop.rs:47`, `core/render.rs:125`, `core/render.rs:115`.

The render probe's “header only” comment is incorrect for the pinned JPEG implementation. `image::codecs::jpeg::JpegDecoder::new` calls `read_to_end` into an owned `Vec`. Construction parses headers; asking for orientation and ICC then creates new header parsers over the same bytes. [Pinned decoder source](https://docs.rs/image/0.25.10/src/image/codecs/jpeg/decoder.rs.html)

A cold JPEG open can therefore read the whole compressed file in preflight, read it again in the render probe, and read it again for actual decoding. If fitting is declined after probing, the original-file response reads it again. The unscaled `image::load_from_memory` JPEG path also copies the borrowed compressed buffer into the decoder's owned input. Cached preflight avoids the first read on unchanged revisits, not the render probe/read pair.

The measurement confirmed that probing a 1,839,256-byte generated JPEG consumed all 1,839,256 bytes. A dimensions-only `jpeg-decoder::read_info` consumed 39 bytes, but that is **not equivalent metadata probing**: it does not establish the same orientation/profile information. Do not replace the probe with it and silently lose fidelity.

Rewrite the flow around one prepared image: known format, checked dimensions, orientation/profile, source generation, and a decoder or shared compressed buffer consumed by decoding. Reuse the already-created PNG/WebP/BMP decoder instead of discarding and reconstructing it. For JPEG, start with a single bounded source read and shared borrowing; investigate genuinely bounded header probing separately. The size warning should be decided before a decoder constructor reads an oversized file: preflight currently checks the 200 MiB threshold **after** construction.

This is the highest-confidence reduction in redundant I/O and compressed-buffer copying. Its latency benefit depends on file size and storage/page-cache behavior, which the CPU-only harness does not measure.

### 2. Slow file preparation runs in synchronous commands under both locks

Locations: `commands.rs:32`, `commands.rs:55`, `commands.rs:66`, `commands.rs:149`, `core/viewer_session.rs:169`.

Opening/scanning a folder, sorting it, and generating preflight metadata run in synchronous Tauri commands. Unmarked synchronous commands execute on the main thread. Combined with the whole-file JPEG read above, a large image can block the platform event loop before the asynchronous render worker is involved. [Tauri command execution](https://v2.tauri.app/develop/calling-rust/#async-commands)

`with_session_and_registry` holds both mutexes throughout the operation. Cropping uses `spawn_blocking`, but still holds both locks through decoding, rotation, encoding, verification, and disk sync. `spawn_blocking` alone does not fix lock contention.

Use owned inputs with `async` commands and `spawn_blocking` for expensive filesystem/CPU work. Split operations into brief state capture, slow preparation outside the locks, and a generation-checked commit. Serialize destructive mutations separately and retain stale-image checks, source-change checks, and atomic replacement. Simply releasing locks around a destructive save without a transaction/generation rule is not a safe rewrite.

This improves responsiveness and reduces blocked protocol/command work; no whole-app stall timing was taken in this review.

### 3. Grayscale and CMYK JPEGs are decoded once just to be rejected

Location: `core/render.rs:173`, especially the pixel-format test at line 189.

The scaled decoder currently reads headers, scales, fully decodes, and **then** checks for RGB24. On grayscale/CMYK input it throws the pixels away, and `decode_to_cover` runs the general decoder as well. `info().pixel_format` is already available after `read_info`.

Move the eligibility test immediately after `read_info`, before scaling/decoding. Preserve the existing general decoder fallback. A future scaled grayscale path could resize a single channel before expanding to RGB, but early rejection is the smaller first change.

The new grayscale benchmark measures this existing double-decode pattern against early rejection. It confirms a useful improvement without changing the final general-decoder output. CMYK savings were not separately timed.

### 4. Cropping rotates pixels that will immediately be discarded

Location: `core/crop.rs:110`.

For a 90°/270° orientation, `apply_orientation` allocates a rotated copy of the full decoded image. `crop_imm` then copies just the crop. When the selection is small, most rotation work is wasted.

Map the selected display-space crop back into source-space coordinates, crop that rectangle, and orient only the cropped pixels. A 90° clockwise example for source height `H` maps display `(x,y,w,h)` to source `(y,H-x-w,h,w)`.

The benchmark checks exact pixel equality for a 700 × 600 selection from a rotated 12 MP image. This transformation can preserve crop pixels exactly; production tests must cover all eight EXIF orientations, bounds, alpha, and bit depths. Source decoding and final encoding still remain, so the geometry-stage gain is not a crop-save speedup of the same ratio.

### 5. Display fitting rotates the source before shrinking it

Location: `core/render.rs:117`.

Resize in source axes to the already-computed `required` dimensions, then orient the much smaller fitted image. This substantially reduces rotation work and its temporary buffer for large PNG/BMP/WebP or full-resolution JPEG frames.

The full-resolution 12 MP rotation/resize measurement demonstrates a large geometry-stage gain. The already-half-scaled JPEG measurement demonstrates a much smaller gain: its source is already close to the fitted size.

This is **not byte-equivalent resizing**. Reordering separable integer convolution passes changed some RGB channels by up to 8/255 on the generated fixture. Geometry is equivalent, but rounding/clipping can differ. Require visual/reference-quality validation before adoption; if identical fitted pixels are required, retain the current order or investigate a resampler that preserves its arithmetic order.

### 6. Shadowing keeps the original decoded frame alive during encoding

Location: `core/render.rs:115–120`.

`let frame = resize_rgb(&frame, fitted)?` borrows the old frame, then shadows its binding. The old owned buffer is still dropped at the end of its scope, rather than immediately after its last use. The compressed `bytes` vector also remains alive during resizing and encoding. Rust's destructor scopes, unlike borrow lifetimes, are observable. [Rust destructor rules](https://doc.rust-lang.org/reference/destructors.html#drop-scopes)

Use distinct names and explicit ownership release:

```rust
let decoded = decode_to_cover(&bytes, width, height, required)?;
drop(bytes);
let resized = resize_rgb(&decoded, required)?;
drop(decoded);
let fitted = apply_orientation(resized, orientation);
encode_bmp(&fitted)
```

This sketch includes the separately gated orientation reorder. Early dropping can also be implemented while preserving the current transform order. It lowers live memory during later stages; it does not necessarily reduce the maximum allocation peak if the resize scratch buffer dominates earlier.

### 7. Supersession is checked only before the blocking job

Location: `lib.rs:178`.

If navigation or resize supersedes an image while it is decoding, the old job still rotates, resizes, encodes, and returns its response before releasing the single-decode gate. Skipping queued jobs protects against a backlog, but does not remove this obsolete work.

Check generation between read/decode/transform/encode stages and before publishing. Abort obsolete fitted work at those boundaries and release the permit. This is cooperative cancellation: it cannot interrupt a synchronous decoder in the middle of its call. Keep active-image generations distinct from any future prefetch request generations.

### 8. BMP encoding is a narrow, measurable optimization opportunity

Location: `core/render.rs:232`.

The dependency encoder reverses row order and issues a tiny `write_all` for each RGB-to-BGR pixel. A pre-sized destination with direct slice writes can reduce encoder overhead. The benchmark prototype validates byte-identical output against `BmpEncoder`, including odd-width padding and successful decode back to the original pixels.

However, the absolute saving is small compared with decoding. Reserving `Vec` capacity alone did not produce a meaningful gain. Prefer the existing encoder unless complete-pipeline profiling shows encoding matters, or submit a focused upstream encoder optimization. A production custom writer needs checked size arithmetic and a narrow RGB8 contract; this review's fixed-size prototype is not production replacement code.

### 9. Reusing resize scratch helps modestly and retains memory

Location: `core/render.rs:219`.

Each resize creates a new `Resizer` and destination. `Resizer` supports reusable intermediate buffers and exposes their size/reset. [Resizer API](https://docs.rs/fast_image_resize/6.1.0/fast_image_resize/struct.Resizer.html)

An owned worker context could retain one resizer and suitable destination buffers. The benchmark found only a small resize-stage gain and retained about 17.3 MB of scratch for its test geometry. Budget and shrink this retained memory; do not add a generic pool to save a fraction of a millisecond without a workload that benefits.

## Larger decoder replacement worth investigating

The current rule uses `jpeg-decoder` for every eligible 1/2, 1/4, or 1/8 decode. Reduced pixel count does not guarantee lower latency: the benchmark's RGB half-scale decode plus resize was slightly slower than full `image`/zune decoding plus SIMD resize, while using less decoded memory. Do not trade that memory benefit away merely because one fixture favors full decoding.

Benchmark a scaled libjpeg-turbo backend through a Rust binding, which supports scaled decompression into caller-provided buffers. This is a distinct backend experiment, not a recommendation to write a new JPEG codec. Compare baseline/progressive JPEG, all scales, color behavior, decoded memory, and deployment/build costs before adding it to the application. It was not benchmarked here. [Rust TurboJPEG scaling API](https://docs.rs/turbojpeg/latest/turbojpeg/struct.Decompressor.html#method.set_scaling_factor)

Calling zune directly can also avoid the `image` adapter's compressed-input copy in a prepared-buffer design. That qualifies the previous review's recommendation: it does not introduce a faster decoding algorithm, but eliminating wrapper work can still help. Measure the overhead separately.

## Fresh measurement results

| Isolated comparison | Current/model baseline | Candidate | Stage ratio |
| --- | ---: | ---: | ---: |
| Reserve BMP capacity only | 1.54 ms | 1.54 ms | 1.00× |
| Direct RGB8 BMP writer versus reserved standard encoder | 1.54 ms | 0.97 ms | 1.59× |
| Reuse resizer scratch and destination | 8.75 ms | 8.36 ms | 1.05× |
| Resize before 90° rotation: full 12 MP input | 25.32 ms | 11.06 ms | 2.29× |
| Full image/zune decode + resize versus half-scale jpeg-decoder + resize | 37.94 ms | 35.55 ms | 1.07× |
| Resize before 90° rotation: already-scaled JPEG frame | 7.12 ms | 6.68 ms | 1.07× |
| Crop before 90° rotation: 700 × 600 selection | 16.23 ms | 0.75 ms | 21.75× |
| Reject grayscale before scaled decode attempt | 69.58 ms | 58.62 ms | 1.19× |

Measurements use a generated 4000 × 3000 image and a 1920 × 1440 fitted output, on x86_64 / AMD Ryzen AI MAX+ 395, Rust 1.90.0, optimized release build with LTO. One warmup and 15 sequential samples per variant; numbers are medians. Variant order is not randomized, so small differences can include clock/cache/order effects. These are CPU stages, not disk latency, webview timing, or end-to-end image presentation. Output-quality limitations above apply.

Benchmark source: `benches/render-bench/src/bin/rust_review.rs`. Results: `benches/render-bench/rust-review-report.json`. The binary includes executable byte/pixel-equivalence checks for the BMP prototype and the source-coordinate crop mapping.

Normal reproduction with the benchmark crate's dependencies available:

```sh
cargo run --release --locked --manifest-path benches/render-bench/Cargo.toml \
  --bin rust_review -- benches/render-bench/rust-review-report.json
```

The existing standalone benchmark lockfile could not resolve offline because its pinned `zlib-rs` 0.6.7 was absent from the local cache. This run used a temporary manifest at `/tmp/manzar-rust-review/Cargo.toml`, the same benchmark source, and exact `image` 0.25.10 / `fast_image_resize` 6.1.0 / `jpeg-decoder` 0.3.2 versions. Some cached transitive dependencies differ from the application/benchmark lockfiles. Neither lockfile was changed. This limits comparisons against historical benchmark reports; comparisons within this run use the same compiled libraries.

## Implementation order

1. **Remove confirmed wasted work:** early pixel-format rejection, explicit buffer release, correctly configured per-image/output limits, and stage timing. Validate fallback output and allocation lifetimes.
2. **Rewrite preparation and command boundaries:** prepare/read once, check file-size limits before whole-file probing, move slow commands to blocking workers, shorten registry/session lock scope, and check generations before commits. Validate metadata, stale mutations, and UI responsiveness.
3. **Optimize transformation order:** inverse-map crops and rotate only the crop; evaluate resize-before-orientation separately against the required fitted quality. Test all EXIF orientations and formats.
4. **Reuse work where profiles show value:** fitted-response caching from the original plan, cooperative cancellation, then bounded resizer/destination reuse. Include scratch and cached frames in the same memory accounting.
5. **Evaluate the larger backend change:** scaled libjpeg-turbo versus current decoders. Consider a custom BMP writer only if encoding remains significant. Keep the GPU presentation investigation separate from these CPU changes.

Each change should report complete Rust pipeline latency and actual viewer p95 latency/memory, rather than multiplying its isolated stage speedup into a claimed app gain. Do not loosen decode/output limits, overwrite safeguards, profile handling, or actual-size detail to achieve a benchmark number. The single-decode gate limits concurrency, not the memory consumption of one pathological image; retain explicit budgets when refactoring.

## What already looks appropriate

The renderer borrows source pixels through `ImageRef`; `into_rgb8` consumes an existing RGB8 image without a pixel copy; registry locks are released before protocol decoding; runtime SIMD dispatch is already present. Natural sorting is allocation-free in its hot comparison path, and folder scanning already avoids per-entry canonicalization. Release builds already use cross-crate LTO and one codegen unit.

There is no evidence to justify replacing all `Box<dyn ImageDecoder>` uses, changing `Arc`/mutex primitives wholesale, enabling unrestricted parallel decodes, hand-writing SIMD, or enabling `target-cpu=native` for distributed binaries. The useful rewrites change **how often work is done, how much data it touches, and how long buffers/locks are held**.
