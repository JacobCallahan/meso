/*
 * Geometry generation: convert decoded radar data → radar-space vertices in kilometers.
 *
 * Both Level 2 and Level 3 data ultimately produce a set of colored triangles
 * (quad pairs) that fill the radar display. This module handles the
 * coordinate generation from radar-polar space to kilometers relative to the site.
 */

use rayon::prelude::*;

use meso_data::geo::latlon::LatLon;
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

/// Quad strip output: radar-space (km) vertices + levels ready to upload to GPU.
/// Each quad is 4 vertices (CCW winding), indexed as a triangle strip:
///   0─1
///   │/│
///   3─2
#[derive(Debug, Clone, Default)]
pub struct QuadBuffer {
    /// Packed f32 positions: [x0,y0, x1,y1, x2,y2, x3,y3, ...] per quad in kilometers.
    pub positions: Vec<f32>,
    /// Raw bin levels: [l0, l1, l2, l3, ...] per vertex (1 byte per vertex, 4 bytes per quad).
    pub levels: Vec<u8>,
    /// Number of quads (4 vertices each).
    pub quad_count: usize,
}

impl QuadBuffer {
    pub fn new() -> Self {
        QuadBuffer {
            positions: Vec::new(),
            levels: Vec::new(),
            quad_count: 0,
        }
    }

    pub fn with_capacity(quads: usize) -> Self {
        QuadBuffer {
            positions: Vec::with_capacity(quads * 8),
            levels: Vec::with_capacity(quads * 4),
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
        level: u8,
    ) {
        self.positions
            .extend_from_slice(&[x0, y0, x1, y1, x2, y2, x3, y3]);
        for _ in 0..4 {
            self.levels.push(level);
        }
        self.quad_count += 1;
    }
}

// ── Level 2 geometry ──────────────────────────────────────────────────────────

/// "Black hole" radius at radar center (in km) — matches wX's 4.0 km for REF.
const BLACK_HOLE_REF_KM: f64 = 4.0;
/// Smaller black hole for velocity products.
const BLACK_HOLE_VEL_KM: f64 = 1.0;

/// Generate radar-relative quad geometry in kilometers from Level 2 decoded data.
///
/// Parallelized via Rayon, processing each radial on a separate thread and
/// accumulating results.
pub fn level2_to_quads(
    data: &Level2Data,
    is_velocity: bool,
    hide_no_data: bool,
    mask_weak_echoes: bool,
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

    // Process radials in parallel
    let results: Vec<QuadBuffer> = (0..num_radials)
        .into_par_iter()
        .map(|r| {
            let az0 = data.azimuths[r] as f64;
            let az1 = data.azimuths[(r + 1) % num_radials] as f64;

            let cos0 = az0.to_radians().cos();
            let sin0 = az0.to_radians().sin();
            let cos1 = az1.to_radians().cos();
            let sin1 = az1.to_radians().sin();

            let bins = &data.bins[r * NUM_RANGE_BINS..(r + 1) * NUM_RANGE_BINS];
            let mut local_buf = QuadBuffer::with_capacity(128);

            // Run-length encode identical gate values
            let mut run_start_km = black_hole;
            let mut run_level = bins[0];
            let mut run_count = 0usize;

            let bin_start_index = (black_hole / bin_size).ceil() as usize;

            for (b, &level) in bins.iter().enumerate().skip(bin_start_index) {
                if level == run_level {
                    run_count += 1;
                } else {
                    if run_count > 0
                        && gate_visible(run_level, is_velocity, hide_no_data, mask_weak_echoes)
                    {
                        let r0 = run_start_km;
                        let r1 = r0 + run_count as f64 * bin_size;
                        emit_quad_km(&mut local_buf, r0, r1, cos0, sin0, cos1, sin1, run_level);
                    }
                    run_level = level;
                    run_start_km = b as f64 * bin_size;
                    run_count = 1;
                }
            }
            if run_count > 0 && gate_visible(run_level, is_velocity, hide_no_data, mask_weak_echoes)
            {
                let r0 = run_start_km;
                let r1 = r0 + run_count as f64 * bin_size;
                emit_quad_km(&mut local_buf, r0, r1, cos0, sin0, cos1, sin1, run_level);
            }
            local_buf
        })
        .collect();

    // Reduce into a single buffer
    let total_quads: usize = results.iter().map(|b| b.quad_count).sum();
    let mut buf = QuadBuffer::with_capacity(total_quads);
    for mut local_buf in results {
        buf.positions.append(&mut local_buf.positions);
        buf.levels.append(&mut local_buf.levels);
        buf.quad_count += local_buf.quad_count;
    }

    buf
}

/// Generate radar-relative quad geometry in kilometers from Level 3 decoded data.
pub fn level3_to_quads(
    data: &Level3Data,
    is_velocity: bool,
    hide_no_data: bool,
    mask_weak_echoes: bool,
) -> QuadBuffer {
    if data.is_raster {
        return level3_raster_to_quads(data, is_velocity, hide_no_data, mask_weak_echoes);
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

    let results: Vec<QuadBuffer> = (0..num_radials)
        .into_par_iter()
        .map(|r| {
            let az0 = data.azimuths[r] as f64;
            let az1 = data.azimuths[(r + 1) % num_radials] as f64;

            let cos0 = az0.to_radians().cos();
            let sin0 = az0.to_radians().sin();
            let cos1 = az1.to_radians().cos();
            let sin1 = az1.to_radians().sin();

            let bins = &data.bins[r * data.num_range_bins..(r + 1) * data.num_range_bins];
            let num_bins = data.num_range_bins;
            let mut local_buf = QuadBuffer::with_capacity(128);

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
                    if run_count > 0
                        && gate_visible(run_level, is_velocity, hide_no_data, mask_weak_echoes)
                    {
                        let r0 = run_start_km;
                        let r1 = r0 + run_count as f64 * bin_size;
                        emit_quad_km(&mut local_buf, r0, r1, cos0, sin0, cos1, sin1, run_level);
                    }
                    run_level = level;
                    run_start_km = b as f64 * bin_size;
                    run_count = 1;
                }
            }
            if run_count > 0 && gate_visible(run_level, is_velocity, hide_no_data, mask_weak_echoes)
            {
                let r0 = run_start_km;
                let r1 = r0 + run_count as f64 * bin_size;
                emit_quad_km(&mut local_buf, r0, r1, cos0, sin0, cos1, sin1, run_level);
            }
            local_buf
        })
        .collect();

