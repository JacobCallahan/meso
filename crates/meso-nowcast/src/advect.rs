/*
 * Eulerian Advection-Diffusion Nowcast Engine (wgpu compute shader).
 *
 * Generates synthetic future radar frames by integrating
 *
 *     dC/dt = -u . grad(C) + K lap(C) - lambda C
 *
 * forward in time from the last observed frame on the GPU. The scheme is a
 * flux-form update (corrected by C div(u) so reflectivity is transported, not
 * accumulated, in convergent flow) with MC-limited MUSCL upwind fluxes,
 * explicit diffusion and exponential decay. The frozen optical-flow field `u`
 * drives the advection. Time stepping is explicit, so each frame interval is
 * split into sub-steps that satisfy the advective (Courant) and diffusive
 * stability limits.
 */

use crate::gpu::Gpu;
use crate::grid::RadarGrid;
use crate::optical_flow::MotionField;
use anyhow::Result;
use rayon::prelude::*;

/// Combined Courant number (|u| + |v| in cells per sub-step) allowed by the scheme.
const MAX_COURANT: f32 = 0.4;
/// Maximum diffusion number K*dt/dx^2 per sub-step (2D explicit limit is 0.25).
const MAX_DIFFUSION_NUMBER: f32 = 0.2;
/// Safety cap on sub-steps per frame interval.
const MAX_SUBSTEPS: usize = 4096;

/// Parameters controlling the nowcast integration.
#[derive(Debug, Clone, Copy)]
pub struct NowcastConfig {
    /// Number of future frames to generate.
    pub steps: usize,
    /// Time delta (in seconds) between successive frames.
    pub interval_secs: i64,
    /// Multiplicative intensity decay factor per frame interval (e.g. 0.995).
    pub decay_factor: f32,
    /// Diffusion coefficient K in km^2 per minute.
    pub diffusion_km2_per_min: f32,
}

/// Default diffusion coefficient (km^2/min).
pub const DEFAULT_DIFFUSION_KM2_PER_MIN: f32 = 0.05;
/// Upper bound exposed in the UI (km^2/min); the lower bound is 0 (no explicit smoothing).
pub const MAX_DIFFUSION_KM2_PER_MIN: f32 = 0.5;

