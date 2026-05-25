/*
 * Overlay layers: warnings, county outlines, range rings, city labels, etc.
 *
 * Each overlay is a collection of vector primitives (polylines, polygons,
 * circles, text labels) that are composited on top of the radar mosaic.
 */

use meso_data::alerts::Warning;
use meso_data::storm_tracks::StormCell;

// ── Warning color mapping ─────────────────────────────────────────────────────

/// RGB color for a given NWS warning event type.
pub fn warning_color(event: &str) -> (u8, u8, u8) {
    let event_lc = event.to_lowercase();
    if event_lc.contains("tornado warning") {
        (255, 0, 255)
    } else if event_lc.contains("tornado watch") {
        (255, 255, 0)
    } else if event_lc.contains("severe thunderstorm warning") {
        (255, 165, 0)
    } else if event_lc.contains("severe thunderstorm watch") {
        (219, 112, 147)
    } else if event_lc.contains("flash flood warning") {
        (0, 255, 0)
    } else if event_lc.contains("flash flood watch") {
        (46, 139, 87)
    } else if event_lc.contains("special marine warning") {
        (255, 165, 0)
    } else if event_lc.contains("blizzard warning") {
        (255, 255, 255)
    } else if event_lc.contains("winter storm warning") {
        (255, 105, 180)
    } else if event_lc.contains("winter storm watch") {
        (70, 130, 180)
    } else if event_lc.contains("ice storm") {
        (139, 0, 139)
    } else if event_lc.contains("wind advisory") {
        (210, 180, 140)
    } else if event_lc.contains("high wind warning") {
        (218, 165, 32)
    } else if event_lc.contains("dense fog") {
        (112, 128, 144)
    } else if event_lc.contains("heat advisory") {
        (255, 127, 0)
    } else if event_lc.contains("excessive heat") {
        (200, 0, 0)
    } else if event_lc.contains("freeze warning") || event_lc.contains("frost") {
        (100, 149, 237)
    } else if event_lc.contains("special weather statement") {
        (255, 228, 181)
    } else {
        (128, 128, 128)
    }
}

// ── Overlay primitives ────────────────────────────────────────────────────────

/// A screen-space polyline (closed or open).
#[derive(Debug, Clone)]
pub struct Polyline {
    /// Screen pixel coordinates: [(x0,y0), (x1,y1), ...]
    pub points: Vec<(f32, f32)>,
    pub color: (u8, u8, u8),
    pub line_width: f32,
    pub closed: bool,
}

/// A text label at a screen position.
#[derive(Debug, Clone)]
pub struct Label {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub color: (u8, u8, u8),
    pub font_size: f32,
}

/// A range ring (circle) centered on the radar site.
#[derive(Debug, Clone)]
pub struct RangeRing {
    /// Ring radius in km.
    pub radius_km: f64,
    pub color: (u8, u8, u8),
    pub line_width: f32,
}

/// A storm cell position + motion vector for display on radar.
#[derive(Debug, Clone)]
pub struct StormCellMarker {
    pub id: String,
    pub lat: f32,
    pub lon: f32,
    pub bearing_deg: f32,
    pub speed_kt: f32,
}

// ── OverlayLayer type ─────────────────────────────────────────────────────────

/// A single named overlay layer containing vector primitives.
#[derive(Debug, Clone)]
pub struct OverlayLayer {
    pub name: String,
    pub visible: bool,
    pub polylines: Vec<Polyline>,
    pub labels: Vec<Label>,
    pub storm_cells: Vec<StormCellMarker>,
}

impl OverlayLayer {
    pub fn new(name: impl Into<String>) -> Self {
        OverlayLayer {
            name: name.into(),
            visible: true,
            polylines: Vec::new(),
            labels: Vec::new(),
            storm_cells: Vec::new(),
        }
    }
}

/// Collection of all active overlay layers for a radar pane.
#[derive(Debug, Clone)]
pub struct OverlaySet {
    pub layers: Vec<OverlayLayer>,
    pub range_rings: Vec<RangeRing>,
    pub rings_visible: bool,
}

impl Default for OverlaySet {
    fn default() -> Self {
        Self::new()
    }
}

impl OverlaySet {
    pub fn new() -> Self {
        OverlaySet {
            layers: Vec::new(),
            range_rings: default_range_rings(),
            rings_visible: true,
        }
    }

    pub fn add_layer(&mut self, layer: OverlayLayer) {
        self.layers.push(layer);
    }

    pub fn get_layer_mut(&mut self, name: &str) -> Option<&mut OverlayLayer> {
        self.layers.iter_mut().find(|l| l.name == name)
    }

    pub fn set_visible(&mut self, name: &str, visible: bool) {
        if let Some(l) = self.get_layer_mut(name) {
            l.visible = visible;
        }
    }
}

/// Default range rings: 25, 50, 100, 150, 250 km.
fn default_range_rings() -> Vec<RangeRing> {
    [25.0, 50.0, 100.0, 150.0, 250.0]
        .iter()
        .map(|&r| RangeRing {
            radius_km: r,
            color: (80, 80, 80),
            line_width: if r == 50.0 || r == 150.0 { 1.5 } else { 0.8 },
        })
        .collect()
}

// ── Warning layer builder ─────────────────────────────────────────────────────

/// Build (or update) the warnings overlay layer from a list of active warnings.
pub fn build_warnings_layer(warnings: &[Warning]) -> OverlayLayer {
    let mut layer = OverlayLayer::new("warnings");
    for w in warnings {
        if !w.is_current || w.polygon.is_empty() {
            continue;
        }
        let color = warning_color(&w.event);
        let pts: Vec<(f32, f32)> = w
            .polygon
            .iter()
            .map(|p| (p.lat as f32, p.lon as f32))
            .collect();
        layer.polylines.push(Polyline {
            points: pts,
            color,
            line_width: 2.0,
            closed: true,
        });
    }
    layer
}

pub fn build_storm_tracks_layer(cells: &[StormCell]) -> OverlayLayer {
    let mut layer = OverlayLayer::new("storm_tracks");
    for cell in cells {
        layer.storm_cells.push(StormCellMarker {
            id: cell.id.clone(),
            lat: cell.lat as f32,
            lon: cell.lon as f32,
            bearing_deg: cell.bearing_deg as f32,
            speed_kt: cell.speed_kt as f32,
        });
    }
    layer
}