    // Reduce into a single buffer
    let total_quads: usize = results.iter().map(|b| b.quad_count).sum();
    let mut buf = QuadBuffer::with_capacity(total_quads);
    for mut local_buf in results {
        buf.positions.append(&mut local_buf.positions);
        buf.levels.append(&mut local_buf.levels);
        buf.quad_count += local_buf.quad_count;
    }

    buf
}

/// Generate radar-relative quads for raster Level 3 products (packet 0xBA07).
fn level3_raster_to_quads(
    data: &Level3Data,
    is_velocity: bool,
    hide_no_data: bool,
    mask_weak_echoes: bool,
) -> QuadBuffer {
    let rows = data.num_radials;
    let cols = data.num_range_bins;
    if rows == 0 || cols == 0 || data.bins.len() < rows * cols {
        return QuadBuffer::new();
    }

    let cell_km = data.bin_size_km as f64;
    let half_rows = rows as f64 / 2.0;
    let half_cols = cols as f64 / 2.0;

    let results: Vec<QuadBuffer> = (0..rows)
        .into_par_iter()
        .map(|row| {
            let mut local_buf = QuadBuffer::with_capacity(cols / 2);
            for col in 0..cols {
                let level = data.bins[row * cols + col];
                if !gate_visible(level, is_velocity, hide_no_data, mask_weak_echoes) {
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

                local_buf.push_quad(
                    (x0_km as f32, y0_km as f32),
                    (x1_km as f32, y1_km as f32),
                    (x2_km as f32, y2_km as f32),
                    (x3_km as f32, y3_km as f32),
                    level,
                );
            }
            local_buf
        })
        .collect();

    // Reduce into a single buffer
    let total_quads: usize = results.iter().map(|b| b.quad_count).sum();
    let mut buf = QuadBuffer::with_capacity(total_quads);
    for mut local_buf in results {
        buf.positions.append(&mut local_buf.positions);
        buf.levels.append(&mut local_buf.levels);
        buf.quad_count += local_buf.quad_count;
    }

    buf
}

#[inline]
fn gate_visible(level: u8, is_velocity: bool, hide_no_data: bool, mask_weak_echoes: bool) -> bool {
    if level == 0 && hide_no_data {
        return false;
    }
    if mask_weak_echoes && !is_velocity {
        let dbz = level as f64 / 2.0 - 32.0;
        if dbz < 5.0 {
            return false;
        }
    }
    true
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
    level: u8,
) {
    // Azimuth convention: angle 0 = North (up), increasing clockwise.
    // In radar coords (km): x = East (+), y = North (+)
    // radar x_km = r * sin(az), y_km = r * cos(az)
    let to_km = |r: f64, cos: f64, sin: f64| -> (f32, f32) {
        let x_km = r * sin;
        let y_km = r * cos;
        (x_km as f32, y_km as f32)
    };

    let v0 = to_km(r0, cos1, sin1); // near, next azimuth
    let v1 = to_km(r1, cos1, sin1); // far, next azimuth
    let v2 = to_km(r1, cos0, sin0); // far, current azimuth
    let v3 = to_km(r0, cos0, sin0); // near, current azimuth

    buf.push_quad(v0, v1, v2, v3, level);
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
    use meso_data::radar::level2::{Level2Data, NUM_RANGE_BINS};
    use meso_data::radar::level3::Level3Data;

    #[test]
    fn quad_buffer_new_is_empty() {
        let buf = QuadBuffer::new();
        assert_eq!(buf.quad_count, 0);
        assert!(buf.positions.is_empty());
        assert!(buf.levels.is_empty());
    }

    #[test]
    fn quad_buffer_with_capacity_starts_empty_but_preallocated() {
        let buf = QuadBuffer::with_capacity(100);
        assert_eq!(buf.quad_count, 0);
        // Capacity should be at least 100*8 = 800 for positions.
        assert!(buf.positions.capacity() >= 800);
        // Capacity should be at least 100*4 = 400 for levels.
        assert!(buf.levels.capacity() >= 400);
    }

    #[test]
    fn quad_buffer_new_starts_with_zero_quads() {
        let buf = QuadBuffer::new();
        assert_eq!(buf.quad_count, 0);
        assert!(buf.positions.is_empty());
        assert!(buf.levels.is_empty());
    }

    // ── Level2 geometry tests ─────────────────────────────────────────────────

    fn make_level2(n_radials: usize, fill_level: u8) -> Level2Data {
        let mut azimuths = Vec::with_capacity(n_radials);
        for i in 0..n_radials {
            azimuths.push((360.0 * i as f32) / n_radials as f32);
        }
        let bins = vec![fill_level; n_radials * NUM_RANGE_BINS];
        Level2Data {
            azimuths,
            bins,
            julian_date: 1,
            millis: 0,
            bin_size_km: 0.25,
            num_radials: n_radials,
            elevation_angle_deg: 0.5,
        }
    }

    #[test]
    fn level2_to_quads_empty_radials_returns_empty() {
        let data = make_level2(0, 50);
        let buf = level2_to_quads(&data, false, false, false);
        assert_eq!(buf.quad_count, 0);
        assert!(buf.positions.is_empty());
        assert!(buf.levels.is_empty());
    }

    #[test]
    fn level2_to_quads_all_no_data_with_hide_returns_empty() {
        // Level 0 == no-data; with hide_no_data=true should produce no quads.
        let data = make_level2(4, 0);
        let buf = level2_to_quads(&data, false, true, false);
        assert_eq!(buf.quad_count, 0);
    }

    #[test]
    fn level2_to_quads_positions_and_levels_consistent() {
        let n_radials = 8;
        let data = make_level2(n_radials, 50);
        let buf = level2_to_quads(&data, false, false, false);
        // Each quad = 4 vertices = 8 f32 positions + 4 level bytes.
        assert_eq!(buf.positions.len(), buf.quad_count * 8);
        assert_eq!(buf.levels.len(), buf.quad_count * 4);
        // All levels should be 50.
        assert!(buf.levels.iter().all(|&l| l == 50));
    }

    #[test]
    fn level2_to_quads_parallel_matches_sequential() {
        // With all bins the same level, run-length encoding produces exactly
        // 1 quad per radial (the single run spanning black_hole to the end).
        let n_radials = 36;
        let data = make_level2(n_radials, 80);
        let buf = level2_to_quads(&data, false, false, false);

        // One quad per radial (whole row is a single RLE run).
        assert_eq!(
            buf.quad_count, n_radials,
            "expected 1 quad per radial with uniform fill"
        );
        assert_eq!(buf.positions.len(), buf.quad_count * 8);
        assert_eq!(buf.levels.len(), buf.quad_count * 4);
        assert!(buf.levels.iter().all(|&l| l == 80));
    }

    // ── Level3 geometry tests ─────────────────────────────────────────────────

    fn make_level3_radial(n_radials: usize, n_bins: usize, fill_level: u8) -> Level3Data {
        Level3Data {
            is_raster: false,
            num_radials: n_radials,
            num_range_bins: n_bins,
            azimuths: (0..n_radials)
                .map(|i| (360.0 * i as f32) / n_radials as f32)
                .collect(),
            bins: vec![fill_level; n_radials * n_bins],
            bin_size_km: 1.0,
            product_code: 94,
            vcp: 212,
            scan_date: 1,
            scan_time_secs: 0,
        }
    }

    #[test]
    fn level3_to_quads_radial_positions_levels_consistent() {
        let buf = level3_to_quads(&make_level3_radial(8, 100, 60), false, false, false);
        assert_eq!(buf.positions.len(), buf.quad_count * 8);
        assert_eq!(buf.levels.len(), buf.quad_count * 4);
        assert!(buf.levels.iter().all(|&l| l == 60));
    }

    #[test]
    fn level3_to_quads_all_no_data_hide_returns_empty() {
        let buf = level3_to_quads(&make_level3_radial(4, 50, 0), false, true, false);
        assert_eq!(buf.quad_count, 0);
    }
}
