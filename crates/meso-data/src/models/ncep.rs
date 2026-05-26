/*
 * NCEP MAG (Model Analysis and Guidance) model data fetching.
 *
 * Models: GFS, NAM, RAP, HRRR
 *
 * Base URL: https://mag.ncep.noaa.gov
 * Image URL patterns:
 *   GFS:  /data/gfs/{run}/{sector}/{param}/gfs_{sector}_{fhr}_{param}.gif
 *   HRRR: /data/hrrr/{run}/hrrr_{sector}_{fhr}_{param}.gif
 *   NAM/RAP: /data/{model}/{run}/{model}_{sector}_{fhr}_{param}.gif
 *
 * Run time is determined by querying the MAG web page for the latest cycle.
 * Forecast hours vary by model (GFS: 0-240/3hr; HRRR: 0-48/1hr; NAM: 0-84/3hr; RAP: 0-51/1hr)
 */

use anyhow::{bail, Context, Result};
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use std::time::Duration;

use crate::cache::Cache;

const MAG_BASE: &str = "https://mag.ncep.noaa.gov";

// ── Product catalog ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct NcepProduct {
    pub id: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone)]
pub struct NcepProductCategory {
    pub name: &'static str,
    pub products: &'static [NcepProduct],
}

macro_rules! prods {
    ($(($id:expr, $lbl:expr)),* $(,)?) => {
        &[$(NcepProduct { id: $id, label: $lbl }),*]
    }
}

// ── GFS products ──────────────────────────────────────────────────────────────

static GFS_PRECIP: &[NcepProduct] = prods![
    ("precip_p01", "Precip (1hr)"),
    ("precip_p03", "Precip (3hr)"),
    ("precip_p06", "Precip (6hr)"),
    ("precip_p12", "Precip (12hr)"),
    ("precip_p24", "Precip (24hr)"),
    ("precip_ptot", "Total Accumulated Precip"),
    ("precip_rate_type", "Precip Rate / 1000-500mb Thickness"),
    ("sim_radar_comp", "Composite Simulated Radar"),
    ("snodpth_chng", "Snow Depth Change from F00"),
];

static GFS_SURFACE: &[NcepProduct] = prods![
    ("10m_wnd_2m_temp", "MSLP / 10m Wind / 2m Temp"),
    ("10m_wnd_precip", "MSLP / 10m Wind / Precip"),
    ("850_temp_mslp_precip", "MSLP / 850mb Temp / Precip"),
    ("1000_500_thick", "MSLP / 1000-500mb Thickness"),
    ("1000_850_thick", "MSLP / 1000-850mb Thickness"),
    ("850_700_thick", "MSLP / 850-700mb Thickness"),
];

static GFS_UPPER_AIR: &[NcepProduct] = prods![
    ("200_wnd_ht", "200mb Wind and Height"),
    ("250_wnd_ht", "250mb Wind and Height"),
    ("300_wnd_ht", "300mb Wind and Height"),
    ("500_vort_ht", "500mb Vorticity, Wind and Height"),
    ("500_wnd_ht", "500mb Wind and Height"),
    ("500_rh_ht", "500mb RH and Height"),
    ("700_rh_ht", "700mb RH, Height and Omega"),
    ("850_temp_ht", "850mb Temp, Wind and Height"),
    ("850_rh_ht", "850mb RH and Height"),
    ("850_pw_ht", "850mb Height / PW / Wind"),
    ("850_vort_ht", "850mb Vorticity, Wind and Height"),
    (
        "850vor_500ht_200wd",
        "850mb Vort / 500mb Height / 200mb Wind"
    ),
    ("925_temp_ht", "925mb Temp, Wind and Height"),
];

pub static GFS_CATEGORIES: &[NcepProductCategory] = &[
    NcepProductCategory {
        name: "GFS Precipitation",
        products: GFS_PRECIP,
    },
    NcepProductCategory {
        name: "GFS Surface",
        products: GFS_SURFACE,
    },
    NcepProductCategory {
        name: "GFS Upper Air",
        products: GFS_UPPER_AIR,
    },
];

