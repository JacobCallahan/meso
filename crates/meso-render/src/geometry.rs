/*
 * Geometry generation: convert decoded radar data → screen-space vertices.
 *
 * Both Level 2 and Level 3 data ultimately produce a set of colored triangles
 * (quad pairs) that fill the radar display.  This module handles the
 * coordinate transformation from radar-polar space to screen pixels.
 *
 * This is the Rust port of the core wX JNI functions:
 *   genMercator / genIndex / decode8BitAndGenRadials / colorGen
 */

use meso_data::geo::latlon::LatLon;
use meso_data::radar::color_palette::ColorPalette;
use meso_data::radar::level2::{Level2Data, NUM_RANGE_BINS};
use meso_data::radar::level3::Level3Data;

use crate::viewport::Viewport;

// ── Vertex types ──────────────────────────────────────────────────────────────

/// A 2D screen vertex with associated color.
#[derive(Debug, Clone, Copy)]
pub struct ColorVertex {
    pub x: f32,
    pub y: f32,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Quad strip output: screen-space vertices + colors ready to upload to GPU.
/// Each quad is 4 vertices (CCW winding), indexed as a triangle strip:
///   0─1
///   │/│
///   3─2
#[derive(Debug, Clone, Default)]
pub struct QuadBuffer {
    /// Packed f32 positions: [x0,y0, x1,y1, x2,y2, x3,y3, ...] per quad.
    pub positions: Vec<f32>,
    /// Packed u8 RGB colors: [r,g,b, r,g,b, r,g,b, r,g,b, ...] per vertex.
    pub colors: Vec<u8>,
    /// Number of quads (4 vertices each).
    pub quad_count: usize,
}

impl QuadBuffer {
    pub fn new() -> Self {
        QuadBuffer {
            positions: Vec::new(),
            colors: Vec::new(),
            quad_count: 0,
        }
    }

    pub fn with_capacity(quads: usize) -> Self {
        QuadBuffer {
            positions: Vec::with_capacity(quads * 8),
            colors: Vec::with_capacity(quads * 12),
            quad_count: 0,
        }
    }

