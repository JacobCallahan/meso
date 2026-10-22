pub mod advect;
pub mod gpu;
pub mod grid;
pub mod nowcast;
pub mod optical_flow;

pub use advect::{
    advect_nowcast, estimate_decay_factor, NowcastConfig, DEFAULT_DIFFUSION_KM2_PER_MIN,
    MAX_DIFFUSION_KM2_PER_MIN,
};
pub use grid::RadarGrid;
pub use nowcast::{
    run_nowcast_level2, run_nowcast_level2_with_progress, run_nowcast_level3,
    run_nowcast_level3_with_progress, NowcastResult,
};
pub use optical_flow::{estimate_motion_pair, estimate_motion_sequence, MotionField};

#[cfg(test)]
mod tests {
    use super::*;
    use meso_data::radar::level3::Level3Data;

    fn make_test_l3(scan_date: u16, scan_time_secs: u32, shift_x: f32, shift_y: f32) -> Level3Data {
        let num_radials = 360;
        let num_bins = 200;
        let bin_size_km = 1.0;
        let mut azimuths = Vec::with_capacity(num_radials);
        for i in 0..num_radials {
            azimuths.push(i as f32);
        }

        let mut bins = vec![0u8; num_radials * num_bins];

        // Synthesize a gaussian echo centered at (50 + shift_x, 50 + shift_y) km
        let center_x = 50.0 + shift_x;
        let center_y = 50.0 + shift_y;
        let radius = 15.0;

        for r in 0..num_radials {
            let az_rad = (r as f32).to_radians();
            let sin_az = az_rad.sin();
            let cos_az = az_rad.cos();

            for b in 0..num_bins {
                let range = b as f32 * bin_size_km;
                let x = range * sin_az;
                let y = range * cos_az;

                let dist = ((x - center_x).powi(2) + (y - center_y).powi(2)).sqrt();
                if dist < radius {
                    let intensity = 120.0 * (1.0 - dist / radius);
                    bins[r * num_bins + b] = intensity.clamp(0.0, 255.0) as u8;
                }
            }
        }

        Level3Data {
            num_radials,
            num_range_bins: num_bins,
            azimuths,
            bins,
            bin_size_km,
            is_raster: false,
            product_code: 94,
            vcp: 212,
            scan_date,
            scan_time_secs,
        }
    }

    #[test]
    fn test_grid_conversion_and_sampling() {
        let l3 = make_test_l3(19000, 36000, 0.0, 0.0);
        let grid = RadarGrid::from_level3(&l3, Some(1.0));
        assert!(grid.width >= 200);
        assert!(grid.height >= 200);

        // Center should sample near 0
        let center_val = grid.sample_km(0.0, 0.0);
        assert!(center_val < 5.0);

        // Echo center at (50, 50) km should have strong intensity
        let echo_val = grid.sample_km(50.0, 50.0);
        assert!(echo_val > 50.0, "Expected strong echo, got {echo_val}");

        // Convert back to Level3
        let recon = grid.to_level3(&l3, 300);
        assert_eq!(recon.num_radials, l3.num_radials);
        assert_eq!(recon.num_range_bins, l3.num_range_bins);
    }

    #[test]
    fn test_optical_flow_translation() {
        let f0 = make_test_l3(19000, 36000, 0.0, 0.0);
        let f1 = make_test_l3(19000, 36300, 4.0, 3.0); // moved +4km East, +3km North

        let g0 = RadarGrid::from_level3(&f0, Some(1.0));
        let g1 = RadarGrid::from_level3(&f1, Some(1.0));

        let motion = estimate_motion_pair(&g0, &g1).unwrap();

        // Check motion at the storm location (50 km, 50 km)
        let (col, row) = g0.km_to_cell(50.0, 50.0);
        let (u, v) = motion.sample_uv(col, row);

        // u is East (+) in cells: +4km / 1.0 km/cell = +4 cells
        // v is South (+) in cells, but movement is North (+3km), so v should be negative ~ -3 cells
        assert!(u > 1.5, "Expected positive eastward u, got {u}");
        assert!(v < -1.0, "Expected negative northward v, got {v}");
    }

