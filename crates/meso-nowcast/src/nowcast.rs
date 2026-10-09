/*
 * High-level Nowcast Pipeline.
 *
 * Coordinates rasterization, multi-frame optical flow motion estimation,
 * semi-Lagrangian advection, and format reconstruction for both Level 2 and Level 3 data.
 */

use anyhow::{bail, Result};
use chrono::{Local, TimeZone, Utc};
use meso_data::radar::level2::Level2Data;
use meso_data::radar::level3::Level3Data;

use crate::advect::{advect_nowcast, NowcastConfig};
use crate::grid::RadarGrid;
use crate::optical_flow::estimate_motion_sequence;

/// Output of a nowcast execution.
pub struct NowcastResult<T> {
    /// Generated synthetic forecast radar frames.
    pub frames: Vec<T>,
    /// Formatted timestamp display strings (including +Xm Nowcast indicator).
    pub timestamps: Vec<String>,
    /// Time delta (seconds) between successive frames.
    pub interval_secs: i64,
}

/// Run radar nowcast on a sequence of observed Level 2 radar scans.
pub fn run_nowcast_level2(
    observed: &[Level2Data],
    target_duration_secs: Option<i64>,
) -> Result<NowcastResult<Level2Data>> {
    if observed.len() < 2 {
        bail!("Nowcasting requires at least 2 observed radar scans to estimate motion");
    }

    // Determine frame interval
    let mut intervals = Vec::new();
    for pair in observed.windows(2) {
        let t0 = epoch_l2(&pair[0]);
        let t1 = epoch_l2(&pair[1]);
        let dt = t1 - t0;
        if dt > 30 && dt < 3600 {
            intervals.push(dt);
        }
    }
    let interval_secs = if !intervals.is_empty() {
        intervals.sort_unstable();
        intervals[intervals.len() / 2]
    } else {
        300 // default 5 minutes
    };

    // Calculate forecast duration (clamped between 1 and 2 hours: 3600s .. 7200s)
    let first_epoch = epoch_l2(observed.first().unwrap());
    let last_epoch = epoch_l2(observed.last().unwrap());
    let observed_span = last_epoch - first_epoch;
    let duration = target_duration_secs.unwrap_or_else(|| {
        observed_span.clamp(3600, 7200)
    });

    let steps = ((duration as f32 / interval_secs as f32).round() as usize).max(2);

    // Rasterize observed frames to Cartesian grids
    let cell_km = (observed.last().unwrap().bin_size_km * 2.0).max(0.5); // 0.5 km resolution
    let grids: Vec<RadarGrid> = observed
        .iter()
        .map(|l2| RadarGrid::from_level2(l2, Some(cell_km)))
        .collect();

    // Estimate dense motion field across observed sequence
    let motion = estimate_motion_sequence(&grids);

    // Advect forward
    let last_grid = grids.last().unwrap();
    let config = NowcastConfig {
        steps,
        interval_secs,
        decay_factor: 0.996, // subtle ~5% decay per hour
    };
    let future_grids = advect_nowcast(last_grid, &motion, &config);

    // Convert back to Level2Data
    let template = observed.last().unwrap();
    let mut future_frames = Vec::with_capacity(steps);
    let mut future_timestamps = Vec::with_capacity(steps);

    for (i, fg) in future_grids.iter().enumerate() {
        let offset_secs = (i + 1) as i64 * interval_secs;
        let l2_frame = fg.to_level2(template, offset_secs);

        let epoch = fg.timestamp_secs;
        let dt_str = match Utc.timestamp_opt(epoch, 0).single() {
            Some(dt) => dt
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M %Z")
                .to_string(),
            None => format!("Forecast +{}s", offset_secs),
        };
        let mins_ahead = (offset_secs + 30) / 60;
        let ts_display = format!("{dt_str} (+{mins_ahead}m Nowcast)");

        future_frames.push(l2_frame);
        future_timestamps.push(ts_display);
    }

    Ok(NowcastResult {
        frames: future_frames,
        timestamps: future_timestamps,
        interval_secs,
    })
}

/// Run radar nowcast on a sequence of observed Level 3 radar scans.
pub fn run_nowcast_level3(
    observed: &[Level3Data],
    target_duration_secs: Option<i64>,
) -> Result<NowcastResult<Level3Data>> {
    if observed.len() < 2 {
        bail!("Nowcasting requires at least 2 observed radar scans to estimate motion");
    }

    // Determine frame interval
    let mut intervals = Vec::new();
    for pair in observed.windows(2) {
        let t0 = epoch_l3(&pair[0]);
        let t1 = epoch_l3(&pair[1]);
        let dt = t1 - t0;
        if dt > 30 && dt < 3600 {
            intervals.push(dt);
        }
    }
    let interval_secs = if !intervals.is_empty() {
        intervals.sort_unstable();
        intervals[intervals.len() / 2]
    } else {
        300 // default 5 minutes
    };

    let first_epoch = epoch_l3(observed.first().unwrap());
    let last_epoch = epoch_l3(observed.last().unwrap());
    let observed_span = last_epoch - first_epoch;
    let duration = target_duration_secs.unwrap_or_else(|| {
        observed_span.clamp(3600, 7200)
    });

    let steps = ((duration as f32 / interval_secs as f32).round() as usize).max(2);

    // Rasterize observed frames to Cartesian grids
    let cell_km = observed.last().unwrap().bin_size_km.max(0.5);
    let grids: Vec<RadarGrid> = observed
        .iter()
        .map(|l3| RadarGrid::from_level3(l3, Some(cell_km)))
        .collect();

    // Estimate dense motion field across observed sequence
    let motion = estimate_motion_sequence(&grids);

    // Advect forward
    let last_grid = grids.last().unwrap();
    let config = NowcastConfig {
        steps,
        interval_secs,
        decay_factor: 0.996,
    };
    let future_grids = advect_nowcast(last_grid, &motion, &config);

    // Convert back to Level3Data
    let template = observed.last().unwrap();
    let mut future_frames = Vec::with_capacity(steps);
    let mut future_timestamps = Vec::with_capacity(steps);

    for (i, fg) in future_grids.iter().enumerate() {
        let offset_secs = (i + 1) as i64 * interval_secs;
        let l3_frame = fg.to_level3(template, offset_secs);

        let epoch = fg.timestamp_secs;
        let dt_str = match Utc.timestamp_opt(epoch, 0).single() {
            Some(dt) => dt
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M %Z")
                .to_string(),
            None => format!("Forecast +{}s", offset_secs),
        };
        let mins_ahead = (offset_secs + 30) / 60;
        let ts_display = format!("{dt_str} (+{mins_ahead}m Nowcast)");

        future_frames.push(l3_frame);
        future_timestamps.push(ts_display);
    }

    Ok(NowcastResult {
        frames: future_frames,
        timestamps: future_timestamps,
        interval_secs,
    })
}

fn epoch_l2(l2: &Level2Data) -> i64 {
    let days = l2.julian_date as i64 - 1;
    days * 86400 + l2.millis as i64 / 1000
}

fn epoch_l3(l3: &Level3Data) -> i64 {
    let days = l3.scan_date as i64 - 1;
    days * 86400 + l3.scan_time_secs as i64
}
