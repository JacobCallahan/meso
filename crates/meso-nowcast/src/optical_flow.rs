/*
 * Optical Flow: dense motion estimation for radar reflectivity fields.
 *
 * Implements a multi-scale pyramidal Lucas-Kanade optical flow algorithm,
 * executed as wgpu compute shaders (see shaders/flow.wgsl), capable of capturing complex, multi-directional, and semi-discrete storm motions.
 *
 * Key components:
 * 1. Multi-level Gaussian image pyramid (coarse-to-fine).
 * 2. Regularized local Lucas-Kanade solver at each scale.
 * 3. Temporal weighting across multiple sequential observed scans.
 * 4. Velocity field diffusion into clear-air boundary zones to support downstream advection.
 */

use crate::gpu::Gpu;
use crate::grid::RadarGrid;
use anyhow::Result;
use rayon::prelude::*;

/// Dense motion vector field.
/// Displacements are in units of grid cells per standard frame interval.
/// `u`: displacement along x (columns, East+, West-).
/// `v`: displacement along y (rows, South+, North-).
#[derive(Debug, Clone)]
pub struct MotionField {
    pub width: usize,
    pub height: usize,
    pub u: Vec<f32>,
    pub v: Vec<f32>,
}

impl MotionField {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            u: vec![0.0; width * height],
            v: vec![0.0; width * height],
        }
    }

    /// Sample the velocity field at continuous coordinates (col, row) using bilinear interpolation.
    #[inline]
    pub fn sample_uv(&self, col: f32, row: f32) -> (f32, f32) {
        if col < 0.0
            || row < 0.0
            || col >= (self.width - 1) as f32
            || row >= (self.height - 1) as f32
        {
            let c = col.clamp(0.0, (self.width - 1) as f32) as usize;
            let r = row.clamp(0.0, (self.height - 1) as f32) as usize;
            let idx = r * self.width + c;
            return (self.u[idx], self.v[idx]);
        }

        let c0 = col.floor() as usize;
        let r0 = row.floor() as usize;
        let c1 = c0 + 1;
        let r1 = r0 + 1;

        let fc = col - c0 as f32;
        let fr = row - r0 as f32;

        let idx00 = r0 * self.width + c0;
        let idx01 = r0 * self.width + c1;
        let idx10 = r1 * self.width + c0;
        let idx11 = r1 * self.width + c1;

        let u00 = self.u[idx00];
        let u01 = self.u[idx01];
        let u10 = self.u[idx10];
        let u11 = self.u[idx11];

        let v00 = self.v[idx00];
        let v01 = self.v[idx01];
        let v10 = self.v[idx10];
        let v11 = self.v[idx11];

        let u_top = u00 * (1.0 - fc) + u01 * fc;
        let u_bot = u10 * (1.0 - fc) + u11 * fc;
        let u_interp = u_top * (1.0 - fr) + u_bot * fr;

        let v_top = v00 * (1.0 - fc) + v01 * fc;
        let v_bot = v10 * (1.0 - fc) + v11 * fc;
        let v_interp = v_top * (1.0 - fr) + v_bot * fr;

        (u_interp, v_interp)
    }

    /// Scale all displacement vectors by a multiplier.
    pub fn scale(&mut self, factor: f32) {
        self.u.par_iter_mut().for_each(|val| *val *= factor);
        self.v.par_iter_mut().for_each(|val| *val *= factor);
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FlowParams {
    w: u32,
    h: u32,
    w2: u32,
    h2: u32,
    radius: u32,
    pad: u32,
    lambda: f32,
    scale: f32,
}

impl FlowParams {
    fn dims(w: usize, h: usize) -> Self {
        Self {
            w: w as u32,
            h: h as u32,
            w2: w as u32,
            h2: h as u32,
            radius: 5,
            pad: 0,
            lambda: 1.0, // Tikhonov regularization
            scale: 1.0,
        }
    }
}

/// Estimate motion between two consecutive radar grids using GPU pyramidal Lucas-Kanade.
pub fn estimate_motion_pair(prev: &RadarGrid, curr: &RadarGrid) -> Result<MotionField> {
    let gpu = Gpu::get()?;
    let k = &gpu.flow;
    let num_levels = 4; // 0, 1, 2, 3 (down to 1/8 resolution)
    let (w0, h0) = (prev.width, prev.height);

    // Level dimensions and image pyramids (level 0 = full resolution).
    let mut dims = vec![(w0, h0)];
    for _ in 1..num_levels {
        let (w, h) = *dims.last().unwrap();
        dims.push(((w / 2).max(1), (h / 2).max(1)));
    }
    let mut prev_pyr = vec![gpu.storage_from(&prev.data)];
    let mut curr_pyr = vec![gpu.storage_from(&curr.data)];
    let mut enc = gpu.encoder();
    for l in 1..num_levels {
        let (sw, sh) = dims[l - 1];
        let (dw, dh) = dims[l];
        let params = gpu.uniform(&FlowParams {
            w2: dw as u32,
            h2: dh as u32,
            ..FlowParams::dims(sw, sh)
        });
        for pyr in [&mut prev_pyr, &mut curr_pyr] {
            let out = gpu.storage_empty(dw * dh);
            gpu.dispatch_2d(
                &mut enc,
                &k.downsample,
                &[(0, &params), (1, &pyr[l - 1]), (5, &out)],
                dw,
                dh,
            );
            pyr.push(out);
        }
    }

    let (tw, th) = dims[num_levels - 1];
    let mut fu = gpu.storage_empty(tw * th);
    let mut fv = gpu.storage_empty(tw * th);

    for l in (0..num_levels).rev() {
        let (w, h) = dims[l];
        if l < num_levels - 1 {
            let (sw, sh) = dims[l + 1];
            let params = gpu.uniform(&FlowParams {
                w2: w as u32,
                h2: h as u32,
                scale: 2.0,
                ..FlowParams::dims(sw, sh)
            });
            let nu = gpu.storage_empty(w * h);
            let nv = gpu.storage_empty(w * h);
            gpu.dispatch_2d(
                &mut enc,
                &k.upsample_flow,
                &[(0, &params), (3, &fu), (4, &fv), (5, &nu), (6, &nv)],
                w,
                h,
            );
            fu = nu;
            fv = nv;
        }

        let params = gpu.uniform(&FlowParams::dims(w, h));
        let terms = gpu.storage_empty(w * h * 5);
        let tmp = gpu.storage_empty(w * h * 5);
        let sums = gpu.storage_empty(w * h * 5);
        for _ in 0..3 {
            gpu.dispatch_2d(
                &mut enc,
                &k.lk_terms,
                &[
                    (0, &params),
                    (1, &prev_pyr[l]),
                    (2, &curr_pyr[l]),
                    (3, &fu),
                    (4, &fv),
                    (5, &terms),
                ],
                w,
                h,
            );
            gpu.dispatch_2d(
                &mut enc,
                &k.blur_h,
                &[(0, &params), (1, &terms), (5, &tmp)],
                w,
                h,
            );
            gpu.dispatch_2d(
                &mut enc,
                &k.blur_v,
                &[(0, &params), (1, &tmp), (5, &sums)],
                w,
                h,
            );
            gpu.dispatch_2d(
                &mut enc,
                &k.lk_solve,
                &[(0, &params), (1, &sums), (5, &fu), (6, &fv)],
                w,
                h,
            );
        }
    }

    // Diffuse velocity into clear-air zones around echoes so advection downstream works.
    let params = gpu.uniform(&FlowParams::dims(w0, h0));
    let n = w0 * h0;
    let mut cur = (
        gpu.storage_empty(n),
        gpu.storage_empty(n),
        gpu.storage_empty(n),
    );
    let mut nxt = (
        gpu.storage_empty(n),
        gpu.storage_empty(n),
        gpu.storage_empty(n),
    );
    enc.copy_buffer_to_buffer(&fu, 0, &cur.0, 0, (n * 4) as u64);
    enc.copy_buffer_to_buffer(&fv, 0, &cur.1, 0, (n * 4) as u64);
    gpu.dispatch_2d(
        &mut enc,
        &k.diffuse_init,
        &[(0, &params), (1, &curr_pyr[0]), (5, &cur.2)],
        w0,
        h0,
    );
    for _ in 0..DIFFUSE_ITERATIONS {
        gpu.dispatch_2d(
            &mut enc,
            &k.diffuse_step,
            &[
                (0, &params),
                (1, &cur.2),
                (3, &cur.0),
                (4, &cur.1),
                (5, &nxt.0),
                (6, &nxt.1),
                (7, &nxt.2),
            ],
            w0,
            h0,
        );
        std::mem::swap(&mut cur, &mut nxt);
    }
    gpu.dispatch_2d(
        &mut enc,
        &k.diffuse_apply,
        &[
            (0, &params),
            (1, &curr_pyr[0]),
            (2, &cur.2),
            (3, &cur.0),
            (4, &cur.1),
            (5, &fu),
            (6, &fv),
        ],
        w0,
        h0,
    );
    gpu.submit(enc);

    Ok(MotionField {
        width: w0,
        height: h0,
        u: gpu.read(&fu, n)?,
        v: gpu.read(&fv, n)?,
    })
}

const DIFFUSE_ITERATIONS: usize = 60;

/// Estimate motion across a sequence of sequential radar grids (at least 2 frames).
/// Computes pairwise optical flow between adjacent frames and calculates a
/// recency-weighted average of the motion fields.
pub fn estimate_motion_sequence(grids: &[RadarGrid]) -> Result<MotionField> {
    estimate_motion_sequence_with_progress(grids, &mut |_, _| {})
}

pub(crate) fn estimate_motion_sequence_with_progress(
    grids: &[RadarGrid],
    progress: &mut dyn FnMut(usize, usize),
) -> Result<MotionField> {
    if grids.len() < 2 {
        let w = grids.first().map(|g| g.width).unwrap_or(256);
        let h = grids.first().map(|g| g.height).unwrap_or(256);
        return Ok(MotionField::new(w, h));
    }

    let w = grids.last().unwrap().width;
    let h = grids.last().unwrap().height;

    // Calculate pairwise flow fields
    let num_pairs = grids.len() - 1;
    let mut weighted_u = vec![0.0f32; w * h];
    let mut weighted_v = vec![0.0f32; w * h];
    let mut total_weight = 0.0f32;

    for i in 0..num_pairs {
        let prev = &grids[i];
        let curr = &grids[i + 1];

        let dt = (curr.timestamp_secs - prev.timestamp_secs).max(1) as f32;
        // Standardize time interval relative to the most recent frame interval
        let recent_dt = (grids.last().unwrap().timestamp_secs
            - grids[grids.len() - 2].timestamp_secs)
            .max(1) as f32;
        let time_scale = if dt > 0.0 { recent_dt / dt } else { 1.0 };

        let mut field = estimate_motion_pair(prev, curr)?;
        if (time_scale - 1.0).abs() > 0.05 {
            field.scale(time_scale);
        }

        // Exponential recency weight: recent pairs have higher weight
        let recency_step = (num_pairs - 1 - i) as f32;
        let weight = (-0.35 * recency_step).exp();

        weighted_u
            .par_iter_mut()
            .zip(field.u.par_iter())
            .for_each(|(w_u, &f_u)| *w_u += f_u * weight);

        weighted_v
            .par_iter_mut()
            .zip(field.v.par_iter())
            .for_each(|(w_v, &f_v)| *w_v += f_v * weight);

        total_weight += weight;
        progress(i + 1, num_pairs);
    }

    if total_weight > 0.0 {
        let inv_weight = 1.0 / total_weight;
        weighted_u.par_iter_mut().for_each(|val| *val *= inv_weight);
        weighted_v.par_iter_mut().for_each(|val| *val *= inv_weight);
    }

    let last = grids.last().unwrap();
    let mut echo_mask = vec![0.0f32; w * h];
    let mut echo_u = vec![0.0f32; w * h];
    let mut echo_v = vec![0.0f32; w * h];
    for i in 0..w * h {
        if last.data[i] > 5.0 {
            echo_mask[i] = 1.0;
            echo_u[i] = weighted_u[i];
            echo_v[i] = weighted_v[i];
        }
    }

    let radius = (15.0 / last.cell_km).round().clamp(4.0, 32.0) as usize;
    let local_weight = box_blur_separable(&echo_mask, w, h, radius);
    if local_weight.iter().any(|&weight| weight > 1e-4) {
        let local_u = box_blur_separable(&echo_u, w, h, radius);
        let local_v = box_blur_separable(&echo_v, w, h, radius);
        for i in 0..w * h {
            if local_weight[i] > 1e-4 {
                let mean_u = local_u[i] / local_weight[i];
                let mean_v = local_v[i] / local_weight[i];
                weighted_u[i] = mean_u + FLOW_LOCAL_WEIGHT * (weighted_u[i] - mean_u);
                weighted_v[i] = mean_v + FLOW_LOCAL_WEIGHT * (weighted_v[i] - mean_v);
            }
        }
    }

    Ok(MotionField {
        width: w,
        height: h,
        u: weighted_u,
        v: weighted_v,
    })
}

/// Fraction of the local (deviation from neighborhood mean) flow kept for advection.
const FLOW_LOCAL_WEIGHT: f32 = 0.3;

// ── Private optical flow details ──────────────────────────────────────────────

/// Fast separable box blur for 2D scalar fields.
fn box_blur_separable(src: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
    if radius == 0 {
        return src.to_vec();
    }
    let mut temp = vec![0.0f32; w * h];
    let mut out = vec![0.0f32; w * h];
    let r = radius as isize;

    // Horizontal pass
    temp.par_chunks_exact_mut(w)
        .enumerate()
        .for_each(|(row, row_out)| {
            let row_offset = row * w;
            let mut sum = 0.0f32;
            let mut count = 0.0f32;

            // Prime the initial window
            for c in -r..=r {
                if c >= 0 && (c as usize) < w {
                    sum += src[row_offset + c as usize];
                    count += 1.0;
                }
            }
            row_out[0] = if count > 0.0 { sum / count } else { 0.0 };

            for (c, value) in row_out.iter_mut().enumerate().skip(1) {
                let left = c as isize - r - 1;
                let right = c as isize + r;
                if left >= 0 {
                    sum -= src[row_offset + left as usize];
                    count -= 1.0;
                }
                if right < w as isize {
                    sum += src[row_offset + right as usize];
                    count += 1.0;
                }
                *value = if count > 0.0 { sum / count } else { 0.0 };
            }
        });

    // Vertical pass
    for c in 0..w {
        let mut sum = 0.0f32;
        let mut count = 0.0f32;

        for r_idx in -r..=r {
            if r_idx >= 0 && (r_idx as usize) < h {
                sum += temp[r_idx as usize * w + c];
                count += 1.0;
            }
        }
        out[c] = if count > 0.0 { sum / count } else { 0.0 };

        for row in 1..h {
            let top = row as isize - r - 1;
            let bot = row as isize + r;
            if top >= 0 {
                sum -= temp[top as usize * w + c];
                count -= 1.0;
            }
            if bot < h as isize {
                sum += temp[bot as usize * w + c];
                count += 1.0;
            }
            out[row * w + c] = if count > 0.0 { sum / count } else { 0.0 };
        }
    }

    out
}
