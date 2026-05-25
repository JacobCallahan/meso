/*
 * SPC Mesoanalysis image fetching.
 *
 * The SPC mesoanalysis page provides near-real-time analysis images at:
 *   https://www.spc.noaa.gov/exper/mesoanalysis/s{sector}/{param}/{param}.gif
 *
 * Sector 19 = CONUS (full US).  All listed products have been verified to
 * return HTTP 200 from the SPC server (403s appear for params not yet generated).
 *
 * Images are cached for 10 minutes (typical SPC update frequency).
 */

use anyhow::{Context, Result};
use reqwest::Client;

use crate::cache::Cache;

// ── Product catalog ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct MesoProduct {
    /// URL parameter name (used in the image URL).
    pub id: &'static str,
    /// Human-readable label.
    pub label: &'static str,
    /// Grouping category.
    pub category: &'static str,
}

pub static MESO_PRODUCTS: &[MesoProduct] = &[
    // Surface
    MesoProduct {
        id: "bigsfc",
        label: "Surface Analysis",
        category: "Surface",
    },
    MesoProduct {
        id: "ttd",
        label: "Surface Dewpoint (°F)",
        category: "Surface",
    },
    MesoProduct {
        id: "thea",
        label: "850mb Theta-E",
        category: "Surface",
    },
    // Moisture / Instability
    MesoProduct {
        id: "mixr",
        label: "Mixing Ratio",
        category: "Moisture/Instability",
    },
    MesoProduct {
        id: "lllr",
        label: "LCL Height",
        category: "Moisture/Instability",
    },
    MesoProduct {
        id: "laps",
        label: "LAPS Analysis",
        category: "Moisture/Instability",
    },
    // Wind / Shear
    MesoProduct {
        id: "ageo",
        label: "700mb RH/Wind (Ageo)",
        category: "Wind/Shear",
    },
    MesoProduct {
        id: "shr6",
        label: "0-6km Bulk Shear",
        category: "Wind/Shear",
    },
    MesoProduct {
        id: "srh1",
        label: "0-1km Storm-Relative Helicity",
        category: "Wind/Shear",
    },
    MesoProduct {
        id: "srh3",
        label: "0-3km Storm-Relative Helicity",
        category: "Wind/Shear",
    },
    // Severe Parameters
    MesoProduct {
        id: "stpc",
        label: "Significant Tornado Param (Composite)",
        category: "Severe",
    },
    MesoProduct {
        id: "scp",
        label: "Supercell Composite Param",
        category: "Severe",
    },
];

/// Return the list of all available mesoanalysis products.
pub fn meso_products() -> &'static [MesoProduct] {
    MESO_PRODUCTS
}

/// Default sector for CONUS coverage.
pub const DEFAULT_SECTOR: u8 = 19;

/// Build the image URL for a given parameter and sector.
pub fn meso_image_url(param: &str, sector: u8) -> String {
    format!("https://www.spc.noaa.gov/exper/mesoanalysis/s{sector}/{param}/{param}.gif")
}

// ── Fetching ──────────────────────────────────────────────────────────────────

/// Fetch a mesoanalysis image for the given parameter, with a 10-minute disk cache.
pub async fn fetch_meso_image(client: &Client, param: &str) -> Result<Vec<u8>> {
    fetch_meso_image_sector(client, param, DEFAULT_SECTOR).await
}

/// Fetch a mesoanalysis image for the given parameter and sector.
pub async fn fetch_meso_image_sector(client: &Client, param: &str, sector: u8) -> Result<Vec<u8>> {
    let url = meso_image_url(param, sector);
    let cache = Cache::new("mesoanalysis");
    if let Some(bytes) = cache.get(&url) {
        return Ok(bytes);
    }
    let bytes = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Mesoanalysis fetch failed: {url}"))?
        .error_for_status()
        .with_context(|| format!("Mesoanalysis HTTP error: {url}"))?
        .bytes()
        .await
        .with_context(|| format!("Mesoanalysis read failed: {url}"))?
        .to_vec();
    cache.put(&url, &bytes, std::time::Duration::from_secs(600));
    Ok(bytes)
}
