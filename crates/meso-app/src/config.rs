/*
 * Application config: persisted to ~/.config/Meso/config.toml
 */

use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A user-defined named location (home, work, storm target, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedLocation {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadarTrackPoint {
    pub lat: f64,
    pub lon: f64,
    pub created_at: String,
    pub frame_index: usize,
    #[serde(default)]
    pub frame_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadarTrack {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub points: Vec<RadarTrackPoint>,
}

fn default_locations() -> Vec<NamedLocation> {
    vec![NamedLocation {
        name: "Home".to_string(),
        lat: 35.665,
        lon: -78.49,
    }]
}

fn default_active_location() -> String {
    "Home".to_string()
}
fn default_radar_active_track_id() -> String {
    "default".to_string()
}
fn default_radar_vector_lead_minutes() -> u16 {
    60
}
fn default_radar_vector_interval_minutes() -> u16 {
    15
}
fn default_alerts_pane_position() -> i32 {
    -1
}
fn default_neg_one() -> i32 {
    -1
}
fn default_sref() -> String {
    "sref".to_string()
}
fn default_conus() -> String {
    "CONUS".to_string()
}

fn default_false() -> bool {
    false
}
fn default_true() -> bool {
    true
}

fn default_cache_radar_hours() -> u32 {
    24
}
fn default_cache_sat_hours() -> u32 {
    24
}
fn default_cache_model_hours() -> u32 {
    24
}
fn default_cache_obs_minutes() -> u32 {
    60
}
fn default_cache_meso_minutes() -> u32 {
    30
}
fn default_updraft_interval_secs() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Last selected NEXRAD site ID (e.g. "KRAX")
    pub radar_site: String,
    /// Last selected radar product (e.g. "N0Q", "N0U")
    pub radar_product: String,
    /// Dual-pane radar mode toggle (false = single pane, true = two panes)
    #[serde(default = "default_false")]
    pub radar_dual_pane: bool,
    /// Left radar pane selected product
    #[serde(default)]
    pub radar_product_left: String,
    /// Right radar pane selected product
    #[serde(default)]
    pub radar_product_right: String,
    /// Number of radar panes (1, 2, or 4)
    pub radar_pane_count: u8,
    /// Last selected GOES sector
    pub goes_sector: String,
    /// Last selected GOES band
    pub goes_band: String,
    /// Last location for alert/forecast queries (lat, lon)
    pub location_lat: f64,
    pub location_lon: f64,
    /// Use GPU rendering (true = wgpu, false = Cairo CPU)
    pub use_gpu: bool,
    /// Window width in pixels
    pub window_width: i32,
    /// Window height in pixels
    pub window_height: i32,
    /// Number of radar animation frames to fetch (2–60)
    pub radar_anim_frames: u8,
    /// Number of satellite animation frames to fetch (2–60)
    pub sat_anim_frames: u8,
    /// Named color palette for reflectivity products
    pub radar_palette_ref: String,
    /// Named color palette for velocity products
    pub radar_palette_vel: String,
    /// Radar viewport zoom (1.0 = default ~460 km view)
    pub radar_zoom: f64,
    /// Radar viewport center latitude (0.0 = use site location)
    pub radar_center_lat: f64,
    /// Radar viewport center longitude (0.0 = use site location)
    pub radar_center_lon: f64,
    /// Satellite viewer zoom (1.0 = fit to widget)
    pub sat_zoom: f64,
    /// Satellite viewer pan offset X (image-space pixels)
    pub sat_pan_x: f64,
    /// Satellite viewer pan offset Y (image-space pixels)
    pub sat_pan_y: f64,
    /// Models pane divider position (-1 = auto-fit to 75%)
    pub models_pane_position: i32,
    /// SPC pane divider position (-1 = auto-fit to 75%)
    pub spc_pane_position: i32,
    /// Alerts pane divider position (-1 = default)
    #[serde(default = "default_alerts_pane_position")]
    pub alerts_pane_position: i32,
    /// Favorited SREF model product IDs
    #[serde(default)]
    pub model_favorites: Vec<String>,
    /// Favorited observation station IDs
    #[serde(default)]
    pub obs_favorites: Vec<String>,
    /// Favorited sounding station IDs
    #[serde(default)]
    pub sounding_favorites: Vec<String>,
    /// Favorited radar site IDs (shown at top of radar site list)
    #[serde(default)]
    pub radar_favorite_sites: Vec<String>,
    /// Last selected sounding station ID
    #[serde(default)]
    pub sounding_last_site: String,
    /// Soundings pane divider position (-1 = auto-fit to 75%)
    #[serde(default = "default_neg_one")]
    pub soundings_pane_position: i32,
    /// National products pane divider position (-1 = auto-fit to 75%)
    #[serde(default = "default_neg_one")]
    pub national_pane_position: i32,
    /// Last selected NCEP model type ("sref", "gfs", "nam", "rap", "hrrr")
    #[serde(default = "default_sref")]
    pub ncep_model_type: String,
    /// Last selected NCEP sector (e.g. "CONUS")
    #[serde(default = "default_conus")]
    pub ncep_sector: String,
    /// Named locations
    #[serde(default = "default_locations")]
    pub locations: Vec<NamedLocation>,
    /// Active location name (must match a `NamedLocation::name`)
    #[serde(default = "default_active_location")]
    pub active_location: String,
    // ── Cache retention ───────────────────────────────────────────────────────
    /// How long to keep radar data (hours)
    #[serde(default = "default_cache_radar_hours")]
    pub cache_radar_hours: u32,
    /// How long to keep satellite data (hours)
    #[serde(default = "default_cache_sat_hours")]
    pub cache_sat_hours: u32,
    /// How long to keep model data (hours)
    #[serde(default = "default_cache_model_hours")]
    pub cache_model_hours: u32,
    /// How long to keep observation data (minutes)
    #[serde(default = "default_cache_obs_minutes")]
    pub cache_obs_minutes: u32,
    /// How long to keep mesoanalysis data (minutes)
    #[serde(default = "default_cache_meso_minutes")]
    pub cache_meso_minutes: u32,
    // ── Radar overlay toggles ─────────────────────────────────────────────────
    /// Show watch/warning polygons on radar
    #[serde(default = "default_true")]
    pub radar_show_warnings: bool,
    /// Show range rings on radar
    #[serde(default = "default_true")]
    pub radar_show_rings: bool,
    /// Show NEXRAD storm track vectors on radar
    #[serde(default = "default_true")]
    pub radar_show_storm_tracks: bool,
    /// Show major roads (interstates/highways) on radar
    #[serde(default = "default_false")]
    pub radar_show_major_roads: bool,
    /// Hide no-data radar bins (value 0)
    #[serde(default = "default_true")]
    pub radar_qc_hide_no_data: bool,
    /// Mask weak reflectivity echoes (approx low-SNR suppression)
    #[serde(default = "default_false")]
    pub radar_qc_mask_weak_echoes: bool,
    /// Custom user-defined radar tracking tracks
    #[serde(default)]
    pub radar_tracks: Vec<RadarTrack>,
    /// Active radar track ID for marker operations
    #[serde(default = "default_radar_active_track_id")]
    pub radar_active_track_id: String,
    /// Show custom radar track points
    #[serde(default = "default_true")]
    pub radar_show_track_points: bool,
    /// Show custom radar track connecting lines
    #[serde(default = "default_true")]
    pub radar_show_track_lines: bool,
    /// Show projected vector based on custom radar tracks
    #[serde(default = "default_true")]
    pub radar_show_track_vector: bool,
    /// Blend acceleration into projected vector estimate
    #[serde(default = "default_true")]
    pub radar_vector_accel_bias: bool,
    /// How many minutes to project track vectors forward
    #[serde(default = "default_radar_vector_lead_minutes")]
    pub radar_vector_lead_minutes: u16,
    /// Interval (minutes) between projected vector dots/segments
    #[serde(default = "default_radar_vector_interval_minutes")]
    pub radar_vector_interval_minutes: u16,
    // ── Updraft daemon ────────────────────────────────────────────────────────
    /// Enable the meso-updraft background caching daemon
    #[serde(default = "default_false")]
    pub updraft_enabled: bool,
    /// Wake interval for the daemon in seconds (default 300 = 5 min)
    #[serde(default = "default_updraft_interval_secs")]
    pub updraft_interval_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            radar_site: "KRAX".to_string(),
            radar_product: "N0Q".to_string(),
            radar_dual_pane: false,
            radar_product_left: "N0Q".to_string(),
            radar_product_right: "N0U".to_string(),
            radar_pane_count: 1,
            goes_sector: "CONUS".to_string(),
            goes_band: "GEOCOLOR".to_string(),
            location_lat: 35.665,
            location_lon: -78.49, // Raleigh/Durham (KRAX) default
            use_gpu: true,
            window_width: 1200,
            window_height: 800,
            radar_anim_frames: 20,
            sat_anim_frames: 10,
            radar_palette_ref: "CODENH".to_string(),
            radar_palette_vel: "CODENH".to_string(),
            radar_zoom: 1.0,
            radar_center_lat: 0.0,
            radar_center_lon: 0.0,
            sat_zoom: 1.0,
            sat_pan_x: 0.0,
            sat_pan_y: 0.0,
            models_pane_position: -1,
            spc_pane_position: -1,
            alerts_pane_position: default_alerts_pane_position(),
            model_favorites: Vec::new(),
            obs_favorites: Vec::new(),
            sounding_favorites: Vec::new(),
            radar_favorite_sites: Vec::new(),
            sounding_last_site: String::new(),
            soundings_pane_position: -1,
            national_pane_position: -1,
            ncep_model_type: "sref".to_string(),
            ncep_sector: "CONUS".to_string(),
            locations: default_locations(),
            active_location: default_active_location(),
            cache_radar_hours: default_cache_radar_hours(),
            cache_sat_hours: default_cache_sat_hours(),
            cache_model_hours: default_cache_model_hours(),
            cache_obs_minutes: default_cache_obs_minutes(),
            cache_meso_minutes: default_cache_meso_minutes(),
            radar_show_warnings: true,
            radar_show_rings: true,
            radar_show_storm_tracks: true,
            radar_show_major_roads: false,
            radar_qc_hide_no_data: true,
            radar_qc_mask_weak_echoes: false,
            radar_tracks: Vec::new(),
            radar_active_track_id: default_radar_active_track_id(),
            radar_show_track_points: true,
            radar_show_track_lines: true,
            radar_show_track_vector: true,
            radar_vector_accel_bias: true,
            radar_vector_lead_minutes: default_radar_vector_lead_minutes(),
            radar_vector_interval_minutes: default_radar_vector_interval_minutes(),
            updraft_enabled: false,
            updraft_interval_secs: default_updraft_interval_secs(),
        }
    }
}

