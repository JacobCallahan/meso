/*
 * Rendered output types: the result of a render pass.
 *
 * A `RenderedImage` is an RGBA pixel buffer (width × height × 4 bytes)
 * ready to be displayed in a GtkGLArea via a texture upload, or painted
 * directly via cairo.
 */

use crate::viewport::Viewport;

/// A rendered RGBA image buffer.
#[derive(Debug, Clone)]
pub struct RenderedImage {
    pub width: u32,
    pub height: u32,
    /// Raw RGBA bytes, row-major, top-left origin.
    pub data: Vec<u8>,
}

impl RenderedImage {
    pub fn new(width: u32, height: u32) -> Self {
        RenderedImage {
            width,
            height,
            data: vec![0u8; (width * height * 4) as usize],
        }
    }

    /// Set a single pixel (RGBA).
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
        if x < self.width && y < self.height {
            let idx = ((y * self.width + x) * 4) as usize;
            self.data[idx] = r;
            self.data[idx + 1] = g;
            self.data[idx + 2] = b;
            self.data[idx + 3] = a;
        }
    }

    /// Alpha-blend a pixel onto this image.
    #[inline]
    pub fn blend_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
        if x < self.width && y < self.height && a > 0 {
            let idx = ((y * self.width + x) * 4) as usize;
            let alpha = a as f32 / 255.0;
            let inv = 1.0 - alpha;
            self.data[idx] = (r as f32 * alpha + self.data[idx] as f32 * inv) as u8;
            self.data[idx + 1] = (g as f32 * alpha + self.data[idx + 1] as f32 * inv) as u8;
            self.data[idx + 2] = (b as f32 * alpha + self.data[idx + 2] as f32 * inv) as u8;
            self.data[idx + 3] = 255;
        }
    }
}

// ── RadarFrame ────────────────────────────────────────────────────────────────

/// Metadata and rendered output for a single radar scan.
#[derive(Debug, Clone)]
pub struct RadarFrame {
    /// Which radar site this frame is from.
    pub site_id: String,
    /// Product name (e.g. "N0Q", "N0U", "Level2-REF").
    pub product: String,
    /// Scan time as ISO 8601 string.
    pub scan_time: String,
    /// Volume Coverage Pattern number.
    pub vcp: u16,
    /// The rendered image (may be None if not yet rendered).
    pub image: Option<RenderedImage>,
    /// The viewport used for this render.
    pub viewport: Viewport,
}

impl RadarFrame {
    pub fn new(
        site_id: String,
        product: String,
        scan_time: String,
        vcp: u16,
        viewport: Viewport,
    ) -> Self {
        RadarFrame {
            site_id,
            product,
            scan_time,
            vcp,
            image: None,
            viewport,
        }
    }
}
