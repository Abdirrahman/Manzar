//! Measurement rig for Manzar's media pipeline.
//!
//! Every figure in `docs/rust-renderer-plan.html` comes from here. The rig
//! exists to answer one question with numbers instead of intuition: what does
//! the viewer actually pay, per image, for handing whole files to a webview and
//! letting it decode, scale and re-raster them?
//!
//! The comparison is always between two paths for the same visible result:
//!
//! * **`webview`** — what ships today. The whole file is read into memory and
//!   handed over the custom protocol; the engine decodes every pixel of it and
//!   re-rasters the full surface on each zoom step.
//! * **`native`** — what the plan proposes. Rust decodes at the smallest DCT
//!   scale that still covers the viewport, resizes with SIMD, and hands over
//!   only the pixels that will actually be lit.

pub mod corpus;
pub mod memory;
pub mod paths;
pub mod timing;
pub mod video;

/// The window a desktop viewer is realistically asked to fill. Every "native"
/// path is allowed to produce exactly this many pixels and no more, because
/// that is all a display can show.
pub const VIEWPORT: (u32, u32) = (2560, 1440);