pub static GFS_SECTORS: &[&str] = &[
    "CONUS",
    "NAMER",
    "ALASKA",
    "SAMER",
    "AFRICA",
    "NORTH-PAC",
    "EAST-PAC",
    "WEST-ATL",
    "ATLANTIC",
    "POLAR",
    "EUROPE",
    "ASIA",
    "SOUTH-PAC",
];

// GFS forecast hours: 0-240 every 3hr, then every 12hr to 384
pub fn gfs_hours() -> Vec<u16> {
    let mut hours: Vec<u16> = (0..=240).step_by(3).collect();
    let extended: Vec<u16> = (252..=384).step_by(12).collect();
    hours.extend(extended);
    hours
}

// ── NAM products ──────────────────────────────────────────────────────────────

static NAM_PRECIP: &[NcepProduct] = prods![
    ("precip_p01", "Precip (1hr)"),
    ("precip_p03", "Precip (3hr)"),
    ("precip_p06", "Precip (6hr)"),
    ("precip_ptot", "Total Accumulated Precip"),
    ("precip_rate_type", "Precip Rate / 1000-500mb Thickness"),
    ("sim_radar_1km", "Simulated Radar 1km"),
    ("snodpth_chng", "Snow Depth Change from F00"),
];

static NAM_SURFACE: &[NcepProduct] = prods![
    ("10m_wnd_2m_temp", "MSLP / 10m Wind / 2m Temp"),
    ("10m_wnd_precip", "MSLP / 10m Wind / Precip"),
    ("850_temp_mslp_precip", "MSLP / 850mb Temp / Precip"),
    ("1000_500_thick", "MSLP / 1000-500mb Thickness"),
    ("1000_850_thick", "MSLP / 1000-850mb Thickness"),
    ("850_700_thick", "MSLP / 850-700mb Thickness"),
];

static NAM_UPPER_AIR: &[NcepProduct] = prods![
    ("200_wnd_ht", "200mb Wind and Height"),
    ("250_wnd_ht", "250mb Wind and Height"),
    ("300_wnd_ht", "300mb Wind and Height"),
    ("500_vort_ht", "500mb Vorticity, Wind and Height"),
    ("500_wnd_ht", "500mb Wind and Height"),
    ("500_rh_ht", "500mb RH and Height"),
    ("700_rh_ht", "700mb RH, Height and Omega"),
    ("850_temp_ht", "850mb Temp, Wind and Height"),
    ("850_rh_ht", "850mb RH and Height"),
    ("850_pw_ht", "850mb Height / PW / Wind"),
    ("850_vort_ht", "850mb Vorticity, Wind and Height"),
    ("925_temp_ht", "925mb Temp, Wind and Height"),
];

pub static NAM_CATEGORIES: &[NcepProductCategory] = &[
    NcepProductCategory {
        name: "NAM Precipitation",
        products: NAM_PRECIP,
    },
    NcepProductCategory {
        name: "NAM Surface",
        products: NAM_SURFACE,
    },
    NcepProductCategory {
        name: "NAM Upper Air",
        products: NAM_UPPER_AIR,
    },
];

pub static NAM_SECTORS: &[&str] = &["CONUS", "NAMER", "NORTH-PAC", "EAST-PAC", "WN-ATL"];

// NAM forecast hours: 0-84 every 3hr
pub fn nam_hours() -> Vec<u16> {
    (0..=84).step_by(3).collect()
}

// ── RAP products ──────────────────────────────────────────────────────────────

static RAP_PRODUCTS: &[NcepProduct] = prods![
    ("precip_p01", "Hourly Total Precip"),
    ("precip_ptot", "Total Accumulated Precip"),
    ("precip_rate", "Precipitation Rate"),
    ("snow_total", "Total Accumulated Snowfall"),
    ("sim_radar_1km", "Simulated Radar 1km"),
    ("sim_radar_comp", "Composite Simulated Radar"),
    ("1000_500_thick", "MSLP / 1000-500mb Thickness"),
    ("1000_850_thick", "MSLP / 1000-850mb Thickness"),
    ("850_700_thick", "MSLP / 850-700mb Thickness"),
    ("cape_cin", "CAPE / CIN"),
    ("helicity", "Helicity and 30m Wind"),
    ("2m_temp_10m_wnd", "2m Temp / 10m Wind"),
    ("2m_dewp_10m_wnd", "2m Dewpoint / 10m Wind"),
    ("10m_wnd_sfc_gust", "10m Wind Gust"),
    ("echo_top", "Echo Tops"),
    ("vis", "Visibility"),
    ("200_wnd_ht", "200mb Wind and Height"),
    ("250_wnd_ht", "250mb Wind and Height"),
    ("300_wnd_ht", "300mb Wind and Height"),
    ("500_vort_ht", "500mb Vorticity, Wind and Height"),
    ("500_temp_ht", "500mb Temp, Wind and Height"),
    ("700_rh_ht", "700mb RH, Wind, Height and Omega"),
    ("850_temp_ht", "850mb Temp, Wind and Height"),
    ("925_temp_ht", "925mb Temp, Wind and Height"),
];