    #[test]
    fn test_multi_directional_flow() {
        // Create grid with cell A moving East and cell B moving North
        let mut g0 = RadarGrid::new(200, 200, 1.0, 1000);
        let mut g1 = RadarGrid::new(200, 200, 1.0, 1300);

        // Cell A at (50, 50): moves +5 in x (East)
        // Cell B at (150, 150): moves +5 in y (North => row decreases by 5)
        for r in 0..200 {
            for c in 0..200 {
                let dist_a0 = ((c as f32 - 50.0).powi(2) + (r as f32 - 50.0).powi(2)).sqrt();
                if dist_a0 < 10.0 {
                    g0.data[r * 200 + c] = 100.0 * (1.0 - dist_a0 / 10.0);
                }

                let dist_a1 = ((c as f32 - 55.0).powi(2) + (r as f32 - 50.0).powi(2)).sqrt();
                if dist_a1 < 10.0 {
                    g1.data[r * 200 + c] = 100.0 * (1.0 - dist_a1 / 10.0);
                }

                let dist_b0 = ((c as f32 - 150.0).powi(2) + (r as f32 - 150.0).powi(2)).sqrt();
                if dist_b0 < 10.0 {
                    g0.data[r * 200 + c] = 100.0 * (1.0 - dist_b0 / 10.0);
                }

                let dist_b1 = ((c as f32 - 150.0).powi(2) + (r as f32 - 145.0).powi(2)).sqrt();
                if dist_b1 < 10.0 {
                    g1.data[r * 200 + c] = 100.0 * (1.0 - dist_b1 / 10.0);
                }
            }
        }

        let motion = estimate_motion_pair(&g0, &g1).unwrap();

        // Sample at cell A
        let (u_a, v_a) = motion.sample_uv(50.0, 50.0);
        // Sample at cell B
        let (u_b, v_b) = motion.sample_uv(150.0, 150.0);

        // Cell A moves predominantly East (u > 0, |u| >> |v|)
        assert!(u_a > 2.0, "Cell A should move East, got u_a={u_a}");
        assert!(v_a.abs() < u_a, "Cell A motion should be mostly eastward");

        // Cell B moves North (v < 0, |v| >> |u|)
        assert!(v_b < -2.0, "Cell B should move North, got v_b={v_b}");
        assert!(
            u_b.abs() < v_b.abs(),
            "Cell B motion should be mostly northward"
        );
    }

    #[test]
    fn test_motion_sequence_preserves_separate_opposing_storms() {
        let mut g0 = RadarGrid::new(200, 200, 1.0, 1000);
        let mut g1 = RadarGrid::new(200, 200, 1.0, 1300);

        for r in 0..200 {
            for c in 0..200 {
                let echo = |cx: f32, cy: f32| {
                    let dist = ((c as f32 - cx).powi(2) + (r as f32 - cy).powi(2)).sqrt();
                    if dist < 10.0 {
                        100.0 * (1.0 - dist / 10.0)
                    } else {
                        0.0
                    }
                };
                g0.data[r * 200 + c] = echo(50.0, 50.0).max(echo(150.0, 150.0));
                g1.data[r * 200 + c] = echo(55.0, 50.0).max(echo(145.0, 150.0));
            }
        }

        let motion = estimate_motion_sequence(&[g0, g1]).unwrap();
        let (east_u, _) = motion.sample_uv(55.0, 50.0);
        let (west_u, _) = motion.sample_uv(145.0, 150.0);
        assert!(east_u > 2.5, "Eastbound storm was slowed to {east_u}");
        assert!(west_u < -2.5, "Westbound storm was slowed to {west_u}");
    }

