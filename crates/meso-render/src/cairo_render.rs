/*
 * Cairo CPU rendering backend.
 *
 * Implements the full radar render pipeline using cairo-rs for CPU-only
 * systems (no GPU / Mesa fallback failed).
 *
 * Renders:
 * - Radial quad geometry → cairo fill paths
 * - Range rings → arc strokes
 * - Warning polygons → line strokes
 * - Background → solid black fill
 */

use anyhow::Result;
use cairo::{Context, Format, ImageSurface};

use meso_data::map_data::MapData;
use meso_data::radar::color_palette::ColorPalette;
use meso_data::radar::level2::Level2Data;
use meso_data::radar::level3::Level3Data;

use crate::frame::RenderedImage;
use crate::geometry::{level2_to_quads, level3_to_quads, QuadBuffer};
use crate::overlay::{OverlaySet, Polyline};
use crate::viewport::Viewport;

// ── Public render functions ───────────────────────────────────────────────────

/// Render a Level 2 scan to a CPU image using Cairo.
pub fn render_level2(
    data: &Level2Data,
    palette: &ColorPalette,
    viewport: &Viewport,
    overlays: &OverlaySet,
    is_velocity: bool,
    map: Option<&MapData>,
) -> Result<RenderedImage> {
    let quads = level2_to_quads(data, palette, viewport, is_velocity);
    render_quads_cairo(&quads, viewport, overlays, map)
}

/// Render a Level 3 scan to a CPU image using Cairo.
pub fn render_level3(
    data: &Level3Data,
    palette: &ColorPalette,
    viewport: &Viewport,
    overlays: &OverlaySet,
    is_velocity: bool,
    map: Option<&MapData>,
) -> Result<RenderedImage> {
    let quads = level3_to_quads(data, palette, viewport, is_velocity);
    render_quads_cairo(&quads, viewport, overlays, map)
}

// ── Internal implementation ───────────────────────────────────────────────────

