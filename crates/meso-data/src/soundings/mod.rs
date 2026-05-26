/*
 * Upper-air sounding data module.
 *
 * Data source: SPC experimental sounding page
 *   Image: https://www.spc.noaa.gov/exper/soundings/LATEST/{SITE}.gif
 *   Text:  https://www.spc.noaa.gov/exper/soundings/LATEST/{SITE}.txt
 *
 * The image is a pre-rendered Skew-T log-P diagram with hodograph inset.
 * The text file contains derived indices (CAPE, CIN, LI, etc.) at the top.
 */

use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;

use crate::cache::Cache;

// ── Site metadata ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SoundingSite {
    pub id: &'static str,
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
}

pub static SITES: &[SoundingSite] = &[
    SoundingSite {
        id: "1Y7",
        name: "AZ, Yuma",
        lat: 32.86,
        lon: -114.40,
    },
    SoundingSite {
        id: "76225",
        name: "MX, Chihuahua",
        lat: 28.70,
        lon: -106.07,
    },
    SoundingSite {
        id: "76405",
        name: "MX, La Paz",
        lat: 24.07,
        lon: -110.33,
    },
    SoundingSite {
        id: "76458",
        name: "MX, Mazatlan",
        lat: 23.18,
        lon: -106.42,
    },
    SoundingSite {
        id: "ABQ",
        name: "NM, Albuquerque",
        lat: 35.03,
        lon: -106.62,
    },
    SoundingSite {
        id: "ABR",
        name: "SD, Aberdeen",
        lat: 45.45,
        lon: -98.41,
    },
    SoundingSite {
        id: "ALB",
        name: "NY, Albany",
        lat: 42.75,
        lon: -73.80,
    },
    SoundingSite {
        id: "AMA",
        name: "TX, Amarillo",
        lat: 35.23,
        lon: -101.70,
    },
    SoundingSite {
        id: "APX",
        name: "MI, Gaylord",
        lat: 44.90,
        lon: -84.72,
    },
    SoundingSite {
        id: "BIS",
        name: "ND, Bismarck",
        lat: 46.77,
        lon: -100.75,
    },
    SoundingSite {
        id: "BMX",
        name: "AL, Birmingham",
        lat: 33.17,
        lon: -86.77,
    },
    SoundingSite {
        id: "BNA",
        name: "TN, Nashville",
        lat: 36.12,
        lon: -86.68,
    },
    SoundingSite {
        id: "BOI",
        name: "ID, Boise",
        lat: 43.57,
        lon: -116.22,
    },
    SoundingSite {
        id: "BRO",
        name: "TX, Brownsville",
        lat: 25.90,
        lon: -97.43,
    },
    SoundingSite {
        id: "BUF",
        name: "NY, Buffalo",
        lat: 42.93,
        lon: -78.73,
    },
    SoundingSite {
        id: "CAR",
        name: "ME, Caribou",
        lat: 46.87,
        lon: -68.02,
    },
    SoundingSite {
        id: "CHS",
        name: "SC, Charleston",
        lat: 32.90,
        lon: -80.03,
    },
    SoundingSite {
        id: "CRP",
        name: "TX, Corpus Christi",
        lat: 27.77,
        lon: -97.50,
    },
    SoundingSite {
        id: "DDC",
        name: "KS, Dodge City",
        lat: 37.77,
        lon: -99.97,
    },
    SoundingSite {
        id: "DRT",
        name: "TX, Del Rio",
        lat: 29.37,
        lon: -100.92,
    },
    SoundingSite {
        id: "DTX",
        name: "MI, Detroit",
        lat: 42.68,
        lon: -83.47,
    },
    SoundingSite {
        id: "DVN",
        name: "IA, Davenport",
        lat: 41.60,
        lon: -90.60,
    },
    SoundingSite {
        id: "EPZ",
        name: "TX, El Paso",
        lat: 31.90,
        lon: -106.70,
    },
    SoundingSite {
        id: "FFC",
        name: "GA, Atlanta",
        lat: 33.36,
        lon: -84.56,
    },
    SoundingSite {
        id: "FWD",
        name: "TX, Dallas",
        lat: 32.80,
        lon: -97.30,
    },
    SoundingSite {
        id: "GGW",
        name: "MT, Glasgow",
        lat: 48.21,
        lon: -106.63,
    },
    SoundingSite {
        id: "GJT",
        name: "CO, Grand Junction",
        lat: 39.12,
        lon: -108.53,
    },
    SoundingSite {
        id: "GRB",
        name: "WI, Green Bay",
        lat: 44.48,
        lon: -88.13,
    },
    SoundingSite {
        id: "GSO",
        name: "NC, Greensboro",
        lat: 36.08,
        lon: -79.95,
    },
    SoundingSite {
        id: "GYX",
        name: "ME, Gray",
        lat: 43.89,
        lon: -70.25,
    },
    SoundingSite {
        id: "IAD",
        name: "DC, Washington",
        lat: 39.08,
        lon: -77.53,
    },
    SoundingSite {
        id: "ILN",
        name: "OH, Wilmington",
        lat: 39.42,
        lon: -83.82,
    },
    SoundingSite {
        id: "ILX",
        name: "IL, Lincoln",
        lat: 40.10,
        lon: -89.30,
    },
    SoundingSite {
        id: "INL",
        name: "MN, International Falls",
        lat: 48.57,
        lon: -93.38,
    },
    SoundingSite {
        id: "JAN",
        name: "MS, Jackson",
        lat: 32.32,
        lon: -90.08,
    },
    SoundingSite {
        id: "JAX",
        name: "FL, Jacksonville",
        lat: 30.43,
        lon: -81.61,
    },
    SoundingSite {
        id: "KEY",
        name: "FL, Key West",
        lat: 24.55,
        lon: -81.75,
    },
    SoundingSite {
        id: "LBF",
        name: "NE, North Platte",
        lat: 41.13,
        lon: -100.68,
    },
    SoundingSite {
        id: "LCH",
        name: "LA, Lake Charles",
        lat: 30.12,
        lon: -93.22,
    },
    SoundingSite {
        id: "LIX",
        name: "LA, New Orleans",
        lat: 30.34,
        lon: -89.83,
    },
    SoundingSite {
        id: "LKN",
        name: "NV, Elko",
        lat: 40.87,
        lon: -115.73,
    },
    SoundingSite {
        id: "LZK",
        name: "AR, Little Rock",
        lat: 34.83,
        lon: -92.25,
    },
    SoundingSite {
        id: "MAF",
        name: "TX, Midland",
        lat: 31.95,
        lon: -102.18,
    },
    SoundingSite {
        id: "MDSD",
        name: "Dominican Republic, Santo Domingo",
        lat: 18.43,
        lon: -69.67,
    },
    SoundingSite {
        id: "MFL",
        name: "FL, Miami",
        lat: 25.75,
        lon: -80.38,
    },
    SoundingSite {
        id: "MFR",
        name: "OR, Medford",
        lat: 42.37,
        lon: -122.87,
    },
    SoundingSite {
        id: "MHX",
        name: "NC, Morehead City",
        lat: 34.70,
        lon: -76.80,
    },
    SoundingSite {
        id: "MKJP",
        name: "Jamaica, Port Royal",
        lat: 17.93,
        lon: -76.78,
    },
    SoundingSite {
        id: "MPX",
        name: "MN, Twin Cities",
        lat: 44.85,
        lon: -93.57,
    },
    SoundingSite {
        id: "NKX",
        name: "CA, San Diego",
        lat: 32.73,
        lon: -117.17,
    },
    SoundingSite {
        id: "NSTU",
        name: "AS, Pago Pago",
        lat: 14.30,
        lon: -170.70,
    },
    SoundingSite {
        id: "OAK",
        name: "CA, Oakland",
        lat: 37.73,
        lon: -122.22,
    },
    SoundingSite {
        id: "OAX",
        name: "NE, Omaha",
        lat: 41.32,
        lon: -96.37,
    },
    SoundingSite {
        id: "OKX",
        name: "NY, New York City",
        lat: 40.86,
        lon: -72.86,
    },
    SoundingSite {
        id: "OTX",
        name: "WA, Spokane",
        lat: 47.68,
        lon: -117.63,
    },
    SoundingSite {
        id: "OUN",
        name: "OK, Norman",
        lat: 35.23,
        lon: -97.47,
    },
    SoundingSite {
        id: "PABE",
        name: "AK, Bethel",
        lat: 60.78,
        lon: -161.80,
    },
    SoundingSite {
        id: "PABR",
        name: "AK, Barrow",
        lat: 71.30,
        lon: -156.78,
    },
    SoundingSite {
        id: "PACD",
        name: "AK, Cold Bay",
        lat: 55.20,
        lon: -162.73,
    },
    SoundingSite {
        id: "PADQ",
        name: "AK, Kodiak",
        lat: 57.75,
        lon: -152.50,
    },
    SoundingSite {
        id: "PAFA",
        name: "AK, Fairbanks",
        lat: 64.82,
        lon: -147.87,
    },
    SoundingSite {
        id: "PAKN",
        name: "AK, King Salmon",
        lat: 58.68,
        lon: -156.65,
    },
    SoundingSite {
        id: "PAMC",
        name: "AK, McGrath",
        lat: 62.95,
        lon: -155.60,
    },
    SoundingSite {
        id: "PANC",
        name: "AK, Anchorage",
        lat: 61.17,
        lon: -150.02,
    },
    SoundingSite {
        id: "PANT",
        name: "AK, Annette",
        lat: 55.03,
        lon: -131.57,
    },
    SoundingSite {
        id: "PAOM",
        name: "AK, Nome",
        lat: 64.50,
        lon: -165.43,
    },
    SoundingSite {
        id: "PAOT",
        name: "AK, Kotzebue",
        lat: 66.87,
        lon: -162.63,
    },
    SoundingSite {
        id: "PASN",
        name: "AK, St Paul Island",
        lat: 57.15,
        lon: -170.22,
    },
    SoundingSite {
        id: "PAYA",
        name: "AK, Yakutat",
        lat: 59.52,
        lon: -139.67,
    },
    SoundingSite {
        id: "PHLI",
        name: "HI, Lihue",
        lat: 21.98,
        lon: -159.35,
    },
    SoundingSite {
        id: "PHTO",
        name: "HI, Hilo",
        lat: 19.72,
        lon: -155.07,
    },
    SoundingSite {
        id: "PIT",
        name: "PA, Pittsburgh",
        lat: 40.50,
        lon: -80.22,
    },
    SoundingSite {
        id: "REV",
        name: "NV, Reno",
        lat: 39.57,
        lon: -119.80,
    },
    SoundingSite {
        id: "RIW",
        name: "WY, Riverton",
        lat: 43.00,
        lon: -108.50,
    },
    SoundingSite {
        id: "RNK",
        name: "VA, Blacksburg",
        lat: 37.21,
        lon: -80.41,
    },
    SoundingSite {
        id: "SGF",
        name: "MO, Springfield",
        lat: 37.14,
        lon: -93.23,
    },
    SoundingSite {
        id: "SHV",
        name: "LA, Shreveport",
        lat: 32.45,
        lon: -93.83,
    },
    SoundingSite {
        id: "SLC",
        name: "UT, Salt Lake City",
        lat: 40.78,
        lon: -111.97,
    },
    SoundingSite {
        id: "SLE",
        name: "OR, Salem",
        lat: 44.92,
        lon: -123.00,
    },
    SoundingSite {
        id: "TBW",
        name: "FL, Tampa Bay",
        lat: 27.70,
        lon: -82.40,
    },
    SoundingSite {
        id: "TFX",
        name: "MT, Great Falls",
        lat: 47.45,
        lon: -111.38,
    },
    SoundingSite {
        id: "TNCC",
        name: "Curaçao, Willemstad",
        lat: 12.20,
        lon: -68.97,
    },
    SoundingSite {
        id: "TOP",
        name: "KS, Topeka",
        lat: 39.07,
        lon: -95.62,
    },
    SoundingSite {
        id: "TUS",
        name: "AZ, Tucson",
        lat: 32.12,
        lon: -110.93,
    },
    SoundingSite {
        id: "UIL",
        name: "WA, Quillayute",
        lat: 47.95,
        lon: -124.55,
    },
    SoundingSite {
        id: "UNR",
        name: "SD, Rapid City",
        lat: 44.07,
        lon: -103.21,
    },
    SoundingSite {
        id: "VBG",
        name: "CA, Vandenberg AFB",
        lat: 34.72,
        lon: -120.57,
    },
    SoundingSite {
        id: "VEF",
        name: "NV, Las Vegas",
        lat: 36.05,
        lon: -115.18,
    },
    SoundingSite {
        id: "WAL",
        name: "VA, Wallops Island",
        lat: 37.85,
        lon: -75.48,
    },
    SoundingSite {
        id: "WMW",
        name: "CAN, QC, Maniwaki",
        lat: 46.00,
        lon: -76.87,
    },
    SoundingSite {
        id: "WPL",
        name: "CAN, ON, Pickle Lake",
        lat: 51.47,
        lon: -90.20,
    },
    SoundingSite {
        id: "YQI",
        name: "CAN, NS, Yarmouth",
        lat: 43.87,
        lon: -66.10,
    },
];

