/*
 * Surface observation fetching (METAR) and TAF.
 *
 * Station lookup uses a bundled station list (obs_stations.txt) with format:
 *   STATION_ID,STATE_CODE,NAME,LAT,LON
 *
 * Two-step process for location-based obs:
 *   1. NWS `points` API: given lat/lon, find the nearest observation stations.
 *   2. aviationweather.gov METAR API: fetch current decoded obs for those stations.
 *
 * State-based obs: look up station IDs in the bundled list, then batch-fetch METARs.
 * TAF: fetched on demand per station from aviationweather.gov.
 */

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::cache::Cache;

// ── Station registry ─────────────────────────────────────────────────────────

static OBS_STATIONS_TXT: &str = include_str!("obs_stations.txt");

#[derive(Debug, Clone)]
pub struct StationInfo {
    pub id: String,
    pub state: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// All stations keyed by state code (e.g. "NC" → Vec<StationInfo>).
static STATIONS_BY_STATE: OnceLock<HashMap<String, Vec<StationInfo>>> = OnceLock::new();

fn stations_by_state() -> &'static HashMap<String, Vec<StationInfo>> {
    STATIONS_BY_STATE.get_or_init(|| {
        let mut map: HashMap<String, Vec<StationInfo>> = HashMap::new();
        for line in OBS_STATIONS_TXT.lines() {
            let parts: Vec<&str> = line.splitn(5, ',').collect();
            if parts.len() < 5 {
                continue;
            }
            let id = parts[0].trim().to_string();
            let state = parts[1].trim().to_string();
            let name = parts[2].trim().to_string();
            let lat = parts[3].trim().parse::<f64>().unwrap_or(0.0);
            let lon = parts[4].trim().parse::<f64>().unwrap_or(0.0);
            if id.is_empty() || state.is_empty() {
                continue;
            }
            map.entry(state).or_default().push(StationInfo {
                id,
                state: parts[1].trim().to_string(),
                name,
                lat,
                lon,
            });
        }
        // Sort each state's stations alphabetically by name
        for v in map.values_mut() {
            v.sort_by(|a, b| {
                let na = if a.name.is_empty() { &a.id } else { &a.name };
                let nb = if b.name.is_empty() { &b.id } else { &b.name };
                na.cmp(nb)
            });
        }
        map
    })
}

/// Return the list of stations for a state, alphabetically sorted.
pub fn stations_for_state(state_code: &str) -> Vec<StationInfo> {
    stations_by_state()
        .get(state_code)
        .cloned()
        .unwrap_or_default()
}

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ObsStation {
    pub station_id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub station_id: String,
    pub station_name: String,
    /// Raw METAR string (e.g. "KOKC 231652Z 05005KT 10SM SCT015 22/16 A3001 RMK AO2")
    pub raw_metar: String,
    /// Temperature in °F (converted from Celsius), if available.
    pub temp_f: Option<f64>,
    /// Dewpoint in °F (converted from Celsius), if available.
    pub dew_f: Option<f64>,
    /// Wind direction in degrees (0-360, 0 = VRB/calm), if available.
    pub wind_dir: Option<u16>,
    /// Wind speed in knots, if available.
    pub wind_speed_kt: Option<u16>,
    /// Wind gust in knots, if available.
    pub wind_gust_kt: Option<u16>,
    /// Visibility in statute miles.
    pub visibility_mi: Option<f64>,
    /// Altimeter setting in inHg.
    pub altimeter_inhg: Option<f64>,
    /// Sky condition string, e.g. "CLR", "FEW030 BKN120", "OVC010"
    pub sky_cover: String,
    /// ISO-8601 observation time from the API.
    pub obs_time: String,
    /// Flight rules category: VFR, MVFR, IFR, LIFR
    pub flight_category: Option<String>,
}

