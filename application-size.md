# Application size

Measured on Linux x86_64 with Rust 1.90.0, comparing application source at
`d7396fac` with this change. Both builds used the same Cargo.lock, toolchain and
installed frontend dependencies. Figures exclude system libraries and build caches.

| Artifact | Before (bytes) | After (bytes) | Reduction |
| --- | ---: | ---: | ---: |
| Frontend files in `dist` | 5,188,121 | 244,089 | 95.30% / 21.26× |
| Default release executable | 15,024,528 | 7,020,400 | 53.27% / 2.14× |
| Executable with both versions stripped | 13,925,880 | 7,020,400 | 49.59% / 1.98× |
| Tauri Debian installer | 8,475,482 | 2,852,382 | 66.35% / 2.97× |

## Changes

- Move README screenshots outside Vite's public directory. The screenshots remain
  byte-identical; all retained frontend files, including JS and CSS, also match exactly.
- Use typed grayscale/RGB resizing to avoid compiling unused pixel-format kernels.
  Lanczos3 filtering, SIMD dispatch and format support are preserved.
- Disable unused runtime ACL mutation and prune plugin commands excluded by the
  existing static capabilities. All application commands and required window/dialog
  commands remain available.
- Strip release symbols. Arch builds override this so makepkg can extract debug
  symbols before stripping; debug builds retain their normal symbols.

The full application did not reach 10×. Its remaining native framework, codecs and
image processing exceed that budget. A package-only `opt-level="s"` trial increased
the executable from 8,017,304 to 8,093,624 bytes before command pruning, so the normal
optimization level remains unchanged. Panic unwinding and JPEG parallelism remain enabled.

## Verification

The frontend build, 2,491 frontend assertions and all 104 Rust tests pass. The new
resize regression test compares exact bytes with the previous dynamic API for both
pixel types at three output sizes. A separate release comparison checked byte-identical
12 MP RGB JPEG, grayscale JPEG and RGB PNG output across 21 alternating process pairs
per case, after two warmups. Median render times were 54.22 → 50.38 ms,
44.46 → 43.41 ms and 78.46 → 77.81 ms respectively; these measure the Rust fit API,
not webview presentation. Command pruning changes plugin dispatch only.

Build and measure a revision with:

```sh
bun run tauri build --no-bundle -- --locked
wc -c src-tauri/target/release/manzar
bun run tauri bundle --bundles deb --ci
wc -c src-tauri/target/release/bundle/deb/*.deb
```

Windows, macOS and AppImage sizes were not measured.