// ── Nearest station lookup ─────────────────────────────────────────────────────

pub fn nearest_site(lat: f64, lon: f64) -> &'static SoundingSite {
    SITES
        .iter()
        .filter(|s| s.lon >= -130.0 && s.lon <= -65.0) // prefer CONUS
        .min_by(|a, b| {
            let da = dist2(a.lat, a.lon, lat, lon);
            let db = dist2(b.lat, b.lon, lat, lon);
            da.partial_cmp(&db).unwrap()
        })
        .unwrap_or(&SITES[0])
}

fn dist2(la: f64, lo: f64, lb: f64, lob: f64) -> f64 {
    let dlat = la - lb;
    let dlon = lo - lob;
    dlat * dlat + dlon * dlon
}

// ── URL helpers ───────────────────────────────────────────────────────────────

const SPC_BASE: &str = "https://www.spc.noaa.gov/exper/soundings/LATEST";

pub fn image_url(site_id: &str) -> String {
    format!("{SPC_BASE}/{site_id}.gif")
}

pub fn text_url(site_id: &str) -> String {
    format!("{SPC_BASE}/{site_id}.txt")
}

// ── Derived indices ───────────────────────────────────────────────────────────

/// Key derived thermodynamic and kinematic indices from the SPC sounding text.
#[derive(Debug, Default, Clone)]
pub struct SoundingIndices {
    pub cape: Option<f64>, // J/kg
    pub cin: Option<f64>,  // J/kg (negative)
    pub li: Option<f64>,   // Lifted Index (°C)
    pub k_index: Option<f64>,
    pub total_totals: Option<f64>,
    pub srh_01km: Option<f64>,   // 0–1 km Storm-Relative Helicity (m²/s²)
    pub srh_03km: Option<f64>,   // 0–3 km SRH
    pub shear_06km: Option<f64>, // 0–6 km bulk wind shear (kt)
    pub lcl_hgt: Option<f64>,    // Lifted Condensation Level (m AGL)
    pub pw: Option<f64>,         // Precipitable Water (in)
    pub raw_text: String,        // full text for display
}