pub static RAP_CATEGORIES: &[NcepProductCategory] = &[NcepProductCategory {
    name: "RAP Products",
    products: RAP_PRODUCTS,
}];

pub static RAP_SECTORS: &[&str] = &["CONUS", "NAMER"];

// RAP forecast hours: 0-51 every 1hr
pub fn rap_hours() -> Vec<u16> {
    (0..=51).collect()
}

// ── HRRR products ─────────────────────────────────────────────────────────────

static HRRR_PRECIP: &[NcepProduct] = prods![
    ("precip_p01", "Hourly Total Precip"),
    ("precip_ptot", "Total Accumulated Precip"),
    ("precip_rate", "Precipitation Rate"),
    ("precip_rate_type", "Precip Rate / 1000-500mb Thickness"),
    ("snow_total", "Total Accumulated Snowfall"),
    ("sim_radar_1km", "Simulated Radar 1km"),
    ("sim_radar_comp", "Composite Simulated Radar"),
    ("sim_radar_max", "Max Simulated Radar"),
];

static HRRR_SEVERE: &[NcepProduct] = prods![
    ("helicity_1km", "0-1km Helicity / Storm Motion"),
    ("helicity_3km", "0-3km Helicity / Storm Motion"),
    ("max_updraft_hlcy", "Max 2-5km Updraft Helicity"),
    ("accu_max_updraft_hlcy", "Accumulated Max Updraft Helicity"),
    ("sfc_cape_cin", "SFC CAPE/CIN"),
    ("best_cape_cin", "Most Unstable CAPE/CIN"),
    ("lightning", "Lightning Flash Rate"),
];

static HRRR_SURFACE: &[NcepProduct] = prods![
    ("1000_500_thick", "MSLP / 1000-500mb Thickness"),
    ("1000_850_thick", "MSLP / 1000-850mb Thickness"),
    ("850_700_thick", "MSLP / 850-700mb Thickness"),
    ("850_temp_mslp_precip", "MSLP / 850mb Temp / Precip"),
    ("10m_wnd", "10m Wind"),
    ("10m_maxwnd", "Max 10m Wind Speed"),
    ("2m_temp_10m_wnd", "2m Temp / 10m Wind"),
    ("2m_dewp_10m_wnd", "2m Dewpoint / 10m Wind"),
    ("10m_wnd_sfc_gust", "10m Wind Gust"),
    ("echo_top", "Echo Tops"),
    ("ceiling", "Cloud Ceiling"),
    ("vis", "Visibility"),
];

static HRRR_UPPER_AIR: &[NcepProduct] = prods![
    ("250_wnd", "250mb Wind"),
    ("300_wnd", "300mb Wind"),
    ("500_vort_ht", "500mb Vorticity, Wind and Height"),
    ("500_temp_ht", "500mb Temp, Wind and Height"),
    ("700_rh_ht", "700mb RH, Wind, Height and Omega"),
    ("850_temp_ht", "850mb Temp, Wind and Height"),
    ("925_temp_wnd", "925mb Temp and Wind"),
];

pub static HRRR_CATEGORIES: &[NcepProductCategory] = &[
    NcepProductCategory {
        name: "HRRR Precipitation",
        products: HRRR_PRECIP,
    },
    NcepProductCategory {
        name: "HRRR Severe",
        products: HRRR_SEVERE,
    },
    NcepProductCategory {
        name: "HRRR Surface",
        products: HRRR_SURFACE,
    },
    NcepProductCategory {
        name: "HRRR Upper Air",
        products: HRRR_UPPER_AIR,
    },
];