impl Observation {
    /// Format wind as a short string: "calm", "NW5", "SW12G18", "VRB3"
    pub fn wind_short(&self) -> String {
        match (self.wind_dir, self.wind_speed_kt) {
            (None, _) | (_, None) => "calm".to_string(),
            (Some(0), Some(0)) => "calm".to_string(),
            (Some(dir), Some(spd)) => {
                let compass = deg_to_compass(dir);
                let gust = self
                    .wind_gust_kt
                    .map(|g| format!("G{g}"))
                    .unwrap_or_default();
                format!("{compass}{spd}{gust}kt")
            }
        }
    }
}

// ── NWS station list ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct NwsPointsResponse {
    properties: NwsPointsProperties,
}

#[derive(Deserialize)]
struct NwsPointsProperties {
    #[serde(rename = "observationStations")]
    observation_stations: String,
}

#[derive(Deserialize)]
struct NwsStationsResponse {
    features: Vec<NwsStationFeature>,
}

#[derive(Deserialize)]
struct NwsStationFeature {
    properties: NwsStationProperties,
}

#[derive(Deserialize)]
struct NwsStationProperties {
    #[serde(rename = "stationIdentifier")]
    station_identifier: String,
    name: String,
}

/// Fetch a list of nearby observation station IDs using the NWS API.
/// Returns stations ordered by distance from the given lat/lon.
pub async fn fetch_nearby_stations(
    client: &Client,
    lat: f64,
    lon: f64,
    limit: usize,
) -> Result<Vec<ObsStation>> {
    let url = format!("https://api.weather.gov/points/{lat:.4},{lon:.4}");
    let cache = Cache::new("observations");

    let stations_url = if let Some(bytes) = cache.get(&url) {
        String::from_utf8(bytes).unwrap_or_default()
    } else {
        let resp: NwsPointsResponse = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("NWS points fetch failed: {url}"))?
            .json()
            .await
            .context("NWS points JSON parse failed")?;
        let su = resp.properties.observation_stations;
        cache.put(&url, su.as_bytes(), std::time::Duration::from_secs(86400));
        su
    };

    let stations_resp: NwsStationsResponse = client
        .get(format!("{stations_url}?limit={limit}"))
        .send()
        .await
        .context("NWS stations fetch failed")?
        .json()
        .await
        .context("NWS stations JSON parse failed")?;

    let stations = stations_resp
        .features
        .into_iter()
        .map(|f| ObsStation {
            station_id: f.properties.station_identifier,
            name: f.properties.name,
        })
        .collect();

    Ok(stations)
}

// ── aviationweather.gov METAR fetch ──────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct AvwxMetar {
    #[serde(rename = "rawOb", default)]
    raw_ob: String,
    #[serde(rename = "icaoId", default)]
    station_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    temp: Option<f64>,
    #[serde(default)]
    dewp: Option<f64>,
    #[serde(default)]
    wdir: Option<serde_json::Value>, // can be "VRB" or a number
    #[serde(default)]
    wspd: Option<u16>,
    #[serde(default)]
    wgst: Option<u16>,
    #[serde(default)]
    visib: Option<serde_json::Value>, // "10+", "1 1/4", or number
    #[serde(default)]
    altim: Option<f64>,
    #[serde(default)]
    clouds: Option<Vec<AvwxCloud>>,
    /// Unix timestamp (seconds since epoch)
    #[serde(rename = "obsTime", default)]
    obs_time: Option<serde_json::Value>,
    #[serde(rename = "fltCat", default)]
    flight_category: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
struct AvwxCloud {
    #[serde(default)]
    cover: String,
    #[serde(default)]
    base: Option<u32>,
}

/// Fetch current METAR observations for a list of station IDs.
pub async fn fetch_metars(
    client: &Client,
    station_ids: &[String],
    station_names: &HashMap<String, String>,
) -> Result<Vec<Observation>> {
    if station_ids.is_empty() {
        return Ok(vec![]);
    }
    let ids_csv = station_ids.join(",");
    let url =
        format!("https://aviationweather.gov/api/data/metar?ids={ids_csv}&format=json&taf=false");

    let cache = Cache::new("observations");
    let bytes = if let Some(b) = cache.get(&url) {
        b
    } else {
        let b = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("METAR fetch failed: {url}"))?
            .bytes()
            .await
            .context("METAR read failed")?
            .to_vec();
        cache.put(&url, &b, std::time::Duration::from_secs(600)); // 10min TTL
        b
    };

    let raw: Vec<AvwxMetar> = serde_json::from_slice(&bytes).context("METAR JSON parse failed")?;

    let observations = raw
        .into_iter()
        .map(|m| map_avwx_metar(m, station_names))
        .collect();

    Ok(observations)
}