    #[test]
    fn test_fast_echo_does_not_stall_at_late_lead_times() {
        let observed = vec![
            make_test_l3(19000, 36000, 0.0, 0.0),
            make_test_l3(19000, 36300, 5.0, 0.0),
            make_test_l3(19000, 36600, 10.0, 0.0),
        ];
        let result = run_nowcast_level3(&observed, Some(3600)).expect("Nowcast should succeed");

        // 12 steps * 5 km = +60 km from the last observed center (60 km) => 120 km
        let last = RadarGrid::from_level3(result.frames.last().unwrap(), Some(1.0));
        let at_expected = last.sample_km(120.0, 50.0);
        let at_stalled = last.sample_km(80.0, 50.0);
        assert!(
            at_expected > 40.0,
            "Echo dried up at late lead: {at_expected}"
        );
        assert!(
            at_expected > at_stalled,
            "Echo stalled: expected={at_expected}, stalled={at_stalled}"
        );
    }

    #[test]
    fn test_steady_echo_keeps_intensity_and_coherence() {
        let observed = vec![
            make_test_l3(19000, 36000, 0.0, 0.0),
            make_test_l3(19000, 36300, 3.0, 0.0),
            make_test_l3(19000, 36600, 6.0, 0.0),
        ];
        let result = run_nowcast_level3(&observed, Some(5400)).unwrap();
        let mass = |f: &Level3Data| {
            let g = RadarGrid::from_level3(f, Some(1.0));
            g.data.iter().map(|&b| b as f64).sum::<f64>()
        };
        let first = mass(&result.frames[0]);
        let last = mass(result.frames.last().unwrap());
        assert!(last > 0.8 * first, "echo mass eroded: {first} -> {last}");
    }

    #[test]
    fn test_estimated_decay_tracks_intensity_trend() {
        let steady = [
            make_test_l3(19000, 36000, 0.0, 0.0),
            make_test_l3(19000, 36300, 5.0, 0.0),
            make_test_l3(19000, 36600, 10.0, 0.0),
        ];
        let grids: Vec<RadarGrid> = steady
            .iter()
            .map(|l| RadarGrid::from_level3(l, Some(1.0)))
            .collect();
        let d = estimate_decay_factor(&grids);
        assert!(d > 0.985 && d <= 1.0, "steady echo decay {d}");

        let mut faded = grids.clone();
        for (i, g) in faded.iter_mut().enumerate() {
            let s = 0.9f32.powi(i as i32);
            g.data.iter_mut().for_each(|v| *v *= s);
        }
        let d = estimate_decay_factor(&faded);
        assert!(
            (0.995..0.998).contains(&d),
            "fading echo should decay within clamp, got {d}"
        );
    }

    #[test]
    fn test_advection_and_nowcast_pipeline() {
        let f0 = make_test_l3(19000, 36000, 0.0, 0.0);
        let f1 = make_test_l3(19000, 36300, 3.0, 0.0);
        let f2 = make_test_l3(19000, 36600, 6.0, 0.0);

        let observed = vec![f0, f1, f2];
        let mut progress = Vec::new();
        let result = run_nowcast_level3_with_progress(
            &observed,
            Some(3600),
            DEFAULT_DIFFUSION_KM2_PER_MIN,
            &mut |message| progress.push(message),
        )
        .expect("Nowcast should succeed");

        assert_eq!(result.interval_secs, 300);
        assert_eq!(result.frames.len(), 12); // 3600 / 300 = 12 frames
        assert_eq!(result.timestamps.len(), 12);
        assert!(result.timestamps[0].contains("Nowcast"));
        assert!(progress.contains(&"Rasterized observed scans 100%".to_string()));
        assert!(progress.contains(&"Estimated motion 100%".to_string()));
        assert!(progress.contains(&"Generated forecast frames 100%".to_string()));
        assert!(progress.contains(&"Converted forecast frames 100%".to_string()));

        // In the 4th forecast frame (+20 min = +12 km East from (50, 50) + 6 km = 68 km),
        // verify echo has moved eastward
        let grid_fc4 = RadarGrid::from_level3(&result.frames[3], Some(1.0));
        let val_at_orig = grid_fc4.sample_km(50.0, 50.0);
        let val_at_new = grid_fc4.sample_km(68.0, 50.0);

        assert!(
            val_at_new > val_at_orig,
            "Storm should have advected east to ~68km: orig={val_at_orig}, new={val_at_new}"
        );
    }
}
