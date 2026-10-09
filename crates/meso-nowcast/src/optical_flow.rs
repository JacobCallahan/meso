/*
 * Optical Flow: dense motion estimation for radar reflectivity fields.
 *
 * Implements a multi-scale pyramidal Lucas-Kanade optical flow algorithm
 * capable of capturing complex, multi-directional, and semi-discrete storm motions.
 *
 * Key components:
 * 1. Multi-level Gaussian image pyramid (coarse-to-fine).
 * 2. Regularized local Lucas-Kanade solver at each scale.
 * 3. Temporal weighting across multiple sequential observed scans.
 * 4. Velocity field diffusion into clear-air boundary zones to support semi-Lagrangian advection.
 */

use crate::grid::RadarGrid;
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
        if col < 0.0 || row < 0.0 || col >= (self.width - 1) as f32 || row >= (self.height - 1) as f32 {
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

/// Estimate motion between two consecutive radar grids using pyramidal Lucas-Kanade.
pub fn estimate_motion_pair(prev: &RadarGrid, curr: &RadarGrid) -> MotionField {
    let num_levels = 4; // 0, 1, 2, 3 (down to 1/8 resolution)
    let prev_pyramid = build_pyramid(&prev.data, prev.width, prev.height, num_levels);
    let curr_pyramid = build_pyramid(&curr.data, curr.width, curr.height, num_levels);

    let (top_w, top_h, _) = prev_pyramid[num_levels - 1];
    let mut current_field = MotionField::new(top_w, top_h);

    // Coarse to fine
    for level_idx in (0..num_levels).rev() {
        let (w, h, ref prev_img) = prev_pyramid[level_idx];
        let (_, _, ref curr_img) = curr_pyramid[level_idx];

        if level_idx < num_levels - 1 {
            // Upsample displacement field from previous coarser level
            current_field = upsample_motion_field(&current_field, w, h);
            current_field.scale(2.0);
        }

        // Run iterations of Lucas-Kanade refinement at this scale
        let iterations = 3;
        let window_radius = 5;
        let lambda = 1.0f32; // Tikhonov regularization

        for _ in 0..iterations {
            refine_lk(prev_img, curr_img, &mut current_field, window_radius, lambda);
        }
    }

    // Diffuse velocity into clear-air zones around echoes so advection downstream works
    diffuse_velocity_field(&mut current_field, &curr.data, 20);

    current_field
}

/// Estimate motion across a sequence of sequential radar grids (at least 2 frames).
/// Computes pairwise optical flow between adjacent frames and calculates a
/// recency-weighted average of the motion fields.
pub fn estimate_motion_sequence(grids: &[RadarGrid]) -> MotionField {
    if grids.len() < 2 {
        let w = grids.first().map(|g| g.width).unwrap_or(256);
        let h = grids.first().map(|g| g.height).unwrap_or(256);
        return MotionField::new(w, h);
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
        let recent_dt = (grids.last().unwrap().timestamp_secs - grids[grids.len() - 2].timestamp_secs).max(1) as f32;
        let time_scale = if dt > 0.0 { recent_dt / dt } else { 1.0 };

        let mut field = estimate_motion_pair(prev, curr);
        if (time_scale - 1.0).abs() > 0.05 {
            field.scale(time_scale);
        }

        // Exponential recency weight: recent pairs have higher weight
        let recency_step = (num_pairs - 1 - i) as f32;
        let weight = (-0.35 * recency_step).exp();

        weighted_u.par_iter_mut()
            .zip(field.u.par_iter())
            .for_each(|(w_u, &f_u)| *w_u += f_u * weight);

        weighted_v.par_iter_mut()
            .zip(field.v.par_iter())
            .for_each(|(w_v, &f_v)| *w_v += f_v * weight);

        total_weight += weight;
    }

    if total_weight > 0.0 {
        let inv_weight = 1.0 / total_weight;
        weighted_u.par_iter_mut().for_each(|val| *val *= inv_weight);
        weighted_v.par_iter_mut().for_each(|val| *val *= inv_weight);
    }

    MotionField {
        width: w,
        height: h,
        u: weighted_u,
        v: weighted_v,
    }
}

// ── Private optical flow details ──────────────────────────────────────────────

/// Build a downsampled Gaussian image pyramid.
fn build_pyramid(data: &[f32], width: usize, height: usize, levels: usize) -> Vec<(usize, usize, Vec<f32>)> {
    let mut pyramid = Vec::with_capacity(levels);
    pyramid.push((width, height, data.to_vec()));

    let mut cur_w = width;
    let mut cur_h = height;
    let mut cur_data = data.to_vec();

    for _ in 1..levels {
        let next_w = (cur_w / 2).max(1);
        let next_h = (cur_h / 2).max(1);
        let mut next_data = vec![0.0f32; next_w * next_h];

        for r in 0..next_h {
            let r0 = (r * 2).min(cur_h - 1);
            let r1 = (r0 + 1).min(cur_h - 1);
            for c in 0..next_w {
                let c0 = (c * 2).min(cur_w - 1);
                let c1 = (c0 + 1).min(cur_w - 1);

                let p00 = cur_data[r0 * cur_w + c0];
                let p01 = cur_data[r0 * cur_w + c1];
                let p10 = cur_data[r1 * cur_w + c0];
                let p11 = cur_data[r1 * cur_w + c1];

                next_data[r * next_w + c] = 0.25 * (p00 + p01 + p10 + p11);
            }
        }

        pyramid.push((next_w, next_h, next_data.clone()));
        cur_w = next_w;
        cur_h = next_h;
        cur_data = next_data;
    }

    pyramid
}

/// Upsample a motion field by 2x.
fn upsample_motion_field(field: &MotionField, target_w: usize, target_h: usize) -> MotionField {
    let mut out = MotionField::new(target_w, target_h);
    let scale_x = field.width as f32 / target_w as f32;
    let scale_y = field.height as f32 / target_h as f32;

    out.u.par_chunks_mut(target_w)
        .zip(out.v.par_chunks_mut(target_w))
        .enumerate()
        .for_each(|(r, (u_row, v_row))| {
            let src_r = (r as f32 + 0.5) * scale_y - 0.5;
            for c in 0..target_w {
                let src_c = (c as f32 + 0.5) * scale_x - 0.5;
                let (u, v) = field.sample_uv(src_c, src_r);
                u_row[c] = u;
                v_row[c] = v;
            }
        });

    out
}

/// Warp an image using the current motion field: warped(x, y) = img(x + u, y + v).
fn warp_image(img: &[f32], w: usize, h: usize, field: &MotionField) -> Vec<f32> {
    let mut warped = vec![0.0f32; w * h];

    warped.par_chunks_mut(w)
        .enumerate()
        .for_each(|(r, row)| {
            for c in 0..w {
                let idx = r * w + c;
                let src_c = c as f32 + field.u[idx];
                let src_r = r as f32 + field.v[idx];
                row[c] = sample_bilinear_slice(img, w, h, src_c, src_r);
            }
        });

    warped
}

#[inline]
fn sample_bilinear_slice(img: &[f32], w: usize, h: usize, c: f32, r: f32) -> f32 {
    if c < 0.0 || r < 0.0 || c >= (w - 1) as f32 || r >= (h - 1) as f32 {
        if c >= -0.5 && c <= w as f32 - 0.5 && r >= -0.5 && r <= h as f32 - 0.5 {
            let ic = c.clamp(0.0, (w - 1) as f32) as usize;
            let ir = r.clamp(0.0, (h - 1) as f32) as usize;
            return img[ir * w + ic];
        }
        return 0.0;
    }

    let c0 = c.floor() as usize;
    let r0 = r.floor() as usize;
    let c1 = c0 + 1;
    let r1 = r0 + 1;

    let fc = c - c0 as f32;
    let fr = r - r0 as f32;

    let v00 = img[r0 * w + c0];
    let v01 = img[r0 * w + c1];
    let v10 = img[r1 * w + c0];
    let v11 = img[r1 * w + c1];

    let top = v00 * (1.0 - fc) + v01 * fc;
    let bot = v10 * (1.0 - fc) + v11 * fc;

    top * (1.0 - fr) + bot * fr
}

/// Refine the motion field at the current scale using regularized Lucas-Kanade.
fn refine_lk(
    prev: &[f32],
    curr: &[f32],
    field: &mut MotionField,
    radius: usize,
    lambda: f32,
) {
    let w = field.width;
    let h = field.height;

    // Warp curr towards prev using current motion field:
    // curr_warped(x, y) = curr(x + u, y + v)
    let curr_warped = warp_image(curr, w, h, field);

    // Compute gradients
    let mut ix = vec![0.0f32; w * h];
    let mut iy = vec![0.0f32; w * h];
    let mut it = vec![0.0f32; w * h];

    for r in 0..h {
        let r_prev = r.saturating_sub(1);
        let r_next = (r + 1).min(h - 1);
        for c in 0..w {
            let c_prev = c.saturating_sub(1);
            let c_next = (c + 1).min(w - 1);

            let idx = r * w + c;

            // Average image for spatial gradient
            let avg_left = 0.5 * (prev[r * w + c_prev] + curr_warped[r * w + c_prev]);
            let avg_right = 0.5 * (prev[r * w + c_next] + curr_warped[r * w + c_next]);
            let avg_top = 0.5 * (prev[r_prev * w + c] + curr_warped[r_prev * w + c]);
            let avg_bottom = 0.5 * (prev[r_next * w + c] + curr_warped[r_next * w + c]);

            ix[idx] = 0.5 * (avg_right - avg_left);
            iy[idx] = 0.5 * (avg_bottom - avg_top);
            it[idx] = curr_warped[idx] - prev[idx];
        }
    }

    // Compute term products
    let mut ixx = vec![0.0f32; w * h];
    let mut iyy = vec![0.0f32; w * h];
    let mut ixy = vec![0.0f32; w * h];
    let mut ixt = vec![0.0f32; w * h];
    let mut iyt = vec![0.0f32; w * h];

    for i in 0..w * h {
        let x = ix[i];
        let y = iy[i];
        let t = it[i];
        ixx[i] = x * x;
        iyy[i] = y * y;
        ixy[i] = x * y;
        ixt[i] = x * t;
        iyt[i] = y * t;
    }

    // Box blur term products across the window
    let sum_xx = box_blur_separable(&ixx, w, h, radius);
    let sum_yy = box_blur_separable(&iyy, w, h, radius);
    let sum_xy = box_blur_separable(&ixy, w, h, radius);
    let sum_xt = box_blur_separable(&ixt, w, h, radius);
    let sum_yt = box_blur_separable(&iyt, w, h, radius);

    // Solve 2x2 system per pixel in parallel
    field.u.par_iter_mut()
        .zip(field.v.par_iter_mut())
        .enumerate()
        .for_each(|(i, (u, v))| {
            let a00 = sum_xx[i] + lambda;
            let a11 = sum_yy[i] + lambda;
            let a01 = sum_xy[i];

            let b0 = -sum_xt[i];
            let b1 = -sum_yt[i];

            let det = a00 * a11 - a01 * a01;
            if det > 1e-6 {
                let du = (a11 * b0 - a01 * b1) / det;
                let dv = (a00 * b1 - a01 * b0) / det;

                // Clamp step delta to avoid overshoot
                let step_limit = 2.0;
                *u += du.clamp(-step_limit, step_limit);
                *v += dv.clamp(-step_limit, step_limit);
            }
        });
}

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

            for c in 1..w {
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
                row_out[c] = if count > 0.0 { sum / count } else { 0.0 };
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

/// Diffuse velocity field into clear-air boundary zones surrounding echoes.
/// Uses iterative normalized convolution so empty space downstream of storms
/// inherits the storm velocity.
fn diffuse_velocity_field(field: &mut MotionField, radar_data: &[f32], iterations: usize) {
    let w = field.width;
    let h = field.height;

    // Weight mask: 1.0 where radar echo is present (> 5.0 raw level), 0.0 otherwise
    let mut mask = vec![0.0f32; w * h];
    for i in 0..w * h {
        if radar_data[i] > 5.0 {
            mask[i] = 1.0;
        }
    }

    let mut u_diff = field.u.clone();
    let mut v_diff = field.v.clone();
    let mut cur_mask = mask.clone();

    for _ in 0..iterations {
        let blurred_mask = box_blur_separable(&cur_mask, w, h, 2);
        let mut weighted_u = vec![0.0f32; w * h];
        let mut weighted_v = vec![0.0f32; w * h];
        for i in 0..w * h {
            weighted_u[i] = u_diff[i] * cur_mask[i];
            weighted_v[i] = v_diff[i] * cur_mask[i];
        }

        let blurred_u = box_blur_separable(&weighted_u, w, h, 2);
        let blurred_v = box_blur_separable(&weighted_v, w, h, 2);

        for i in 0..w * h {
            if cur_mask[i] < 0.1 && blurred_mask[i] > 1e-4 {
                u_diff[i] = blurred_u[i] / blurred_mask[i];
                v_diff[i] = blurred_v[i] / blurred_mask[i];
                cur_mask[i] = (blurred_mask[i] * 1.5).min(1.0);
            }
        }
    }

    // Apply diffused velocities to empty regions in the motion field
    for i in 0..w * h {
        if mask[i] < 0.1 && cur_mask[i] > 0.01 {
            field.u[i] = u_diff[i];
            field.v[i] = v_diff[i];
        }
    }
}