/// Map a raw `AvwxMetar` struct into an `Observation`, optionally looking up
/// station names from `name_map` when the API does not include them.
fn map_avwx_metar(m: AvwxMetar, name_map: &HashMap<String, String>) -> Observation {
    let name = m
        .name
        .filter(|n| !n.is_empty())
        .or_else(|| name_map.get(&m.station_id).cloned())
        .unwrap_or_default();
    let temp_f = m.temp.map(|c| c * 9.0 / 5.0 + 32.0);
    let dew_f = m.dewp.map(|c| c * 9.0 / 5.0 + 32.0);

    let wind_dir = m.wdir.as_ref().and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_u64().map(|n| n as u16),
        serde_json::Value::String(s) if s == "VRB" => Some(0),
        _ => None,
    });

    let visibility_mi = m.visib.as_ref().and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => {
            if s == "10+" {
                Some(10.0)
            } else {
                parse_visibility_str(s)
            }
        }
        _ => None,
    });

    let sky_cover = build_sky_string(m.clouds.as_deref());

    let obs_time = match &m.obs_time {
        Some(serde_json::Value::Number(n)) => {
            n.as_i64().map(unix_to_obs_time_str).unwrap_or_default()
        }
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => String::new(),
    };

    Observation {
        station_id: m.station_id,
        station_name: name,
        raw_metar: m.raw_ob,
        temp_f,
        dew_f,
        wind_dir,
        wind_speed_kt: m.wspd,
        wind_gust_kt: m.wgst,
        visibility_mi,
        altimeter_inhg: m.altim,
        sky_cover,
        obs_time,
        flight_category: m.flight_category,
    }
}

/// High-level convenience: fetch nearby stations then their METARs in one call.
pub async fn fetch_nearby_observations(
    client: &Client,
    lat: f64,
    lon: f64,
    limit: usize,
) -> Result<Vec<Observation>> {
    let stations = fetch_nearby_stations(client, lat, lon, limit).await?;
    if stations.is_empty() {
        return Ok(vec![]);
    }
    let ids: Vec<String> = stations.iter().map(|s| s.station_id.clone()).collect();
    let name_map: HashMap<String, String> = stations
        .iter()
        .map(|s| (s.station_id.clone(), s.name.clone()))
        .collect();
    fetch_metars(client, &ids, &name_map).await
}

/// Fetch all current METAR observations for a US state code (e.g. "NC", "TX").
///
/// Uses the bundled station list to find station IDs for the state, then
/// batch-fetches METARs from aviationweather.gov.  Results are cached for 10 minutes.
pub async fn fetch_metars_for_state(client: &Client, state_code: &str) -> Result<Vec<Observation>> {
    let stations = stations_for_state(state_code);
    if stations.is_empty() {
        return Ok(vec![]);
    }
    let ids: Vec<String> = stations.iter().map(|s| s.id.clone()).collect();
    let name_map: HashMap<String, String> = stations
        .iter()
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect();
    fetch_metars(client, &ids, &name_map).await
}

// ── TAF types ─────────────────────────────────────────────────────────────────

/// A single TAF forecast period.
#[derive(Debug, Clone)]
pub struct TafPeriod {
    /// Unix epoch start of this period.
    pub time_from: i64,
    /// Unix epoch end of this period.
    pub time_to: i64,
    /// Change indicator: None, "TEMPO", "BECMG", "FM", "PROB30", "PROB40"
    pub change_type: Option<String>,
    /// Wind direction (0 = calm/VRB), if available.
    pub wind_dir: Option<u16>,
    /// Wind speed in knots.
    pub wind_speed: Option<u16>,
    /// Wind gust in knots.
    pub wind_gust: Option<u16>,
    /// Visibility (statute miles or "6+").
    pub visibility: Option<String>,
    /// Weather string (e.g. "-SHRA", "TSRA").
    pub wx_string: Option<String>,
    /// Sky cover string (e.g. "OVC010 SCT040").
    pub sky_cover: String,
}