fn render_quads_cairo(
    quads: &QuadBuffer,
    viewport: &Viewport,
    overlays: &OverlaySet,
    map: Option<&MapData>,
) -> Result<RenderedImage> {
    let w = viewport.width as i32;
    let h = viewport.height as i32;

    let mut surface = ImageSurface::create(Format::ARgb32, w, h)?;

    // All drawing in its own scope so ctx is dropped before we access surface data
    {
        let ctx = Context::new(&surface)?;

        // Black background
        ctx.set_source_rgb(0.0, 0.0, 0.0);
        ctx.paint()?;

        // Draw radar quads
        let positions = &quads.positions;
        let colors = &quads.colors;
        let n = quads.quad_count;

        for i in 0..n {
            let base_p = i * 8;
            let base_c = i * 12;
            if base_p + 7 >= positions.len() || base_c + 2 >= colors.len() {
                break;
            }
            let r = colors[base_c] as f64 / 255.0;
            let g = colors[base_c + 1] as f64 / 255.0;
            let b = colors[base_c + 2] as f64 / 255.0;

            let (x0, y0) = (positions[base_p] as f64, positions[base_p + 1] as f64);
            let (x1, y1) = (positions[base_p + 2] as f64, positions[base_p + 3] as f64);
            let (x2, y2) = (positions[base_p + 4] as f64, positions[base_p + 5] as f64);
            let (x3, y3) = (positions[base_p + 6] as f64, positions[base_p + 7] as f64);

            ctx.set_source_rgb(r, g, b);
            ctx.move_to(x0, y0);
            ctx.line_to(x1, y1);
            ctx.line_to(x2, y2);
            ctx.line_to(x3, y3);
            ctx.close_path();
            ctx.fill()?;
        }

        // Draw range rings (centered on the radar site, not the widget center)
        let (site_sx, site_sy) = viewport.latlon_to_screen(&viewport.site_origin);
        let ppkm = viewport.pixels_per_km();

        if overlays.rings_visible {
            for ring in &overlays.range_rings {
                let (r, g, b) = ring.color;
                ctx.set_source_rgba(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 0.5);
                ctx.set_line_width(ring.line_width as f64);
                let radius_px = ring.radius_km * ppkm;
                ctx.arc(site_sx, site_sy, radius_px, 0.0, 2.0 * std::f64::consts::PI);
                ctx.stroke()?;
            }
        }

        // Draw map layers (counties, states, lakes, cities)
        if let Some(map) = map {
            draw_map_cairo(&ctx, map, viewport)?;
        }

        // Draw visible overlay layers
        for layer in &overlays.layers {
            if !layer.visible {
                continue;
            }
            for poly in &layer.polylines {
                draw_polyline_cairo(&ctx, poly, viewport)?;
            }
            for cell in &layer.storm_cells {
                let ll = meso_data::geo::latlon::LatLon {
                    lat: cell.lat as f64,
                    lon: cell.lon as f64,
                };
                let (cx, cy) = viewport.latlon_to_screen(&ll);
                let cx = cx as f64;
                let cy = cy as f64;

                // The STI bearing is a FROM-direction (met convention); add 180° to get
                // the actual direction of motion.
                let motion_bearing = (cell.bearing_deg as f64 + 180.0) % 360.0;
                let speed_kt = cell.speed_kt as f64;

                ctx.set_source_rgb(1.0, 1.0, 1.0);
                ctx.set_line_width(1.8);

                // Main vector line: current position → 60-min position
                let (end_lat, end_lon) = meso_data::storm_tracks::bearing_point(
                    ll.lat, ll.lon, motion_bearing, speed_kt * 1852.0,
                );
                let end_ll = meso_data::geo::latlon::LatLon { lat: end_lat, lon: end_lon };
                let (ex, ey) = viewport.latlon_to_screen(&end_ll);
                ctx.move_to(cx, cy);
                ctx.line_to(ex as f64, ey as f64);
                let _ = ctx.stroke();

                // Cell dot
                ctx.arc(cx, cy, 4.0, 0.0, std::f64::consts::TAU);
                let _ = ctx.fill();

                // Arrowhead at 60-min tip
                if speed_kt > 0.5 {
                    let head_len = 8.0;
                    let rev = (motion_bearing + 180.0).to_radians();
                    for off in [-25.0_f64, 25.0] {
                        let a = rev + off.to_radians();
                        ctx.move_to(ex as f64, ey as f64);
                        ctx.line_to(ex as f64 + a.sin() * head_len, ey as f64 - a.cos() * head_len);
                        let _ = ctx.stroke();
                    }
                }

                // Dots at 15, 30, 45 min positions
                for &frac in &[0.25_f64, 0.50, 0.75] {
                    let dist = speed_kt * 1852.0 * frac;
                    let (tlat, tlon) = meso_data::storm_tracks::bearing_point(
                        ll.lat, ll.lon, motion_bearing, dist,
                    );
                    let tick_ll = meso_data::geo::latlon::LatLon { lat: tlat, lon: tlon };
                    let (tx, ty) = viewport.latlon_to_screen(&tick_ll);
                    ctx.arc(tx as f64, ty as f64, 2.5, 0.0, std::f64::consts::TAU);
                    let _ = ctx.fill();
                }

                // Storm ID label
                ctx.set_font_size(9.0);
                ctx.move_to(cx + 6.0, cy - 6.0);
                let _ = ctx.show_text(&cell.id);
            }
        }
    } // ctx dropped here

    // Extract pixel data from surface
    let data_surf = surface.data()?;
    // Cairo ARGB32 is premultiplied BGRA in native byte order → convert to RGBA
    let mut out = RenderedImage::new(viewport.width, viewport.height);
    for (i, chunk) in data_surf.chunks(4).enumerate() {
        let b = chunk[0];
        let g = chunk[1];
        let r = chunk[2];
        let a = chunk[3];
        let idx = i * 4;
        if idx + 3 < out.data.len() {
            out.data[idx] = r;
            out.data[idx + 1] = g;
            out.data[idx + 2] = b;
            out.data[idx + 3] = a;
        }
    }

    Ok(out)
}

fn draw_polyline_cairo(ctx: &Context, poly: &Polyline, viewport: &Viewport) -> Result<()> {
    if poly.points.is_empty() {
        return Ok(());
    }
    let (r, g, b) = poly.color;
    ctx.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    ctx.set_line_width(poly.line_width as f64);

    // Points in the warnings layer are stored as (lat, lon) — convert to screen
    let (x0, y0) = viewport.latlon_to_screen(&meso_data::geo::latlon::LatLon {
        lat: poly.points[0].0 as f64,
        lon: poly.points[0].1 as f64,
    });
    ctx.move_to(x0, y0);
    for pt in poly.points.iter().skip(1) {
        let (x, y) = viewport.latlon_to_screen(&meso_data::geo::latlon::LatLon {
            lat: pt.0 as f64,
            lon: pt.1 as f64,
        });
        ctx.line_to(x, y);
    }
    if poly.closed {
        ctx.close_path();
    }
    ctx.stroke()?;
    Ok(())
}