    #[inline]
    fn push_quad(
        &mut self,
        (x0, y0): (f32, f32),
        (x1, y1): (f32, f32),
        (x2, y2): (f32, f32),
        (x3, y3): (f32, f32),
        (r, g, b): (u8, u8, u8),
    ) {
        self.positions
            .extend_from_slice(&[x0, y0, x1, y1, x2, y2, x3, y3]);
        for _ in 0..4 {
            self.colors.push(r);
            self.colors.push(g);
            self.colors.push(b);
        }
        self.quad_count += 1;
    }
}

// ── Level 2 geometry ──────────────────────────────────────────────────────────

/// "Black hole" radius at radar center (in km) — matches wX's 4.0 km for REF.
const BLACK_HOLE_REF_KM: f64 = 4.0;
/// Smaller black hole for velocity products.
const BLACK_HOLE_VEL_KM: f64 = 1.0;

/// Generate screen-space quad geometry from Level 2 decoded data.
///
/// Produces one quad per contiguous run of equal gate values per radial,
/// exactly matching wX's rendering approach.
pub fn level2_to_quads(
    data: &Level2Data,
    palette: &ColorPalette,
    viewport: &Viewport,
    is_velocity: bool,
) -> QuadBuffer {
    let num_radials = data.azimuths.len();
    if num_radials == 0 {
        return QuadBuffer::new();
    }

    let black_hole = if is_velocity {
        BLACK_HOLE_VEL_KM
    } else {
        BLACK_HOLE_REF_KM
    };
    let bin_size = data.bin_size_km as f64;

    // Estimate capacity: ~720 radials × ~400 runs each
    let mut buf = QuadBuffer::with_capacity(num_radials * 400);

    for r in 0..num_radials {
        let az0 = data.azimuths[r] as f64;
        let az1 = data.azimuths[(r + 1) % num_radials] as f64;

        let cos0 = az0.to_radians().cos();
        let sin0 = az0.to_radians().sin();
        let cos1 = az1.to_radians().cos();
        let sin1 = az1.to_radians().sin();

        let bins = &data.bins[r * NUM_RANGE_BINS..(r + 1) * NUM_RANGE_BINS];

        // Run-length encode identical gate values
        let mut run_start_km = black_hole;
        let mut run_level = bins[0];
        let mut run_count = 0usize;

        let bin_start_index = (black_hole / bin_size).ceil() as usize;

        for (b, &level) in bins.iter().enumerate().skip(bin_start_index) {
            if level == run_level {
                run_count += 1;
            } else {
                if run_count > 0 && run_level != 0 {
                    let r0 = run_start_km;
                    let r1 = r0 + run_count as f64 * bin_size;
                    let color = palette.color(run_level);
                    emit_quad_km(&mut buf, r0, r1, cos0, sin0, cos1, sin1, color, viewport);
                }
                run_level = level;
                run_start_km = b as f64 * bin_size;
                run_count = 1;
            }
        }
        if run_count > 0 && run_level != 0 {
            let r0 = run_start_km;
            let r1 = r0 + run_count as f64 * bin_size;
            let color = palette.color(run_level);
            emit_quad_km(&mut buf, r0, r1, cos0, sin0, cos1, sin1, color, viewport);
        }
    }

    buf
}

/// Generate screen-space quad geometry from Level 3 decoded data.
pub fn level3_to_quads(
    data: &Level3Data,
    palette: &ColorPalette,
    viewport: &Viewport,
    is_velocity: bool,
) -> QuadBuffer {
    if data.is_raster {
        return level3_raster_to_quads(data, palette, viewport);
    }

    let num_radials = data.num_radials;
    if num_radials == 0 {
        return QuadBuffer::new();
    }

    let black_hole = if is_velocity {
        BLACK_HOLE_VEL_KM
    } else {
        BLACK_HOLE_REF_KM
    };
    let bin_size = data.bin_size_km as f64;

    let mut buf = QuadBuffer::with_capacity(num_radials * 300);

    for r in 0..num_radials {
        let az0 = data.azimuths[r] as f64;
        let az1 = data.azimuths[(r + 1) % num_radials] as f64;

        let cos0 = az0.to_radians().cos();
        let sin0 = az0.to_radians().sin();
        let cos1 = az1.to_radians().cos();
        let sin1 = az1.to_radians().sin();

        let bins = &data.bins[r * data.num_range_bins..(r + 1) * data.num_range_bins];
        let num_bins = data.num_range_bins;

        let bin_start_index = (black_hole / bin_size).ceil() as usize;
        let mut run_start_km = bin_start_index as f64 * bin_size;
        let mut run_level = if bin_start_index < num_bins {
            bins[bin_start_index]
        } else {
            0
        };
        let mut run_count = 0usize;

        for (b, &level) in bins.iter().enumerate().skip(bin_start_index) {
            if level == run_level {
                run_count += 1;
            } else {
                if run_count > 0 && run_level != 0 {
                    let r0 = run_start_km;
                    let r1 = r0 + run_count as f64 * bin_size;
                    let color = palette.color(run_level);
                    emit_quad_km(&mut buf, r0, r1, cos0, sin0, cos1, sin1, color, viewport);
                }
                run_level = level;
                run_start_km = b as f64 * bin_size;
                run_count = 1;
            }
        }
        if run_count > 0 && run_level != 0 {
            let r0 = run_start_km;
            let r1 = r0 + run_count as f64 * bin_size;
            let color = palette.color(run_level);
            emit_quad_km(&mut buf, r0, r1, cos0, sin0, cos1, sin1, color, viewport);
        }
    }

    buf
}

/// Generate screen-space quads for raster Level 3 products (packet 0xBA07).
fn level3_raster_to_quads(
    data: &Level3Data,
    palette: &ColorPalette,
    viewport: &Viewport,
) -> QuadBuffer {
    let rows = data.num_radials;
    let cols = data.num_range_bins;
    if rows == 0 || cols == 0 || data.bins.len() < rows * cols {
        return QuadBuffer::new();
    }

    let cell_km = data.bin_size_km as f64;
    let half_rows = rows as f64 / 2.0;
    let half_cols = cols as f64 / 2.0;
    let mut buf = QuadBuffer::with_capacity(rows * cols / 2);

    for row in 0..rows {
        for col in 0..cols {
            let level = data.bins[row * cols + col];
            if level == 0 {
                continue;
            }

            let x0_km = (col as f64 - half_cols) * cell_km;
            let y0_km = (row as f64 - half_rows) * -cell_km;
            let x1_km = x0_km;
            let y1_km = (row as f64 + 1.0 - half_rows) * -cell_km;
            let x2_km = (col as f64 + 1.0 - half_cols) * cell_km;
            let y2_km = y1_km;
            let x3_km = x2_km;
            let y3_km = y0_km;

            let (x0, y0) = viewport.radar_km_to_screen(x0_km, y0_km);
            let (x1, y1) = viewport.radar_km_to_screen(x1_km, y1_km);
            let (x2, y2) = viewport.radar_km_to_screen(x2_km, y2_km);
            let (x3, y3) = viewport.radar_km_to_screen(x3_km, y3_km);
            let color = palette.color(level);

            buf.push_quad(
                (x0 as f32, y0 as f32),
                (x1 as f32, y1 as f32),
                (x2 as f32, y2 as f32),
                (x3 as f32, y3 as f32),
                color,
            );
        }
    }

    buf
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn emit_quad_km(
    buf: &mut QuadBuffer,
    r0: f64,
    r1: f64,
    cos0: f64,
    sin0: f64,
    cos1: f64,
    sin1: f64,
    color: (u8, u8, u8),
    vp: &Viewport,
) {
    // Azimuth convention: angle 0 = North (up), increasing clockwise.
    // In screen coords: x = East, y = Down.
    // radar x_km = r * sin(az),  y_km = r * cos(az)
    let to_screen = |r: f64, cos: f64, sin: f64| -> (f32, f32) {
        let x_km = r * sin;
        let y_km = r * cos;
        let (px, py) = vp.radar_km_to_screen(x_km, y_km);
        (px as f32, py as f32)
    };

    let v0 = to_screen(r0, cos1, sin1); // near, next azimuth
    let v1 = to_screen(r1, cos1, sin1); // far, next azimuth
    let v2 = to_screen(r1, cos0, sin0); // far, current azimuth
    let v3 = to_screen(r0, cos0, sin0); // near, current azimuth

    buf.push_quad(v0, v1, v2, v3, color);
}

// ── Warning polygon geometry ──────────────────────────────────────────────────

/// Convert a warning polygon (lat/lon list) to screen-space line segments.
pub fn warning_to_lines(polygon: &[LatLon], viewport: &Viewport) -> Vec<(f32, f32)> {
    polygon
        .iter()
        .map(|p| {
            let (x, y) = viewport.latlon_to_screen(p);
            (x as f32, y as f32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_buffer_new_is_empty() {
        let buf = QuadBuffer::new();
        assert_eq!(buf.quad_count, 0);
        assert!(buf.positions.is_empty());
        assert!(buf.colors.is_empty());
    }

    #[test]
    fn quad_buffer_with_capacity_starts_empty_but_preallocated() {
        let buf = QuadBuffer::with_capacity(100);
        assert_eq!(buf.quad_count, 0);
        // Capacity should be at least 100*8 = 800 for positions.
        assert!(buf.positions.capacity() >= 800);
        // Capacity should be at least 100*12 = 1200 for colors.
        assert!(buf.colors.capacity() >= 1200);
    }

    #[test]
    fn quad_buffer_new_starts_with_zero_quads() {
        let buf = QuadBuffer::new();
        assert_eq!(buf.quad_count, 0);
        assert!(buf.positions.is_empty());
        assert!(buf.colors.is_empty());
    }
}
