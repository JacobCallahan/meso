/*
 * RadarGrid: regular Cartesian 2D grid representation of radar reflectivity/velocity.
 *
 * Provides bidirectional conversion between:
 * - Level2Data (polar radials) <-> RadarGrid (Cartesian 2D)
 * - Level3Data (polar radials or BA07 raster) <-> RadarGrid (Cartesian 2D)
 */

use meso_data::radar::level2::{Level2Data, NUM_RANGE_BINS};
use meso_data::radar::level3::Level3Data;
use rayon::prelude::*;

/// A 2D Cartesian grid centered on the radar site.
/// Origin (0 km, 0 km) is the radar site.
/// x is East (+), West (-).
/// y is North (+), South (-).
/// Row 0 is at maximum North (+y), increasing row index goes South (-y).
/// Col 0 is at maximum West (-x), increasing col index goes East (+x).
#[derive(Debug, Clone)]
pub struct RadarGrid {
    pub width: usize,
    pub height: usize,
    pub cell_km: f32,
    pub range_km: f32,
    pub data: Vec<f32>,
    pub timestamp_secs: i64,
}

impl RadarGrid {
    /// Create a new blank RadarGrid with the specified dimensions.
    pub fn new(width: usize, height: usize, cell_km: f32, timestamp_secs: i64) -> Self {
        let range_km = (width as f32 / 2.0) * cell_km;
        Self {
            width,
            height,
            cell_km,
            range_km,
            data: vec![0.0; width * height],
            timestamp_secs,
        }
    }

    /// Convert a grid cell (col, row) to radar-relative kilometer coordinates (x_km, y_km).
    #[inline]
    pub fn cell_to_km(&self, col: f32, row: f32) -> (f32, f32) {
        let half_w = self.width as f32 / 2.0;
        let half_h = self.height as f32 / 2.0;
        let x_km = (col - half_w) * self.cell_km;
        let y_km = (half_h - row) * self.cell_km;
        (x_km, y_km)
    }

    /// Convert radar-relative kilometer coordinates (x_km, y_km) to continuous grid coordinates (col, row).
    #[inline]
    pub fn km_to_cell(&self, x_km: f32, y_km: f32) -> (f32, f32) {
        let half_w = self.width as f32 / 2.0;
        let half_h = self.height as f32 / 2.0;
        let col = x_km / self.cell_km + half_w;
        let row = half_h - y_km / self.cell_km;
        (col, row)
    }

