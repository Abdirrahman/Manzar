# Application size

Measured on Linux x86_64 with Rust 1.90.0, comparing application source at
`d7396fac` with this change. Both builds used the same Rust Cargo.lock and toolchain.
Figures exclude system libraries and build caches.

| Artifact | Before (bytes) | After (bytes) | Reduction |
| --- | ---: | ---: | ---: |
| Frontend files in `dist` | 5,188,121 | 228,425 | 95.60% / 22.71× |
| Default release executable | 15,024,528 | 5,488,304 | 63.47% / 2.74× |
| Executable with both versions stripped | 13,925,880 | 5,488,304 | 60.59% / 2.54× |
| Tauri Debian installer | 8,475,482 | 2,315,450 | 72.68% / 3.66× |

The second pass starts at `97b77cb7`: 7,020,400-byte executable,
2,852,382-byte installer and 244,089 frontend bytes. It saves another 21.82%,
18.82% and 6.42% respectively.

## Changes

- Move README screenshots outside Vite's public directory, preserving their bytes.
- Use typed grayscale/RGB resizing to avoid compiling unused pixel-format kernels.
  Lanczos3 filtering, SIMD dispatch and format support are preserved.
- Replace generic image geometry/conversion with validated native-endian byte buffers.
  This removes processing for unused layouts while preserving encoded output, ICC,
  EXIF, transparency, memory limits and verified atomic crop replacement.
- Disable unused runtime ACL mutation and prune plugin commands. Replace `core:default`
  with the six core commands actually needed, plus dialog open. All ten application
  commands remain available; debug devtools and native window gestures are retained.
- Invoke the three window actions directly and omit the unused module-preload polyfill.
  The application has a single JavaScript bundle with no dynamic imports.
- Strip release symbols. Arch builds override this so makepkg can extract debug
  symbols before stripping; debug builds retain their normal symbols.
- Optimize ten UI/runtime crates for size; image processing stays at level 3.

The full application did not reach 10×. A package-only `opt-level="s"` trial grew
the binary, and safe linker folding saved only eight bytes; neither is enabled.
Panic unwinding, JPEG parallelism, X11/DBus compatibility and supported codecs remain.

## Verification

The frontend build, 2,491 frontend assertions and all 108 Rust tests pass. Vite 7.3.7
clears the dependency audit. New tests compare the previous image implementation's
geometry, fitted pixels and final encoded crop bytes across supported layouts and
all eight EXIF orientations. The 16-to-8-bit conversion checks every possible sample.
Independent review found no actionable security or correctness defects.

Second-pass release comparisons checked identical output across 21 measured alternating
process pairs per case, after two warmups. The fit cases use 12 MP sources; crop cases
use 24 MP PNGs. Timings cover the Rust API, excluding fixture/reset/output capture
and webview presentation:

| Operation | Previous median (ms) | Current median (ms) |
| --- | ---: | ---: |
| Fit RGB JPEG | 50.56 | 53.64 |
| Fit grayscale JPEG | 42.87 | 32.75 |
| Fit RGB PNG | 77.01 | 79.26 |
| Crop rotated PNG | 131.23 | 134.11 |
| Crop upright PNG | 131.25 | 133.13 |

This is a size/performance tradeoff: grayscale improves 23.6%, while other measured
medians increase 1.4–6.1%. Generated ACLs and packaged executable contents were checked.

Build and measure a revision with:

```sh
bun run tauri build --no-bundle -- --locked
wc -c src-tauri/target/release/manzar
bun run tauri bundle --bundles deb --ci
wc -c src-tauri/target/release/bundle/deb/*.deb
```

Windows, macOS and AppImage sizes were not measured. Native GUI interactions were
reviewed in source but were not exercised in this environment.
