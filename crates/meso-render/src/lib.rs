/*!
 * wx-render: radar and satellite rendering engine for Meso.
 *
 * Provides two rendering backends:
 * - `wgpu` (primary): GPU-accelerated via WebGPU/OpenGL (Mesa), used when a
 *   compatible GPU adapter is available.
 * - `cairo` (fallback): CPU software rasterization via cairo-rs, for
 *   headless/software-only systems.
 *
 * Public API surface:
 * - `RadarRenderer` — full rendering pipeline for L2/L3 radial products
 * - `RadarFrame` — decoded + rendered frame ready for display
 * - `OverlayLayer` — vector overlay (warnings, county lines, etc.)
 * - `Viewport` — lat/lon bounds and pixel dimensions for the view
 */

pub mod cairo_render;
pub mod frame;
pub mod geometry;
pub mod overlay;
pub mod viewport;
pub mod wgpu_render;

pub use frame::{RadarFrame, RenderedImage};
pub use overlay::{OverlayLayer, OverlaySet};
pub use viewport::Viewport;