impl SoundingIndices {
    /// Parse key indices from SPC sounding text output.
    pub fn from_text(text: &str) -> Self {
        let mut s = SoundingIndices {
            raw_text: text.to_string(),
            ..Default::default()
        };

        for line in text.lines() {
            let l = line.trim();
            if let Some(v) = parse_value(l, "CAPE:") {
                s.cape = Some(v);
            }
            if let Some(v) = parse_value(l, "CINH:") {
                s.cin = Some(v);
            }
            if let Some(v) = parse_value(l, "Lifted Index:") {
                s.li = Some(v);
            }
            if let Some(v) = parse_value(l, "K index:") {
                s.k_index = Some(v);
            }
            if let Some(v) = parse_value(l, "TT index:") {
                s.total_totals = Some(v);
            }
            if let Some(v) = parse_value(l, "0-1km SRH:") {
                s.srh_01km = Some(v);
            }
            if let Some(v) = parse_value(l, "0-3km SRH:") {
                s.srh_03km = Some(v);
            }
            if let Some(v) = parse_value(l, "0-6km Shear:") {
                s.shear_06km = Some(v);
            }
            if let Some(v) = parse_value(l, "Precip Water:") {
                s.pw = Some(v);
            }
            if let Some(v) = parse_value(l, "LCL =") {
                s.lcl_hgt = Some(v);
            }
        }
        s
    }
}