/// A TAF for a single station.
#[derive(Debug, Clone)]
pub struct Taf {
    pub station_id: String,
    pub name: String,
    /// Raw TAF text.
    pub raw_taf: String,
    /// Issue time (Unix epoch).
    pub issue_time: i64,
    /// Valid-from (Unix epoch).
    pub valid_from: i64,
    /// Valid-to (Unix epoch).
    pub valid_to: i64,
    pub periods: Vec<TafPeriod>,
}

#[derive(Deserialize, Debug)]
struct RawTaf {
    #[serde(rename = "icaoId", default)]
    station_id: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "rawTAF", default)]
    raw_taf: String,
    #[serde(rename = "issueTime", default)]
    issue_time: Option<serde_json::Value>,
    #[serde(rename = "validTimeFrom", default)]
    valid_from: Option<i64>,
    #[serde(rename = "validTimeTo", default)]
    valid_to: Option<i64>,
    #[serde(default)]
    fcsts: Vec<RawTafPeriod>,
}

#[derive(Deserialize, Debug)]
struct RawTafPeriod {
    #[serde(rename = "timeFrom", default)]
    time_from: Option<i64>,
    #[serde(rename = "timeTo", default)]
    time_to: Option<i64>,
    #[serde(rename = "fcstChange", default)]
    fcst_change: Option<String>,
    #[serde(default)]
    wdir: Option<serde_json::Value>,
    #[serde(default)]
    wspd: Option<u16>,
    #[serde(default)]
    wgst: Option<u16>,
    #[serde(default)]
    visib: Option<serde_json::Value>,
    #[serde(rename = "wxString", default)]
    wx_string: Option<String>,
    #[serde(default)]
    clouds: Option<Vec<AvwxCloud>>,
}

