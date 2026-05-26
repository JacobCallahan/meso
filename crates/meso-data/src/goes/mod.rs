/*
 * GOES satellite imagery fetching.
 *
 * Builds CDN URLs for current images and scrapes the NESDIS animation endpoint
 * for multi-frame playback.  Ported from wX's UtilityGoes.kt.
 */

use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;

use crate::geo::latlon::LatLon;

// ── Sector metadata ───────────────────────────────────────────────────────────

/// A GOES sector definition.
#[derive(Debug, Clone)]
pub struct Sector {
    pub code: &'static str,
    pub name: &'static str,
    /// Representative center (used for "nearest sector" lookup).
    pub center: Option<LatLon>,
    /// Which GOES satellite serves this sector.
    pub satellite: GoesSatellite,
    /// Image pixel size string (e.g. "1250x750")
    pub image_size: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoesSatellite {
    Goes19, // GOES-East
    Goes17, // GOES-West
}

impl GoesSatellite {
    pub fn sat_code(self) -> &'static str {
        match self {
            GoesSatellite::Goes19 => "G19",
            GoesSatellite::Goes17 => "G17",
        }
    }
    pub fn cdn_name(self) -> &'static str {
        match self {
            GoesSatellite::Goes19 => "GOES19",
            GoesSatellite::Goes17 => "GOES17",
        }
    }
}

pub static SECTORS: &[Sector] = &[
    Sector {
        code: "FD",
        name: "Full Disk: GOES-East",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1808x1808",
    },
    Sector {
        code: "FD-G17",
        name: "Full Disk: GOES-West",
        center: None,
        satellite: GoesSatellite::Goes17,
        image_size: "1808x1808",
    },
    Sector {
        code: "CONUS",
        name: "CONUS: GOES-East",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1250x750",
    },
    Sector {
        code: "CONUS-G17",
        name: "PACUS: GOES-West",
        center: None,
        satellite: GoesSatellite::Goes17,
        image_size: "1250x750",
    },
    Sector {
        code: "pnw",
        name: "Pacific Northwest",
        center: Some(LatLon {
            lat: 41.59,
            lon: -119.86,
        }),
        satellite: GoesSatellite::Goes17,
        image_size: "latest",
    },
    Sector {
        code: "nr",
        name: "Northern Rockies",
        center: Some(LatLon {
            lat: 41.14,
            lon: -104.82,
        }),
        satellite: GoesSatellite::Goes19,
        image_size: "latest",
    },
    Sector {
        code: "umv",
        name: "Upper Mississippi Valley",
        center: Some(LatLon {
            lat: 40.62,
            lon: -93.93,
        }),
        satellite: GoesSatellite::Goes19,
        image_size: "latest",
    },
    Sector {
        code: "cgl",
        name: "Central Great Lakes",
        center: Some(LatLon {
            lat: 39.12,
            lon: -82.53,
        }),
        satellite: GoesSatellite::Goes19,
        image_size: "latest",
    },
    Sector {
        code: "ne",
        name: "Northeast",
        center: Some(LatLon {
            lat: 39.36,
            lon: -74.43,
        }),
        satellite: GoesSatellite::Goes19,
        image_size: "latest",
    },
    Sector {
        code: "psw",
        name: "Pacific Southwest",
        center: Some(LatLon {
            lat: 38.52,
            lon: -118.62,
        }),
        satellite: GoesSatellite::Goes17,
        image_size: "latest",
    },
    Sector {
        code: "sr",
        name: "Southern Rockies",
        center: Some(LatLon {
            lat: 34.65,
            lon: -108.68,
        }),
        satellite: GoesSatellite::Goes19,
        image_size: "latest",
    },
    Sector {
        code: "sp",
        name: "Southern Plains",
        center: Some(LatLon {
            lat: 31.46,
            lon: -96.06,
        }),
        satellite: GoesSatellite::Goes19,
        image_size: "latest",
    },
    Sector {
        code: "smv",
        name: "Southern Mississippi Valley",
        center: Some(LatLon {
            lat: 31.33,
            lon: -89.29,
        }),
        satellite: GoesSatellite::Goes19,
        image_size: "latest",
    },
    Sector {
        code: "se",
        name: "Southeast",
        center: Some(LatLon {
            lat: 30.33,
            lon: -81.66,
        }),
        satellite: GoesSatellite::Goes19,
        image_size: "latest",
    },
    Sector {
        code: "gm",
        name: "Gulf of Mexico",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1000x1000",
    },
    Sector {
        code: "car",
        name: "Caribbean",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1000x1000",
    },
    Sector {
        code: "eus",
        name: "U.S. Atlantic Coast",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1000x1000",
    },
    Sector {
        code: "pr",
        name: "Puerto Rico",
        center: Some(LatLon {
            lat: 18.23,
            lon: -66.03,
        }),
        satellite: GoesSatellite::Goes19,
        image_size: "latest",
    },
    Sector {
        code: "ak",
        name: "Alaska",
        center: None,
        satellite: GoesSatellite::Goes17,
        image_size: "1000x1000",
    },
    Sector {
        code: "cak",
        name: "Central Alaska",
        center: None,
        satellite: GoesSatellite::Goes17,
        image_size: "1200x1200",
    },
    Sector {
        code: "sea",
        name: "Southeastern Alaska",
        center: None,
        satellite: GoesSatellite::Goes17,
        image_size: "1200x1200",
    },
    Sector {
        code: "hi",
        name: "Hawaii",
        center: Some(LatLon {
            lat: 20.76,
            lon: -155.33,
        }),
        satellite: GoesSatellite::Goes17,
        image_size: "1200x1200",
    },
    Sector {
        code: "wus",
        name: "US Pacific Coast",
        center: None,
        satellite: GoesSatellite::Goes17,
        image_size: "1000x1000",
    },
    Sector {
        code: "tpw",
        name: "Tropical Pacific",
        center: None,
        satellite: GoesSatellite::Goes17,
        image_size: "1800x1080",
    },
    Sector {
        code: "tsp",
        name: "South Pacific",
        center: None,
        satellite: GoesSatellite::Goes17,
        image_size: "1800x1080",
    },
    Sector {
        code: "eep",
        name: "Eastern Pacific",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1800x1080",
    },
    Sector {
        code: "np",
        name: "Northern Pacific",
        center: None,
        satellite: GoesSatellite::Goes17,
        image_size: "1800x1080",
    },
    Sector {
        code: "na",
        name: "Northern Atlantic",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1800x1080",
    },
    Sector {
        code: "taw",
        name: "Tropical Atlantic",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1800x1080",
    },
    Sector {
        code: "can",
        name: "Canada",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1125x560",
    },
    Sector {
        code: "mex",
        name: "Mexico",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1000x1000",
    },
    Sector {
        code: "cam",
        name: "Central America",
        center: None,
        satellite: GoesSatellite::Goes19,
        image_size: "1000x1000",
    },
];

