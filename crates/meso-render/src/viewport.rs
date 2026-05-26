/*
 * Viewport: defines the mapping between lat/lon space and screen pixels.
 *
 * The viewport tracks the center lat/lon, zoom level, and pixel dimensions.
 * It provides methods to convert between lat/lon ↔ screen coordinates using
 * a simple equirectangular (Mercator-like) projection suitable for radar range.
 */

use meso_data::geo::latlon::LatLon;

/// Radar display viewport — defines the visible region and projection.
#[derive(Debug, Clone)]
pub struct Viewport {
    /// Current center of the view (moves with pan).
    pub center: LatLon,
    /// Fixed radar site origin — set once at creation, never changes with pan/zoom.
    /// All radar-relative km offsets are projected from this point.
    pub site_origin: LatLon,
    /// Zoom factor (1.0 = default showing ~460 km radius for L2).
    pub zoom: f64,
    /// Pixel width of the render target.
    pub width: u32,
    /// Pixel height of the render target.
    pub height: u32,
    /// Radar range in km (used to scale the projection).
    pub range_km: f64,
}

impl Viewport {
    /// Create a default viewport centered on the given site.
    pub fn new(center: LatLon, width: u32, height: u32) -> Self {
        Viewport {
            site_origin: center,
            center,
            zoom: 1.0,
            width,
            height,
            range_km: 460.0, // L2 super-res range
        }
    }

    /// Scale factor: pixels per km at current zoom.
    pub fn pixels_per_km(&self) -> f64 {
        let min_dim = self.width.min(self.height) as f64;
        (min_dim / 2.0) / self.range_km * self.zoom
    }

    /// Convert a lat/lon to screen pixel coordinates (origin = top-left).
    pub fn latlon_to_screen(&self, loc: &LatLon) -> (f64, f64) {
        let ppkm = self.pixels_per_km();
        // Equirectangular projection relative to center
        let lat_rad = self.center.lat.to_radians();
        let dx_km = (loc.lon - self.center.lon).to_radians() * 6371.0 * lat_rad.cos();
        let dy_km = (loc.lat - self.center.lat).to_radians() * 6371.0;

        let px = self.width as f64 / 2.0 + dx_km * ppkm;
        let py = self.height as f64 / 2.0 - dy_km * ppkm; // y flipped (screen)
        (px, py)
    }

    /// Convert a radar-relative (x_km, y_km) offset from the site origin to screen coords.
    /// x_km is East+, y_km is North+.
    /// Uses the fixed site_origin so that radar data stays in sync with map layers
    /// regardless of where the user has panned.
    pub fn radar_km_to_screen(&self, x_km: f64, y_km: f64) -> (f64, f64) {
        let ppkm = self.pixels_per_km();
        // Project the site origin into screen space, then add the km offset.
        let (site_sx, site_sy) = self.latlon_to_screen(&self.site_origin);
        let px = site_sx + x_km * ppkm;
        let py = site_sy - y_km * ppkm; // y flipped (screen down = south)
        (px, py)
    }

    /// Convert screen pixel coords back to lat/lon.
    pub fn screen_to_latlon(&self, px: f64, py: f64) -> LatLon {
        let ppkm = self.pixels_per_km();
        let lat_rad = self.center.lat.to_radians();
        let dx_km = (px - self.width as f64 / 2.0) / ppkm;
        let dy_km = -(py - self.height as f64 / 2.0) / ppkm;
        let dlon = dx_km / (6371.0 * lat_rad.cos());
        let dlat = dy_km / 6371.0;
        LatLon {
            lat: self.center.lat + dlat.to_degrees(),
            lon: self.center.lon + dlon.to_degrees(),
        }
    }

    /// Zoom in/out by a factor (> 1 = zoom in).
    pub fn zoom_by(&mut self, factor: f64) {
        self.zoom = (self.zoom * factor).clamp(0.1, 20.0);
    }

    /// Zoom by `factor` keeping the point at screen coordinates (px, py) fixed.
    ///
    /// The anchor lat/lon is computed before zooming; after the zoom the center
    /// is adjusted so that anchor maps back to the same screen position.
    pub fn zoom_around_screen_point(&mut self, px: f64, py: f64, factor: f64) {
        let anchor = self.screen_to_latlon(px, py);
        self.zoom = (self.zoom * factor).clamp(0.1, 20.0);
        // Recompute where anchor would render now and shift center to compensate.
        let (new_ax, new_ay) = self.latlon_to_screen(&anchor);
        let dx_px = px - new_ax;
        let dy_px = py - new_ay;
        // pan_pixels(+d, 0) moves content right by d pixels; correct the drift directly.
        self.pan_pixels(dx_px, dy_px);
    }

    /// Pan by a pixel offset.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64) {
        let ppkm = self.pixels_per_km();
        let lat_rad = self.center.lat.to_radians();
        let dlon = -dx / ppkm / (6371.0 * lat_rad.cos());
        let dlat = dy / ppkm / 6371.0;
        self.center.lat = (self.center.lat + dlat.to_degrees()).clamp(-85.0, 85.0);
        self.center.lon += dlon.to_degrees();
    }
}