fn parse_value(line: &str, prefix: &str) -> Option<f64> {
    let rest = line.strip_prefix(prefix)?.trim();
    // Take first whitespace-delimited token
    let tok = rest.split_whitespace().next()?;
    tok.parse::<f64>().ok()
}

// ── Fetchers ──────────────────────────────────────────────────────────────────

pub async fn fetch_image(client: &Client, site_id: &str) -> Result<Vec<u8>> {
    let url = image_url(site_id);
    let cache = Cache::new("soundings");
    if let Some(b) = cache.get(&url) {
        return Ok(b);
    }
    let bytes = client
        .get(&url)
        .send()
        .await
        .context("sounding image fetch failed")?
        .bytes()
        .await
        .context("sounding image body read")?;
    cache.put(&url, &bytes, Duration::from_secs(30 * 60));
    Ok(bytes.to_vec())
}

pub async fn fetch_indices(client: &Client, site_id: &str) -> Result<SoundingIndices> {
    let url = text_url(site_id);
    let cache = Cache::new("soundings");
    let text = if let Some(b) = cache.get(&url) {
        String::from_utf8_lossy(&b).into_owned()
    } else {
        let t = client
            .get(&url)
            .send()
            .await
            .context("sounding text fetch failed")?
            .text()
            .await
            .context("sounding text body read")?;
        cache.put(&url, t.as_bytes(), Duration::from_secs(30 * 60));
        t
    };
    Ok(SoundingIndices::from_text(&text))
}