pub static HRRR_SECTORS: &[&str] = &[
    "CONUS", "US-NW", "US-SW", "US-NC", "US-SC", "US-NE", "US-SE", "ALASKA",
];

// HRRR forecast hours: 0-48 every 1hr
pub fn hrrr_hours() -> Vec<u16> {
    (0..=48).collect()
}

// ── Model types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NcepModel {
    Gfs,
    Nam,
    Rap,
    Hrrr,
}

impl NcepModel {
    pub fn label(&self) -> &'static str {
        match self {
            NcepModel::Gfs => "GFS",
            NcepModel::Nam => "NAM",
            NcepModel::Rap => "RAP",
            NcepModel::Hrrr => "HRRR",
        }
    }

    pub fn short(&self) -> &'static str {
        match self {
            NcepModel::Gfs => "gfs",
            NcepModel::Nam => "nam",
            NcepModel::Rap => "rap",
            NcepModel::Hrrr => "hrrr",
        }
    }

    pub fn categories(&self) -> &'static [NcepProductCategory] {
        match self {
            NcepModel::Gfs => GFS_CATEGORIES,
            NcepModel::Nam => NAM_CATEGORIES,
            NcepModel::Rap => RAP_CATEGORIES,
            NcepModel::Hrrr => HRRR_CATEGORIES,
        }
    }

    pub fn sectors(&self) -> &'static [&'static str] {
        match self {
            NcepModel::Gfs => GFS_SECTORS,
            NcepModel::Nam => NAM_SECTORS,
            NcepModel::Rap => RAP_SECTORS,
            NcepModel::Hrrr => HRRR_SECTORS,
        }
    }

    pub fn forecast_hours(&self) -> Vec<u16> {
        match self {
            NcepModel::Gfs => gfs_hours(),
            NcepModel::Nam => nam_hours(),
            NcepModel::Rap => rap_hours(),
            NcepModel::Hrrr => hrrr_hours(),
        }
    }

    pub fn all() -> &'static [NcepModel] {
        &[
            NcepModel::Gfs,
            NcepModel::Nam,
            NcepModel::Rap,
            NcepModel::Hrrr,
        ]
    }
}

// ── URL builders ──────────────────────────────────────────────────────────────

/// Build the image URL for an NCEP model frame.
///
/// `run` — cycle time in `YYYYMMDDHH` format (e.g. "2024052412")
/// `sector` — area code (e.g. "CONUS")
/// `param` — product parameter string
/// `fhr` — forecast hour (0-based)
pub fn frame_url(model: &NcepModel, run: &str, sector: &str, param: &str, fhr: u16) -> String {
    let m = model.short();
    let sec = sector.to_lowercase();
    // MAG frame paths use the cycle hour directory token (e.g. "00", "06"),
    // while the cycle picker includes the full date + hour.
    let run_token = cycle_hour_token(run);
    match model {
        NcepModel::Gfs => {
            let fhr_str = format!("{fhr:03}");
            format!("{MAG_BASE}/data/{m}/{run_token}/{sec}/{param}/{m}_{sec}_{fhr_str}_{param}.gif")
        }
        NcepModel::Hrrr => {
            let fhr_str = format!("{fhr:03}");
            format!("{MAG_BASE}/data/{m}/{run_token}/{m}_{sec}_{fhr_str}_{param}.gif")
        }
        _ => {
            let fhr_str = format!("{fhr:03}");
            format!("{MAG_BASE}/data/{m}/{run_token}/{m}_{sec}_{fhr_str}_{param}.gif")
        }
    }
}

// ── Run time determination ────────────────────────────────────────────────────

/// Fetch the latest available run time for an NCEP model + sector.
/// Returns `YYYYMMDDHH` string (e.g. "2024052412").
pub async fn fetch_latest_run(
    client: &Client,
    model: &NcepModel,
    sector: &str,
    first_param: &str,
) -> Result<String> {
    let m_upper = model.label();
    let url = format!(
        "{MAG_BASE}/model-guidance-model-parameter.php?group=Model%20Guidance&model={m_upper}&area={sector}&ps=area"
    );

    let cache = Cache::new("ncep_run");
    if let Some(b) = cache.get(&url) {
        return Ok(String::from_utf8_lossy(&b).into_owned());
    }

    let html = client
        .get(&url)
        .send()
        .await
        .context("NCEP run-time page fetch")?
        .text()
        .await
        .context("NCEP run-time page read")?;

    // Parse: data-cycle-date="2024 0524 12 UTC"  → strip spaces/UTC → "2024052412"
    let run =
        parse_run_from_html(&html, model, sector, first_param).unwrap_or_else(default_run_time);

    cache.put(&url, run.as_bytes(), Duration::from_secs(30 * 60));
    Ok(run)
}

