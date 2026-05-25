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