impl Config {
    pub fn config_path() -> Option<PathBuf> {
        ProjectDirs::from("", "", "Meso").map(|p| p.config_dir().join("config.toml"))
    }

    /// Return the currently active named location, if any.
    #[allow(dead_code)]
    pub fn active_loc(&self) -> Option<&NamedLocation> {
        if self.active_location.is_empty() {
            return None;
        }
        self.locations
            .iter()
            .find(|l| l.name == self.active_location)
    }

    pub fn load() -> Self {
        let path = match Self::config_path() {
            Some(p) => p,
            None => return Self::default(),
        };
        if !path.exists() {
            return Self::default();
        }
        let mut cfg = match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        };

        // Backward compatibility: older configs only had `radar_product`.
        if cfg.radar_product_left.is_empty() {
            cfg.radar_product_left = cfg.radar_product.clone();
        }
        if cfg.radar_product_right.is_empty() {
            cfg.radar_product_right = cfg.radar_product.clone();
        }
        if cfg.radar_product.is_empty() {
            cfg.radar_product = cfg.radar_product_left.clone();
        }
        if cfg.radar_active_track_id.is_empty() {
            cfg.radar_active_track_id = default_radar_active_track_id();
        }

        cfg
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path().ok_or_else(|| anyhow::anyhow!("No config dir"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let s = toml::to_string_pretty(self)?;
        std::fs::write(&path, s)?;
        Ok(())
    }
}