fn parse_run_from_html(
    html: &str,
    _model: &NcepModel,
    _sector: &str,
    _param: &str,
) -> Option<String> {
    // MAG emits data-cycle-date with either quote style:
    //   data-cycle-date="2024 0524 12 UTC" or data-cycle-date='20260525 00 UTC'
    for marker in ["data-cycle-date=\"", "data-cycle-date='"] {
        if let Some(start) = html.find(marker) {
            let start = start + marker.len();
            let quote = if marker.ends_with('"') { '"' } else { '\'' };
            if let Some(end_rel) = html[start..].find(quote) {
                let raw = &html[start..start + end_rel];
                if let Some(run) = normalize_run(raw) {
                    return Some(run);
                }
            }
        }
    }
    None
}

fn default_run_time() -> String {
    // Fall back to most recent synoptic hour (00/06/12/18)
    use chrono::Timelike;
    let now = chrono::Utc::now();
    let hour = (now.hour() / 6) * 6;
    format!("{}{:02}", now.format("%Y%m%d"), hour)
}

fn normalize_run(raw: &str) -> Option<String> {
    // Accept either "YYYY MMDD HH UTC" or "YYYYMMDD HH UTC".
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 10 {
        Some(digits[..10].to_string())
    } else {
        None
    }
}

fn cycle_hour_token(run: &str) -> String {
    let digits: String = run.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 2 {
        digits[digits.len() - 2..].to_string()
    } else {
        "00".to_string()
    }
}

// ── Image fetch ───────────────────────────────────────────────────────────────

pub async fn fetch_frame(
    client: &Client,
    model: &NcepModel,
    run: &str,
    sector: &str,
    param: &str,
    fhr: u16,
) -> Result<Vec<u8>> {
    let cache = Cache::new("ncep_frames");
    let url = frame_url(model, run, sector, param, fhr);
    if let Some(b) = cache.get(&url) {
        return Ok(b);
    }
    match fetch_frame_once(client, &url).await {
        Ok(bytes) => {
            cache.put(&url, &bytes, Duration::from_secs(60 * 60));
            Ok(bytes)
        }
        Err(primary_err) => {
            // Some products (notably precip accum/rate families) may not provide F+000.
            // If that happens, retry from the product's natural first forecast hour.
            if fhr == 0 {
                let fallback_hour = first_hour_for_param(param).unwrap_or(1);
                if fallback_hour != 0 {
                    let fallback_url = frame_url(model, run, sector, param, fallback_hour);
                    if let Ok(bytes) = fetch_frame_once(client, &fallback_url).await {
                        cache.put(&url, &bytes, Duration::from_secs(60 * 60));
                        cache.put(&fallback_url, &bytes, Duration::from_secs(60 * 60));
                        return Ok(bytes);
                    }
                }
            }
            Err(primary_err)
        }
    }
}

async fn fetch_frame_once(client: &Client, url: &str) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("NCEP frame fetch failed for {url}"))?
        .error_for_status()
        .with_context(|| format!("NCEP frame HTTP status error for {url}"))?;
    if let Some(ct) = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
    {
        if !ct.starts_with("image/") {
            bail!("NCEP frame response was not an image for {url} (content-type: {ct})");
        }
    }
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("NCEP frame body read failed for {url}"))?;
    Ok(bytes.to_vec())
}

fn first_hour_for_param(param: &str) -> Option<u16> {
    // Examples: precip_p01, precip_p03, precip_p06, precip_p12, precip_p24
    let idx = param.find("_p")?;
    let digits = &param[idx + 2..];
    let hh: String = digits.chars().take_while(|c| c.is_ascii_digit()).collect();
    if hh.is_empty() {
        None
    } else {
        hh.parse::<u16>().ok()
    }
}