/// Draw all map geometry layers (counties, states, lakes, cities) using Cairo.
fn draw_map_cairo(ctx: &Context, map: &MapData, viewport: &Viewport) -> Result<()> {
    let w = viewport.width as f64;
    let h = viewport.height as f64;

    // Helper: check if a segment might be visible (rough bounding check)
    let in_view = |lat1: f32, lon1: f32, lat2: f32, lon2: f32| -> bool {
        let (x1, y1) = viewport.latlon_to_screen(&meso_data::geo::latlon::LatLon {
            lat: lat1 as f64,
            lon: lon1 as f64,
        });
        let (x2, y2) = viewport.latlon_to_screen(&meso_data::geo::latlon::LatLon {
            lat: lat2 as f64,
            lon: lon2 as f64,
        });
        let margin = 20.0_f64;
        let min_x = x1.min(x2);
        let max_x = x1.max(x2);
        let min_y = y1.min(y2);
        let max_y = y1.max(y2);
        max_x >= -margin && min_x <= w + margin && max_y >= -margin && min_y <= h + margin
    };

    let draw_segments = |ctx: &Context, segs: &[meso_data::map_data::GeoSegment]| -> Result<()> {
        for seg in segs {
            if !in_view(seg.lat1, seg.lon1, seg.lat2, seg.lon2) {
                continue;
            }
            let (x1, y1) = viewport.latlon_to_screen(&meso_data::geo::latlon::LatLon {
                lat: seg.lat1 as f64,
                lon: seg.lon1 as f64,
            });
            let (x2, y2) = viewport.latlon_to_screen(&meso_data::geo::latlon::LatLon {
                lat: seg.lat2 as f64,
                lon: seg.lon2 as f64,
            });
            ctx.move_to(x1, y1);
            ctx.line_to(x2, y2);
        }
        ctx.stroke()?;
        Ok(())
    };

    // County lines — dark gray, thinner
    ctx.set_source_rgba(0.45, 0.45, 0.45, 0.85);
    ctx.set_line_width(0.8);
    draw_segments(ctx, &map.counties)?;

    // State lines — brighter gray/white, thicker
    ctx.set_source_rgba(0.85, 0.85, 0.85, 0.95);
    ctx.set_line_width(1.5);
    draw_segments(ctx, &map.states)?;

    // Lakes/rivers — light blue
    ctx.set_source_rgba(0.4, 0.6, 0.9, 0.85);
    ctx.set_line_width(1.0);
    draw_segments(ctx, &map.lakes)?;

    // City labels — LOD tuned by zoom (more/smaller towns as users zoom in)
    let zoom = viewport.zoom;
    let (pop_threshold, max_labels, min_sep_px) = city_lod_params(zoom);
    ctx.set_source_rgba(1.0, 1.0, 0.6, 1.0);
    ctx.set_font_size((9.0 + zoom.min(5.0) * 0.7).clamp(8.5, 12.5));
    let mut placed: Vec<(f64, f64)> = Vec::new();

    for city in &map.cities {
        if city.population < pop_threshold {
            break; // sorted descending, no need to continue
        }
        let (cx, cy) = viewport.latlon_to_screen(&meso_data::geo::latlon::LatLon {
            lat: city.lat as f64,
            lon: city.lon as f64,
        });
        if cx < -10.0 || cx > w + 10.0 || cy < -10.0 || cy > h + 10.0 {
            continue;
        }
        if placed.len() >= max_labels {
            break;
        }

        // Avoid dense overlaps by requiring a minimum screen-space separation.
        let too_close = placed.iter().any(|(px, py)| {
            let dx = cx - *px;
            let dy = cy - *py;
            (dx * dx + dy * dy) < (min_sep_px * min_sep_px)
        });
        if too_close {
            continue;
        }
        placed.push((cx, cy));

        // Draw a small dot at city location
        ctx.arc(cx, cy, 2.0, 0.0, 2.0 * std::f64::consts::PI);
        ctx.fill()?;
        // Label slightly offset
        ctx.move_to(cx + 3.0, cy - 3.0);
        ctx.show_text(&city.name)?;
    }

    Ok(())
}

fn city_lod_params(zoom: f64) -> (u32, usize, f64) {
    if zoom < 0.7 {
        (1_500_000, 20, 78.0)
    } else if zoom < 1.0 {
        (800_000, 30, 66.0)
    } else if zoom < 1.5 {
        (300_000, 55, 54.0)
    } else if zoom < 2.5 {
        (120_000, 90, 44.0)
    } else if zoom < 4.0 {
        (50_000, 140, 34.0)
    } else if zoom < 6.0 {
        (20_000, 220, 26.0)
    } else {
        (5_000, 320, 20.0)
    }
}