impl Default for NowcastConfig {
    fn default() -> Self {
        Self {
            steps: 12,           // 1 hour at 5-minute intervals
            interval_secs: 300,  // 5 minutes
            decay_factor: 0.995, // ~6% decay per hour
            diffusion_km2_per_min: DEFAULT_DIFFUSION_KM2_PER_MIN,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct AdvectParams {
    width: u32,
    height: u32,
    diffusion: f32,
    decay: f32,
}

/// Number of sub-steps per frame interval needed for stability.
fn substeps_for(motion: &MotionField, diffusion_cells2: f32) -> usize {
    let max_speed = motion
        .u
        .par_iter()
        .zip(motion.v.par_iter())
        .map(|(u, v)| u.abs() + v.abs())
        .reduce(|| 0.0f32, f32::max);
    let combined = max_speed / MAX_COURANT + diffusion_cells2 / MAX_DIFFUSION_NUMBER;
    (combined.ceil().max(1.0) as usize).min(MAX_SUBSTEPS)
}

/// Generate a sequence of future RadarGrid frames by Eulerian advection-diffusion on the GPU.
pub fn advect_nowcast(
    last_grid: &RadarGrid,
    motion: &MotionField,
    config: &NowcastConfig,
) -> Result<Vec<RadarGrid>> {
    advect_nowcast_with_progress(last_grid, motion, config, &mut |_, _| {})
}

pub(crate) fn advect_nowcast_with_progress(
    last_grid: &RadarGrid,
    motion: &MotionField,
    config: &NowcastConfig,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Vec<RadarGrid>> {
    let gpu = Gpu::get()?;
    let (width, height) = (last_grid.width, last_grid.height);
    let n_cells = width * height;
    anyhow::ensure!(
        motion.width == width && motion.height == height,
        "motion field size does not match radar grid"
    );

    // K in cells^2 per frame interval.
    let interval_min = config.interval_secs as f32 / 60.0;
    let k_cells2 = config
        .diffusion_km2_per_min
        .clamp(0.0, MAX_DIFFUSION_KM2_PER_MIN)
        * interval_min
        / (last_grid.cell_km * last_grid.cell_km);
    let n_sub = substeps_for(motion, k_cells2);
    tracing::debug!(
        "nowcast: {n_sub} sub-steps per interval on {}",
        gpu.adapter_name
    );

    let inv_n = 1.0 / n_sub as f32;
    let u: Vec<f32> = motion.u.iter().map(|v| v * inv_n).collect();
    let v: Vec<f32> = motion.v.iter().map(|v| v * inv_n).collect();
    let buf_u = gpu.storage_from(&u);
    let buf_v = gpu.storage_from(&v);

    let params = gpu.uniform(&AdvectParams {
        width: width as u32,
        height: height as u32,
        diffusion: k_cells2 * inv_n,
        decay: config.decay_factor.clamp(0.0, 1.0).powf(inv_n),
    });

    let mut ping = gpu.storage_from(&last_grid.data);
    let mut pong = gpu.storage_empty(n_cells);

    let mut frames = Vec::with_capacity(config.steps);
    for step in 1..=config.steps {
        let mut enc = gpu.encoder();
        for _ in 0..n_sub {
            gpu.dispatch_2d(
                &mut enc,
                &gpu.advect,
                &[
                    (0, &params),
                    (1, &ping),
                    (2, &pong),
                    (3, &buf_u),
                    (4, &buf_v),
                ],
                width,
                height,
            );
            std::mem::swap(&mut ping, &mut pong);
        }
        gpu.submit(enc);

        let mut grid = RadarGrid::new(
            width,
            height,
            last_grid.cell_km,
            last_grid.timestamp_secs + step as i64 * config.interval_secs,
        );
        grid.data = gpu.read(&ping, n_cells)?;
        // Clean up sub-threshold noise
        grid.data
            .par_iter_mut()
            .for_each(|val| *val = if *val < 2.0 { 0.0 } else { val.min(255.0) });
        frames.push(grid);
        progress(step, config.steps);
    }

    Ok(frames)
}

/// Estimate the per-interval intensity factor from the observed sequence as
/// the echo-sum ratio between successive frames inside the inner part of the
/// grid (where storms are mostly conserved). Optical-flow warping is avoided
/// because it can absorb intensity change into spurious motion.
/// The measured loss is clamped to [0.99, 1.0] and then halved (DECAY_SCALE),
/// since the raw estimate over-dries long leads; falls back to 0.998.
pub fn estimate_decay_factor(grids: &[RadarGrid]) -> f32 {
    const FALLBACK: f32 = 0.998;
    const DECAY_SCALE: f32 = 0.5;
    let mut ratios = Vec::new();

    for pair in grids.windows(2).rev().take(6) {
        let (prev, curr) = (&pair[0], &pair[1]);
        if prev.width != curr.width || prev.height != curr.height {
            continue;
        }
        let (w, h) = (curr.width, curr.height);
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let max_r2 = (0.35 * w.min(h) as f32).powi(2);

        let (ps, cs) = (0..h)
            .into_par_iter()
            .map(|r| {
                let (mut ps, mut cs) = (0.0f64, 0.0f64);
                let dy2 = (r as f32 - cy).powi(2);
                for c in 0..w {
                    if (c as f32 - cx).powi(2) + dy2 > max_r2 {
                        continue;
                    }
                    let (p, q) = (prev.data[r * w + c], curr.data[r * w + c]);
                    if p > 5.0 {
                        ps += p as f64;
                    }
                    if q > 5.0 {
                        cs += q as f64;
                    }
                }
                (ps, cs)
            })
            .reduce(|| (0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
        if ps > 5.0 * 500.0 {
            ratios.push((cs / ps) as f32);
        }
    }

    if ratios.is_empty() {
        return FALLBACK;
    }
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let measured = ratios[ratios.len() / 2].clamp(0.99, 1.0);
    1.0 - (1.0 - measured) * DECAY_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(w: usize, h: usize, cx: f32, cy: f32, sigma: f32) -> RadarGrid {
        let mut g = RadarGrid::new(w, h, 1.0, 0);
        for r in 0..h {
            for c in 0..w {
                let d2 = (c as f32 - cx).powi(2) + (r as f32 - cy).powi(2);
                g.data[r * w + c] = 250.0 * (-d2 / (2.0 * sigma * sigma)).exp();
            }
        }
        g
    }

    fn uniform_flow(w: usize, h: usize, u: f32, v: f32) -> MotionField {
        let mut m = MotionField::new(w, h);
        m.u.fill(u);
        m.v.fill(v);
        m
    }

    fn centroid(g: &RadarGrid) -> (f64, f64, f64) {
        let (mut m, mut sx, mut sy) = (0.0, 0.0, 0.0);
        for r in 0..g.height {
            for c in 0..g.width {
                let v = g.data[r * g.width + c] as f64;
                m += v;
                sx += v * c as f64;
                sy += v * r as f64;
            }
        }
        (m, sx / m, sy / m)
    }

    fn cfg(steps: usize, decay: f32, k: f32) -> NowcastConfig {
        NowcastConfig {
            steps,
            interval_secs: 60,
            decay_factor: decay,
            diffusion_km2_per_min: k,
        }
    }

    #[test]
    fn advects_blob_by_flow_and_conserves_mass() {
        let g = blob(160, 120, 40.0, 60.0, 6.0);
        let m = uniform_flow(160, 120, 3.0, 1.0);
        let out = advect_nowcast(&g, &m, &cfg(10, 1.0, 0.0)).unwrap();
        let (m0, x0, y0) = centroid(&g);
        let (m1, x1, y1) = centroid(out.last().unwrap());
        assert!((x1 - x0 - 30.0).abs() < 0.5, "dx={}", x1 - x0);
        assert!((y1 - y0 - 10.0).abs() < 0.5, "dy={}", y1 - y0);
        assert!((m1 / m0 - 1.0).abs() < 0.03, "mass ratio {}", m1 / m0);
    }

    #[test]
    fn diffusion_widens_blob_with_expected_variance() {
        let g = blob(200, 200, 100.0, 100.0, 8.0);
        let m = uniform_flow(200, 200, 0.0, 0.0);
        let k = 0.5; // km^2/min, 1 km cells, 20 one-minute steps
        let out = advect_nowcast(&g, &m, &cfg(20, 1.0, k)).unwrap();
        let var = |g: &RadarGrid| {
            let (mass, cx, _) = centroid(g);
            let mut s = 0.0;
            for r in 0..g.height {
                for c in 0..g.width {
                    s += g.data[r * g.width + c] as f64 * (c as f64 - cx).powi(2);
                }
            }
            s / mass
        };
        let grew = var(out.last().unwrap()) - var(&g);
        let expected = 2.0 * k as f64 * 20.0;
        // Output frames drop echoes below 2 dBZ-units, which trims the tails slightly.
        assert!(
            grew > 0.7 * expected && grew < 1.05 * expected,
            "var growth {grew}, expected {expected}"
        );
        let still = advect_nowcast(&g, &m, &cfg(20, 1.0, 0.0)).unwrap();
        assert!(
            var(still.last().unwrap()) - var(&g) < 0.5,
            "K=0 should not widen the blob"
        );
    }

    #[test]
    fn combined_advection_and_diffusion_preserve_echo_mass() {
        let mut g = RadarGrid::new(32, 32, 0.5, 0);
        g.data[16 * g.width + 16] = 100.0;
        let m = uniform_flow(32, 32, 2.0, 0.0);
        let config = NowcastConfig {
            steps: 1,
            interval_secs: 300,
            decay_factor: 1.0,
            diffusion_km2_per_min: DEFAULT_DIFFUSION_KM2_PER_MIN,
        };

        let out = advect_nowcast(&g, &m, &config).unwrap();
        let total: f32 = out[0].data.iter().sum();
        assert!(total <= 100.0, "echo mass increased from 100 to {total}");
    }

    #[test]
    fn fast_flow_is_stable_via_substepping() {
        let g = blob(200, 100, 30.0, 50.0, 5.0);
        let m = uniform_flow(200, 100, 20.0, 0.0);
        let out = advect_nowcast(&g, &m, &cfg(5, 1.0, 0.0)).unwrap();
        let last = out.last().unwrap();
        assert!(last.data.iter().all(|v| v.is_finite() && *v <= 255.0));
        let (_, x1, _) = centroid(last);
        assert!((x1 - 130.0).abs() < 1.0, "x1={x1}");
    }

    #[test]
    fn convergent_flow_does_not_intensify_echoes() {
        let (w, h) = (120, 40);
        let mut g = RadarGrid::new(w, h, 1.0, 0);
        for r in 0..h {
            for c in 0..w {
                g.data[r * w + c] = 40.0;
            }
        }
        let mut m = MotionField::new(w, h);
        for r in 0..h {
            for c in 0..w {
                m.u[r * w + c] = (60.0 - c as f32) * 0.03;
            }
        }
        let out = advect_nowcast(&g, &m, &cfg(12, 1.0, 0.0)).unwrap();
        let peak = out
            .last()
            .unwrap()
            .data
            .iter()
            .cloned()
            .fold(0.0f32, f32::max);
        assert!(peak < 45.0, "echo intensified to {peak}");
    }

    #[test]
    fn decay_reduces_intensity() {
        let g = blob(80, 80, 40.0, 40.0, 6.0);
        let m = uniform_flow(80, 80, 0.0, 0.0);
        let out = advect_nowcast(&g, &m, &cfg(2, 0.9, 0.0)).unwrap();
        let ratio = out[1].data[40 * 80 + 40] / g.data[40 * 80 + 40];
        assert!((ratio - 0.81).abs() < 0.01, "ratio {ratio}");
    }
}
