use crate::geo::latlon::LatLon;

/// Convert a geographic lat/lon to Mercator screen pixel coordinates.
///
/// This is a direct port of `genMercator` from wX's JNI C code, used to
/// transform lat/lon overlay points (warnings, fronts, cities, etc.) into
/// the same coordinate space as the radar radial geometry.
///
/// * `center_x` / `center_y` — radar site lat/lon (center of projection)
/// * `x_img` / `y_img`       — pixel center of the image (e.g. 500.0 for a 1000-pixel canvas)
/// * `scale`                  — pixels per degree of latitude at the center lat (one_degree_scale_factor)
pub fn latlon_to_screen(
    point: &LatLon,
    center: &LatLon,
    x_img: f32,
    y_img: f32,
    scale: f32,
) -> (f32, f32) {
    let x = x_img + (point.lon - center.lon) as f32 * scale;
    let y = y_img - (point.lat - center.lat) as f32 * scale;
    (x, y)
}

/// Compute the `scale` (pixels per degree) for a given canvas height and zoom level.
///
/// `zoom` is expressed as degrees of latitude visible on screen (e.g. 4.0 means
/// the canvas spans 4 degrees of latitude total). A smaller zoom = more zoomed in.
pub fn compute_scale(canvas_height_px: f32, zoom_lat_degrees: f32) -> f32 {
    canvas_height_px / zoom_lat_degrees
}

/// Convert a (lat, lon) into the radial geometry coordinate space used by the
/// radar rendering engine.  Returns `(x, y)` in the same float range as the
/// vertex buffer produced by `generate_radial_geometry`.
///
/// This matches the Mercator projection used in `OglBuffers` / `genMercator.c`.
/// The coordinate system is centered at (0,0) with ±1.0 being the canvas edge.
pub fn latlon_to_gl(point: &LatLon, center: &LatLon, scale: f32) -> (f32, f32) {
    let x = (point.lon - center.lon) as f32 * scale;
    let y = (point.lat - center.lat) as f32 * scale;
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::latlon::LatLon;

    const CENTER: LatLon = LatLon {
        lat: 35.0,
        lon: -97.0,
    };

    #[test]
    fn latlon_to_screen_center_maps_to_image_center() {
        let (x, y) = latlon_to_screen(&CENTER, &CENTER, 500.0, 500.0, 100.0);
        assert!((x - 500.0).abs() < 0.001, "x={x}");
        assert!((y - 500.0).abs() < 0.001, "y={y}");
    }

    #[test]
    fn latlon_to_screen_point_north_has_smaller_y() {
        let north = LatLon::new(CENTER.lat + 1.0, CENTER.lon);
        let (_, y) = latlon_to_screen(&north, &CENTER, 500.0, 500.0, 100.0);
        // Screen y increases downward; north → smaller y value.
        assert!(
            y < 500.0,
            "point north of center should have y < 500.0, got {y}"
        );
    }

    #[test]
    fn latlon_to_screen_point_east_has_larger_x() {
        let east = LatLon::new(CENTER.lat, CENTER.lon + 1.0);
        let (x, _) = latlon_to_screen(&east, &CENTER, 500.0, 500.0, 100.0);
        assert!(
            x > 500.0,
            "point east of center should have x > 500.0, got {x}"
        );
    }

    #[test]
    fn compute_scale_pixels_per_degree() {
        // 1000 px canvas / 10° zoom = 100 px per degree.
        let scale = compute_scale(1000.0, 10.0);
        assert!((scale - 100.0).abs() < 0.001, "scale={scale}");
    }

    #[test]
    fn compute_scale_zoomed_in_gives_larger_value() {
        let wide = compute_scale(1000.0, 20.0);
        let narrow = compute_scale(1000.0, 4.0);
        assert!(narrow > wide, "smaller zoom window → larger px/degree");
    }

    #[test]
    fn latlon_to_gl_center_is_origin() {
        let (x, y) = latlon_to_gl(&CENTER, &CENTER, 100.0);
        assert!(x.abs() < 0.001 && y.abs() < 0.001);
    }

    #[test]
    fn latlon_to_gl_north_positive_y() {
        let north = LatLon::new(CENTER.lat + 1.0, CENTER.lon);
        let (_, y) = latlon_to_gl(&north, &CENTER, 100.0);
        assert!(
            y > 0.0,
            "north of center should have positive y in GL space"
        );
    }
}
