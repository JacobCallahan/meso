/*
 * Semi-Lagrangian Advection Engine for Radar Extrapolation.
 *
 * Generates synthetic future radar frames by backward semi-Lagrangian trajectory
 * tracing from the last observed frame.
 *
 * Key advantages over Eulerian step-by-step chaining:
 * 1. Zero cumulative numerical diffusion (samples directly from last observed frame).
 * 2. Unconditionally stable regardless of storm speed (no CFL time-step restriction).
 * 3. 2nd-order Runge-Kutta (RK2) backward trajectory integration preserves curved/rotational flow.
 * 4. Progressive physical decay of storm intensity over multi-hour lead times.
 */

use crate::grid::RadarGrid;
use crate::optical_flow::MotionField;
use rayon::prelude::*;

/// Parameters controlling the nowcast advection.
#[derive(Debug, Clone, Copy)]
pub struct NowcastConfig {
    /// Number of future frames to generate.
    pub steps: usize,
    /// Time delta (in seconds) between successive frames.
    pub interval_secs: i64,
    /// Multiplicative intensity decay factor per frame interval (e.g. 0.995).
    pub decay_factor: f32,
}

impl Default for NowcastConfig {
    fn default() -> Self {
        Self {
            steps: 12,           // 1 hour at 5-minute intervals
            interval_secs: 300,  // 5 minutes
            decay_factor: 0.995, // ~6% decay per hour
        }
    }
}

/// Generate a sequence of future RadarGrid frames using semi-Lagrangian advection.
pub fn advect_nowcast(
    last_grid: &RadarGrid,
    motion: &MotionField,
    config: &NowcastConfig,
) -> Vec<RadarGrid> {
    let width = last_grid.width;
    let height = last_grid.height;
    let cell_km = last_grid.cell_km;
    let base_epoch = last_grid.timestamp_secs;

    let mut forecast_grids = Vec::with_capacity(config.steps);

    for step in 1..=config.steps {
        let step_f = step as f32;
        let step_epoch = base_epoch + step as i64 * config.interval_secs;
        let decay = config.decay_factor.powi(step as i32);

        let mut future_grid = RadarGrid::new(width, height, cell_km, step_epoch);

        future_grid.data
            .par_chunks_exact_mut(width)
            .enumerate()
            .for_each(|(r, row_slice)| {
                for c in 0..width {
                    let col_f = c as f32;
                    let row_f = r as f32;

                    // RK2 midpoint backward trajectory tracing
                    let (u0, v0) = motion.sample_uv(col_f, row_f);
                    let c_mid = col_f - 0.5 * step_f * u0;
                    let r_mid = row_f - 0.5 * step_f * v0;

                    let (u_mid, v_mid) = motion.sample_uv(c_mid, r_mid);
                    let c_src = col_f - step_f * u_mid;
                    let r_src = row_f - step_f * v_mid;

                    let mut val = last_grid.sample_bilinear(c_src, r_src);

                    // Apply intensity decay
                    val *= decay;

                    // Clean up sub-threshold noise
                    if val < 2.0 {
                        val = 0.0;
                    }

                    row_slice[c] = val.clamp(0.0, 255.0);
                }
            });

        forecast_grids.push(future_grid);
    }

    forecast_grids
}