// ── Band (product) codes ──────────────────────────────────────────────────────

pub static BAND_CODES: &[&str] = &[
    "GEOCOLOR",
    "01",
    "02",
    "03",
    "04",
    "05",
    "06",
    "07",
    "08",
    "09",
    "10",
    "11",
    "12",
    "13",
    "14",
    "15",
    "16",
    "AirMass",
    "Sandwich",
    "DayCloudPhase",
    "NightMicrophysics",
    "FireTemperature",
    "Dust",
    "GLM",
    "DMW",
];

pub static BAND_LABELS: &[&str] = &[
    "True color daytime / multispectral IR at night",
    "0.47 µm (Band 1) Blue - Visible",
    "0.64 µm (Band 2) Red - Visible",
    "0.86 µm (Band 3) Veggie - Near IR",
    "1.37 µm (Band 4) Cirrus - Near IR",
    "1.6 µm (Band 5) Snow/Ice - Near IR",
    "2.2 µm (Band 6) Cloud Particle - Near IR",
    "3.9 µm (Band 7) Shortwave Window - IR",
    "6.2 µm (Band 8) Upper-Level Water Vapor - IR",
    "6.9 µm (Band 9) Mid-Level Water Vapor - IR",
    "7.3 µm (Band 10) Lower-level Water Vapor - IR",
    "8.4 µm (Band 11) Cloud Top - IR",
    "9.6 µm (Band 12) Ozone - IR",
    "10.3 µm (Band 13) Clean Longwave Window - IR",
    "11.2 µm (Band 14) Longwave Window - IR",
    "12.3 µm (Band 15) Dirty Longwave Window - IR",
    "13.3 µm (Band 16) CO2 Longwave - IR",
    "AirMass RGB composite",
    "Sandwich RGB (Band 3 + 13)",
    "Day Cloud Phase",
    "Night Microphysics",
    "Fire Temperature",
    "Dust RGB",
    "GLM FED + GeoColor",
    "DMW",
];

// ── URL builders ──────────────────────────────────────────────────────────────

const CDN_BASE: &str = "https://cdn.star.nesdis.noaa.gov";
const ANIM_BASE: &str = "https://www.star.nesdis.noaa.gov";

/// Return the lookup `Sector` for the given code, if any.
pub fn find_sector(code: &str) -> Option<&'static Sector> {
    SECTORS.iter().find(|s| s.code == code)
}