/// Fetch the current TAF for a station.  Returns `None` if no TAF exists.
/// Results are cached for 30 minutes.
pub async fn fetch_taf(client: &Client, station_id: &str) -> Result<Option<Taf>> {
    let url = format!(
        "https://aviationweather.gov/api/data/taf?ids={station_id}&format=json&metar=false"
    );

    let cache = Cache::new("observations");
    let bytes = if let Some(b) = cache.get(&url) {
        b
    } else {
        let b = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("TAF fetch failed: {url}"))?
            .bytes()
            .await
            .context("TAF read failed")?
            .to_vec();
        cache.put(&url, &b, std::time::Duration::from_secs(1800)); // 30 min
        b
    };

    let raw_list: Vec<RawTaf> = serde_json::from_slice(&bytes)
        .with_context(|| format!("TAF JSON parse failed for {station_id}"))?;

    let raw = match raw_list.into_iter().next() {
        Some(r) => r,
        None => return Ok(None),
    };

    let issue_time = match &raw.issue_time {
        Some(serde_json::Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    };

    let periods = raw
        .fcsts
        .into_iter()
        .map(|p| {
            let wind_dir = p.wdir.as_ref().and_then(|v| match v {
                serde_json::Value::Number(n) => n.as_u64().map(|n| n as u16),
                serde_json::Value::String(s) if s == "VRB" => Some(0),
                _ => None,
            });
            let visibility = p
                .visib
                .as_ref()
                .and_then(|v| match v {
                    serde_json::Value::Number(n) => Some(format!("{}", n.as_f64().unwrap_or(0.0))),
                    serde_json::Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .filter(|s| !s.is_empty());
            TafPeriod {
                time_from: p.time_from.unwrap_or(0),
                time_to: p.time_to.unwrap_or(0),
                change_type: p.fcst_change,
                wind_dir,
                wind_speed: p.wspd,
                wind_gust: p.wgst,
                visibility,
                wx_string: p.wx_string.filter(|s| !s.is_empty()),
                sky_cover: build_sky_string(p.clouds.as_deref()),
            }
        })
        .collect();

    Ok(Some(Taf {
        station_id: raw.station_id,
        name: raw.name,
        raw_taf: raw.raw_taf,
        issue_time,
        valid_from: raw.valid_from.unwrap_or(0),
        valid_to: raw.valid_to.unwrap_or(0),
        periods,
    }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert wind degrees to 16-point compass abbreviation.
pub fn deg_to_compass(deg: u16) -> &'static str {
    const DIRS: &[&str] = &[
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let idx = ((deg as f32 + 11.25) / 22.5) as usize % 16;
    DIRS[idx]
}

/// Parse a fractional visibility string like "1 1/4" → 1.25, "3/4" → 0.75.
fn parse_visibility_str(s: &str) -> Option<f64> {
    let s = s.trim();
    // Try pure number first
    if let Ok(v) = s.parse::<f64>() {
        return Some(v);
    }
    // "A B/C" or "B/C"
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.as_slice() {
        [whole, frac] => {
            let w: f64 = whole.parse().ok()?;
            let f = parse_fraction(frac)?;
            Some(w + f)
        }
        [frac] => parse_fraction(frac),
        _ => None,
    }
}

fn parse_fraction(s: &str) -> Option<f64> {
    let mut it = s.splitn(2, '/');
    let num: f64 = it.next()?.parse().ok()?;
    let den: f64 = it.next()?.parse().ok()?;
    if den == 0.0 {
        return None;
    }
    Some(num / den)
}

/// Build a compact sky cover string from cloud layers.
/// e.g. "FEW030 BKN080 OVC200"
fn build_sky_string(clouds: Option<&[AvwxCloud]>) -> String {
    match clouds {
        None | Some([]) => "CLR".to_string(),
        Some(layers) => {
            let parts: Vec<String> = layers
                .iter()
                .filter(|c| !c.cover.is_empty() && c.cover != "CLR" && c.cover != "SKC")
                .map(|c| {
                    if let Some(base) = c.base {
                        format!("{}{:03}", c.cover, base / 100)
                    } else {
                        c.cover.clone()
                    }
                })
                .collect();
            if parts.is_empty() {
                "CLR".to_string()
            } else {
                parts.join(" ")
            }
        }
    }
}

/// Convert a Unix epoch timestamp to a stored obs_time string (epoch as string).
/// This is used so time_ago_str can parse it back efficiently.
fn unix_to_obs_time_str(ts: i64) -> String {
    ts.to_string()
}

/// Parse ISO-8601 obs time and return a human-readable "Xm ago" / "Xh ago" string.
/// Also accepts Unix epoch seconds as a decimal string.
pub fn time_ago_str(obs_time: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Try parsing as a Unix timestamp first (what the API now returns)
    let secs = if let Ok(ts) = obs_time.parse::<u64>() {
        ts
    } else {
        // Fall back to ISO-8601 parsing
        let ts = obs_time.trim_end_matches('Z').trim_end_matches("+00:00");
        match parse_iso_to_unix(ts) {
            Some(s) => s,
            None => return obs_time.to_string(),
        }
    };

    let diff = now.saturating_sub(secs);
    let mins = diff / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    let rem_mins = mins % 60;
    if rem_mins == 0 {
        format!("{hours}h ago")
    } else {
        format!("{hours}h{rem_mins}m ago")
    }
}

fn parse_iso_to_unix(s: &str) -> Option<u64> {
    // "YYYY-MM-DDTHH:MM:SS"
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let year: u64 = s[0..4].parse().ok()?;
    let month: u64 = s[5..7].parse().ok()?;
    let day: u64 = s[8..10].parse().ok()?;
    let hour: u64 = s[11..13].parse().ok()?;
    let min: u64 = s[14..16].parse().ok()?;
    let sec: u64 = s[17..19].parse().ok()?;

    // Quick and dirty: days-since-epoch approximation
    // Using Julian day method for accuracy
    let a = (14u64.saturating_sub(month)) / 12;
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    let jdn = day + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045;
    let unix_epoch_jdn: u64 = 2440588;
    let days = jdn.saturating_sub(unix_epoch_jdn);
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}