    /// Sample the grid at continuous grid coordinates (col, row) using bilinear interpolation.
    #[inline]
    pub fn sample_bilinear(&self, col: f32, row: f32) -> f32 {
        if col < 0.0 || row < 0.0 || col >= (self.width - 1) as f32 || row >= (self.height - 1) as f32 {
            if col >= -0.5 && col <= self.width as f32 - 0.5 && row >= -0.5 && row <= self.height as f32 - 0.5 {
                let c = col.clamp(0.0, (self.width - 1) as f32) as usize;
                let r = row.clamp(0.0, (self.height - 1) as f32) as usize;
                return self.data[r * self.width + c];
            }
            return 0.0;
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

        let v00 = self.data[idx00];
        let v01 = self.data[idx01];
        let v10 = self.data[idx10];
        let v11 = self.data[idx11];

        let top = v00 * (1.0 - fc) + v01 * fc;
        let bottom = v10 * (1.0 - fc) + v11 * fc;

        top * (1.0 - fr) + bottom * fr
    }

    /// Sample the grid at radar-relative kilometer coordinates (x_km, y_km) using bilinear interpolation.
    #[inline]
    pub fn sample_km(&self, x_km: f32, y_km: f32) -> f32 {
        let (col, row) = self.km_to_cell(x_km, y_km);
        self.sample_bilinear(col, row)
    }

    /// Create a RadarGrid from Level2Data.
    pub fn from_level2(l2: &Level2Data, target_cell_km: Option<f32>) -> Self {
        let bin_size = l2.bin_size_km;
        let max_range = NUM_RANGE_BINS as f32 * bin_size;
        // Default cell size: 0.5 km for Level 2 super-res (balanced speed & fidelity)
        let cell_km = target_cell_km.unwrap_or(0.5).max(0.25);
        let grid_size = ((max_range * 2.0) / cell_km).ceil() as usize;
        let grid_size = (grid_size | 1).max(256); // ensure odd size for exact center pixel

        let timestamp_secs = {
            let days = l2.julian_date as i64 - 1;
            days * 86400 + l2.millis as i64 / 1000
        };

        let mut grid = Self::new(grid_size, grid_size, cell_km, timestamp_secs);

        let num_radials = l2.azimuths.len();
        if num_radials == 0 {
            return grid;
        }

        // Parallel rasterization across rows
        let width = grid.width;
        let height = grid.height;
        let half_w = width as f32 / 2.0;
        let half_h = height as f32 / 2.0;

        grid.data
            .par_chunks_exact_mut(width)
            .enumerate()
            .for_each(|(row, row_slice)| {
                let y_km = (half_h - row as f32) * cell_km;
                for col in 0..width {
                    let x_km = (col as f32 - half_w) * cell_km;
                    let range = (x_km * x_km + y_km * y_km).sqrt();

                    if range < 1.0 || range >= max_range {
                        row_slice[col] = 0.0;
                        continue;
                    }

                    // Azimuth in degrees clockwise from North
                    let mut az = x_km.atan2(y_km).to_degrees();
                    if az < 0.0 {
                        az += 360.0;
                    }

                    let val = sample_polar_l2(l2, range, az);
                    row_slice[col] = val;
                }
            });

        grid
    }

    /// Create a RadarGrid from Level3Data.
    pub fn from_level3(l3: &Level3Data, target_cell_km: Option<f32>) -> Self {
        let timestamp_secs = {
            let days = l3.scan_date as i64 - 1;
            days * 86400 + l3.scan_time_secs as i64
        };

        if l3.is_raster {
            let rows = l3.num_radials;
            let cols = l3.num_range_bins;
            let cell_km = l3.bin_size_km;
            let mut grid = Self::new(cols, rows, cell_km, timestamp_secs);
            for r in 0..rows {
                for c in 0..cols {
                    grid.data[r * cols + c] = l3.bins[r * cols + c] as f32;
                }
            }
            return grid;
        }

        let bin_size = l3.bin_size_km;
        let max_range = l3.num_range_bins as f32 * bin_size;
        let cell_km = target_cell_km.unwrap_or(1.0).max(0.5);
        let grid_size = ((max_range * 2.0) / cell_km).ceil() as usize;
        let grid_size = (grid_size | 1).max(256);

        let mut grid = Self::new(grid_size, grid_size, cell_km, timestamp_secs);
        let num_radials = l3.num_radials;
        if num_radials == 0 {
            return grid;
        }

        let width = grid.width;
        let height = grid.height;
        let half_w = width as f32 / 2.0;
        let half_h = height as f32 / 2.0;

        grid.data
            .par_chunks_exact_mut(width)
            .enumerate()
            .for_each(|(row, row_slice)| {
                let y_km = (half_h - row as f32) * cell_km;
                for col in 0..width {
                    let x_km = (col as f32 - half_w) * cell_km;
                    let range = (x_km * x_km + y_km * y_km).sqrt();

                    if range < 1.0 || range >= max_range {
                        row_slice[col] = 0.0;
                        continue;
                    }

                    let mut az = x_km.atan2(y_km).to_degrees();
                    if az < 0.0 {
                        az += 360.0;
                    }

                    let val = sample_polar_l3(l3, range, az);
                    row_slice[col] = val;
                }
            });

        grid
    }

    /// Resample this RadarGrid back into a Level2Data structure based on a template frame.
    pub fn to_level2(&self, template: &Level2Data, forecast_secs: i64) -> Level2Data {
        let num_radials = template.azimuths.len();
        let mut bins = vec![0u8; num_radials * NUM_RANGE_BINS];
        let bin_size = template.bin_size_km;

        bins.par_chunks_exact_mut(NUM_RANGE_BINS)
            .enumerate()
            .for_each(|(r, radial_bins)| {
                let az_deg = template.azimuths[r];
                let rad = az_deg.to_radians();
                let sin_az = rad.sin();
                let cos_az = rad.cos();

                for b in 0..NUM_RANGE_BINS {
                    let range_km = b as f32 * bin_size;
                    let x_km = range_km * sin_az;
                    let y_km = range_km * cos_az;

                    let val = self.sample_km(x_km, y_km);
                    radial_bins[b] = val.round().clamp(0.0, 255.0) as u8;
                }
            });

        // Compute updated julian date and millis
        let new_epoch = self.timestamp_secs + forecast_secs;
        let julian_date = (new_epoch / 86400 + 1) as i16;
        let millis = ((new_epoch % 86400) * 1000) as i32;

        Level2Data {
            azimuths: template.azimuths.clone(),
            bins,
            julian_date,
            millis,
            bin_size_km: template.bin_size_km,
            num_radials: template.num_radials,
            elevation_angle_deg: template.elevation_angle_deg,
        }
    }

    /// Resample this RadarGrid back into a Level3Data structure based on a template frame.
    pub fn to_level3(&self, template: &Level3Data, forecast_secs: i64) -> Level3Data {
        let new_epoch = self.timestamp_secs + forecast_secs;
        let scan_date = (new_epoch / 86400 + 1) as u16;
        let scan_time_secs = (new_epoch % 86400) as u32;

        if template.is_raster {
            let rows = template.num_radials;
            let cols = template.num_range_bins;
            let mut bins = vec![0u8; rows * cols];

            for r in 0..rows {
                for c in 0..cols {
                    let val = self.data.get(r * cols + c).copied().unwrap_or(0.0);
                    bins[r * cols + c] = val.round().clamp(0.0, 255.0) as u8;
                }
            }

            return Level3Data {
                num_radials: rows,
                num_range_bins: cols,
                azimuths: Vec::new(),
                bins,
                bin_size_km: template.bin_size_km,
                is_raster: true,
                product_code: template.product_code,
                vcp: template.vcp,
                scan_date,
                scan_time_secs,
            };
        }

        let num_radials = template.num_radials;
        let num_bins = template.num_range_bins;
        let mut bins = vec![0u8; num_radials * num_bins];
        let bin_size = template.bin_size_km;

        bins.par_chunks_exact_mut(num_bins)
            .enumerate()
            .for_each(|(r, radial_bins)| {
                let az_deg = template.azimuths[r];
                let rad = az_deg.to_radians();
                let sin_az = rad.sin();
                let cos_az = rad.cos();

                for b in 0..num_bins {
                    let range_km = b as f32 * bin_size;
                    let x_km = range_km * sin_az;
                    let y_km = range_km * cos_az;

                    let val = self.sample_km(x_km, y_km);
                    radial_bins[b] = val.round().clamp(0.0, 255.0) as u8;
                }
            });

        Level3Data {
            num_radials,
            num_range_bins: num_bins,
            azimuths: template.azimuths.clone(),
            bins,
            bin_size_km: template.bin_size_km,
            is_raster: false,
            product_code: template.product_code,
            vcp: template.vcp,
            scan_date,
            scan_time_secs,
        }
    }
}

/// Helper: Sample polar Level2Data with angular and range interpolation.
fn sample_polar_l2(l2: &Level2Data, range: f32, az: f32) -> f32 {
    let _num_radials = l2.azimuths.len();
    let bin_size = l2.bin_size_km;
    let b_f = range / bin_size;
    let b0 = b_f.floor() as usize;
    if b0 >= NUM_RANGE_BINS - 1 {
        return 0.0;
    }
    let b1 = b0 + 1;
    let fb = b_f - b0 as f32;

    // Find bounding radials
    let (r0, r1, faz) = find_bounding_radials(&l2.azimuths, az);

    let v00 = l2.bins[r0 * NUM_RANGE_BINS + b0] as f32;
    let v01 = l2.bins[r0 * NUM_RANGE_BINS + b1] as f32;
    let v10 = l2.bins[r1 * NUM_RANGE_BINS + b0] as f32;
    let v11 = l2.bins[r1 * NUM_RANGE_BINS + b1] as f32;

    let rad0 = v00 * (1.0 - fb) + v01 * fb;
    let rad1 = v10 * (1.0 - fb) + v11 * fb;

    rad0 * (1.0 - faz) + rad1 * faz
}

/// Helper: Sample polar Level3Data with angular and range interpolation.
fn sample_polar_l3(l3: &Level3Data, range: f32, az: f32) -> f32 {
    let _num_radials = l3.num_radials;
    let num_bins = l3.num_range_bins;
    let bin_size = l3.bin_size_km;
    let b_f = range / bin_size;
    let b0 = b_f.floor() as usize;
    if b0 >= num_bins - 1 {
        return 0.0;
    }
    let b1 = b0 + 1;
    let fb = b_f - b0 as f32;

    let (r0, r1, faz) = find_bounding_radials(&l3.azimuths, az);

    let v00 = l3.bins[r0 * num_bins + b0] as f32;
    let v01 = l3.bins[r0 * num_bins + b1] as f32;
    let v10 = l3.bins[r1 * num_bins + b0] as f32;
    let v11 = l3.bins[r1 * num_bins + b1] as f32;

    let rad0 = v00 * (1.0 - fb) + v01 * fb;
    let rad1 = v10 * (1.0 - fb) + v11 * fb;

    rad0 * (1.0 - faz) + rad1 * faz
}

/// Find the indices of the two radials that bracket the given azimuth (0..360),
/// and the fractional position between them [0.0, 1.0).
fn find_bounding_radials(azimuths: &[f32], target_az: f32) -> (usize, usize, f32) {
    let n = azimuths.len();
    if n == 0 {
        return (0, 0, 0.0);
    }
    if n == 1 {
        return (0, 0, 0.0);
    }

    // Binary search or linear scan since radials are monotonically increasing (modulo 360)
    for i in 0..n {
        let a0 = azimuths[i];
        let next_idx = (i + 1) % n;
        let a1 = azimuths[next_idx];

        let span = if a1 >= a0 {
            a1 - a0
        } else {
            a1 + 360.0 - a0
        };

        let diff = if target_az >= a0 {
            target_az - a0
        } else {
            target_az + 360.0 - a0
        };

        if diff <= span && span > 0.0 {
            let frac = (diff / span).clamp(0.0, 1.0);
            return (i, next_idx, frac);
        }
    }

    (0, 1, 0.0)
}