/// Find the nearest sector to a lat/lon coordinate.
/// Falls back to CONUS if no sector has a center defined nearby.
pub fn nearest_sector(loc: &LatLon) -> &'static str {
    let mut best = "CONUS";
    let mut best_dist = f64::MAX;
    for sec in SECTORS {
        if let Some(center) = &sec.center {
            let d = loc.distance_km(center);
            if d < best_dist {
                best_dist = d;
                best = sec.code;
            }
        }
    }
    best
}

/// Build the CDN URL for the most-recent image for a sector/band combination.
pub fn image_url(sector_code: &str, band: &str) -> String {
    let sec = find_sector(sector_code);
    let satellite = sec.map(|s| s.satellite).unwrap_or(GoesSatellite::Goes19);
    let size = sec.map(|s| s.image_size).unwrap_or("latest");

    let sector_path = match sector_code {
        "FD" | "FD-G17" => "FD".to_string(),
        "CONUS" | "CONUS-G17" => "CONUS".to_string(),
        other => format!("SECTOR/{other}"),
    };

    // GLM maps to EXTENT3 in wX
    let band_local = if band == "GLM" { "EXTENT3" } else { band };

    let filename = if size == "latest" {
        "latest.jpg".to_string()
    } else {
        format!("{size}.jpg")
    };

    format!(
        "{CDN_BASE}/{}/ABI/{sector_path}/{band_local}/{filename}",
        satellite.cdn_name()
    )
}

/// Fetch the list of animation frame URLs for a sector/band from the NESDIS HTML page.
pub async fn animation_urls(
    client: &Client,
    sector_code: &str,
    band: &str,
    frame_count: usize,
) -> Result<Vec<String>> {
    let sec = find_sector(sector_code);
    let satellite = sec.map(|s| s.satellite).unwrap_or(GoesSatellite::Goes19);
    let sat_str = satellite.sat_code();

    // GLM band maps to EXTENT3 in animation URL (matches wX behavior)
    let band_param = if band == "GLM" { "EXTENT3" } else { band };

    let base_url = match sector_code {
        "FD" | "FD-G17" => format!(
            "{ANIM_BASE}/GOES/fulldisk_band.php?sat={sat_str}&band={band_param}&length={frame_count}"
        ),
        "CONUS" | "CONUS-G17" => format!(
            "{ANIM_BASE}/GOES/conus_band.php?sat={sat_str}&band={band_param}&length={frame_count}"
        ),
        sector => format!(
            "{ANIM_BASE}/GOES/sector_band.php?sat={sat_str}&sector={sector}&band={band_param}&length={frame_count}"
        ),
    };

    let html = client
        .get(&base_url)
        .send()
        .await
        .context("GOES animation request failed")?
        .text()
        .await
        .context("GOES animation response read failed")?;

    parse_animation_urls(&html, frame_count)
}

fn parse_animation_urls(html: &str, max_frames: usize) -> Result<Vec<String>> {
    // The NESDIS page embeds a JS array: animationImages = ['https://cdn.star...jpg', ...];
    // Extract the array contents first, then pull each quoted URL.
    let compressed = html.replace("\r\n", " ").replace('\n', " ");

    let array_re = Regex::new(r"animationImages\s*=\s*\[(.*?)\]").unwrap();
    let url_re = Regex::new(r"'(https[^']+\.jpg)'").unwrap();

    let urls: Vec<String> = if let Some(cap) = array_re.captures(&compressed) {
        let array_str = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        url_re
            .captures_iter(array_str)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect()
    } else {
        // Fallback: match any CDN .jpg in anchor hrefs (older page format)
        let anchor_re =
            Regex::new(r#"<a href="(https://cdn\.star\.nesdis\.noaa\.gov[^"]+\.jpg)">"#).unwrap();
        anchor_re
            .captures_iter(&compressed)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect()
    };

    if urls.is_empty() {
        anyhow::bail!(
            "No animation frames found in NESDIS page (got {} bytes of HTML)",
            html.len()
        );
    }

    let start = urls.len().saturating_sub(max_frames);
    Ok(urls[start..].to_vec())
}

/// Fetch the raw bytes of a GOES image URL.
/// Results are cached: "latest" single-image URLs for 5 min, animation frame
/// URLs for 30 min (they're archived and never change).
pub async fn fetch_image(client: &Client, url: &str) -> Result<Vec<u8>> {
    use crate::cache::Cache;
    use std::time::Duration;

    let is_anim_frame = !url.contains("latest");
    let ttl = if is_anim_frame {
        Duration::from_secs(30 * 60)
    } else {
        Duration::from_secs(5 * 60)
    };

    let cache = Cache::new("goes");
    if let Some(bytes) = cache.get(url) {
        return Ok(bytes);
    }

    let bytes = client
        .get(url)
        .send()
        .await
        .context("GOES image fetch failed")?
        .bytes()
        .await
        .context("GOES image body read failed")?;
    cache.put(url, &bytes, ttl);
    Ok(bytes.to_vec())
}
