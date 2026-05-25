/*
 * Updraft subscription types and helpers.
 *
 * Subscriptions are stored in a separate file from the main config so the
 * daemon can read/write them without touching the full config.
 *
 * File location: same directory as config.toml → `subscriptions.toml`
 * Format:
 *   [[radar]]
 *   station = "KRAX"
 *   product = "N0Q"
 *
 *   [[satellite]]
 *   sector = "CONUS"
 *   band = "GEOCOLOR"
 */

use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RadarSubscription {
    pub station: String,
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SatSubscription {
    pub sector: String,
    pub band: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Subscriptions {
    #[serde(default)]
    pub radar: Vec<RadarSubscription>,
    #[serde(default)]
    pub satellite: Vec<SatSubscription>,
}

impl Subscriptions {
    pub fn is_radar_subscribed(&self, station: &str, product: &str) -> bool {
        self.radar
            .iter()
            .any(|s| s.station == station && s.product == product)
    }

    pub fn is_sat_subscribed(&self, sector: &str, band: &str) -> bool {
        self.satellite
            .iter()
            .any(|s| s.sector == sector && s.band == band)
    }

    /// Toggle a radar subscription: add if absent, remove if present.
    /// Returns `true` if the subscription is now active.
    pub fn toggle_radar(&mut self, station: &str, product: &str) -> bool {
        if let Some(pos) = self
            .radar
            .iter()
            .position(|s| s.station == station && s.product == product)
        {
            self.radar.remove(pos);
            false
        } else {
            self.radar.push(RadarSubscription {
                station: station.to_string(),
                product: product.to_string(),
            });
            true
        }
    }

    /// Toggle a satellite subscription. Returns `true` if now active.
    pub fn toggle_sat(&mut self, sector: &str, band: &str) -> bool {
        if let Some(pos) = self
            .satellite
            .iter()
            .position(|s| s.sector == sector && s.band == band)
        {
            self.satellite.remove(pos);
            false
        } else {
            self.satellite.push(SatSubscription {
                sector: sector.to_string(),
                band: band.to_string(),
            });
            true
        }
    }
}

pub fn subscriptions_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "Meso").map(|p| p.config_dir().join("subscriptions.toml"))
}

pub fn load_subscriptions() -> Subscriptions {
    let path = match subscriptions_path() {
        Some(p) => p,
        None => return Subscriptions::default(),
    };
    if !path.exists() {
        return Subscriptions::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_default(),
        Err(_) => Subscriptions::default(),
    }
}

pub fn save_subscriptions(subs: &Subscriptions) -> Result<()> {
    let path = subscriptions_path().ok_or_else(|| anyhow::anyhow!("No config dir"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let s = toml::to_string_pretty(subs)?;
    std::fs::write(&path, s)?;
    Ok(())
}
