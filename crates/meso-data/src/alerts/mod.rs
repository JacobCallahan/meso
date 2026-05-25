/*
 * NWS alerts and SPC products fetching and parsing.
 *
 * Fetches active alerts from api.weather.gov and parses the JSON-LD format
 * into structured warning objects. Also provides SPC watch/MCD/MPD fetching.
 *
 * Ported from wX's ObjectWarning.kt, CapAlert.kt, UtilitySpc.kt.
 */

use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;

use crate::geo::latlon::LatLon;
use crate::geo::sites;

// ── Data structures ───────────────────────────────────────────────────────────

/// A single NWS active alert / warning.
#[derive(Debug, Clone)]
pub struct Warning {
    /// Alert URL (api.weather.gov URN)
    pub url: String,
    /// Geographic area description
    pub area: String,
    /// Effective time string
    pub effective: String,
    /// Expiration time string
    pub expires: String,
    /// Event type (e.g. "Tornado Warning", "Severe Thunderstorm Watch")
    pub event: String,
    /// Issuing WFO name
    pub sender: String,
    /// Polygon coordinates as raw string from API
    pub polygon_raw: String,
    /// VTEC string
    pub vtec: String,
    /// Whether this alert is still active (based on VTEC EXP/CAN status)
    pub is_current: bool,
    /// Decoded polygon lat/lon pairs, if available
    pub polygon: Vec<LatLon>,
}

impl Warning {
    pub fn new(
        url: String,
        area: String,
        effective: String,
        expires: String,
        event: String,
        sender: String,
        polygon_raw: String,
        vtec: String,
    ) -> Self {
        let effective = clean_time(&effective);
        let expires = clean_time(&expires);
        let is_current = vtec_is_current(&vtec);
        let polygon = parse_polygon(&polygon_raw);
        Warning {
            url,
            area,
            effective,
            expires,
            event,
            sender,
            polygon_raw,
            vtec,
            is_current,
            polygon,
        }
    }

    /// Return the closest NEXRAD radar site to the warning polygon centroid.
    pub fn closest_radar(&self) -> Option<String> {
        if self.polygon.is_empty() {
            return None;
        }
        let lat_sum: f64 = self.polygon.iter().map(|p| p.lat).sum();
        let lon_sum: f64 = self.polygon.iter().map(|p| p.lon).sum();
        let n = self.polygon.len() as f64;
        let centroid = LatLon {
            lat: lat_sum / n,
            lon: lon_sum / n,
        };
        Some(sites::nearest_site(&centroid, false).to_string())
    }

    /// Return a short label combining event type and sender.
    pub fn label(&self) -> String {
        format!("{} — {}", self.event, self.sender)
    }
}

// ── VTEC helpers ──────────────────────────────────────────────────────────────

/// Determine if a VTEC string represents a currently active event.
fn vtec_is_current(vtec: &str) -> bool {
    if vtec.starts_with("O.EXP") || vtec.starts_with("O.CAN") {
        return false;
    }
    // TODO: Parse times and compare to now if needed
    true
}

fn clean_time(t: &str) -> String {
    let re = Regex::new(r":00-0\d:00").unwrap();
    re.replace_all(&t.replace('T', " "), "").to_string()
}

fn parse_polygon(raw: &str) -> Vec<LatLon> {
    // raw is like "-97.5,35.2 -97.6,35.3 ..."
    let mut points = Vec::new();
    let cleaned = raw.replace('[', "").replace(']', "").replace(',', " ");
    let nums: Vec<f64> = cleaned
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    // NWS coords are [lon, lat] pairs
    let mut i = 0;
    while i + 1 < nums.len() {
        points.push(LatLon {
            lat: nums[i + 1],
            lon: nums[i],
        });
        i += 2;
    }
    points
}

// ── API fetching ──────────────────────────────────────────────────────────────

const NWS_API: &str = "https://api.weather.gov";

/// Full detail text for a single NWS alert fetched from the alert's own URL.
#[derive(Debug, Clone)]
pub struct AlertDetail {
    pub headline: String,
    pub description: String,
    pub instruction: String,
}

/// Fetch the full description/instruction text for a specific alert by its URL.
pub async fn fetch_alert_detail(client: &Client, alert_url: &str) -> Result<AlertDetail> {
    let resp: serde_json::Value = client
        .get(alert_url)
        .header("Accept", "application/geo+json")
        .send()
        .await
        .context("NWS alert detail request failed")?
        .json()
        .await
        .context("NWS alert detail JSON parse failed")?;

    let props = &resp["properties"];
    Ok(AlertDetail {
        headline: props["headline"].as_str().unwrap_or("").to_string(),
        description: props["description"].as_str().unwrap_or("").to_string(),
        instruction: props["instruction"].as_str().unwrap_or("").to_string(),
    })
}

/// Fetch all active alerts for a given state (two-letter code) or area.
/// Pass `area` as a two-letter state code (e.g. "OK") or "US" for all.
pub async fn fetch_active_alerts(client: &Client, area: &str) -> Result<Vec<Warning>> {
    let url = if area.eq_ignore_ascii_case("US") {
        format!("{NWS_API}/alerts/active?status=actual")
    } else {
        format!("{NWS_API}/alerts/active?area={area}&status=actual")
    };

    let text = client
        .get(&url)
        .header("Accept", "application/geo+json")
        .send()
        .await
        .context("NWS alerts request failed")?
        .text()
        .await
        .context("NWS alerts response read failed")?;

    parse_alerts_json(&text)
}

/// Fetch all active NWS warnings for a US state (e.g. "NC").
pub async fn fetch_active_alerts_by_state(
    client: &reqwest::Client,
    state: &str,
) -> Result<Vec<Warning>> {
    fetch_active_alerts(client, state).await
}

/// Fetch alerts for a specific NWS zone or point.
pub async fn fetch_alerts_for_point(client: &Client, lat: f64, lon: f64) -> Result<Vec<Warning>> {
    let url = format!("{NWS_API}/alerts/active?point={lat},{lon}&status=actual");
    let text = client
        .get(&url)
        .header("Accept", "application/geo+json")
        .send()
        .await
        .context("NWS point alerts request failed")?
        .text()
        .await
        .context("NWS point alerts response read failed")?;
    parse_alerts_json(&text)
}

fn parse_alerts_json(json: &str) -> Result<Vec<Warning>> {
    // Use regex-based parsing to match wX's approach and avoid full serde complexity
    // (the NWS GeoJSON schema has some awkward nested structures)
    let html = json
        .replace(
            "\"geometry\": null,",
            "\"geometry\": null, \"coordinates\":[[]]}",
        )
        .replace('\n', " ");

    let re_url = Regex::new(r#""id":\s*"(https://api\.weather\.gov/alerts/urn[^"]*?)""#).unwrap();
    let re_area = Regex::new(r#""areaDesc":\s*"([^"]*?)""#).unwrap();
    let re_eff = Regex::new(r#""effective":\s*"([^"]*?)""#).unwrap();
    let re_exp = Regex::new(r#""expires":\s*"([^"]*?)""#).unwrap();
    let re_event = Regex::new(r#""event":\s*"([^"]*?)""#).unwrap();
    let re_sender = Regex::new(r#""senderName":\s*"([^"]*?)""#).unwrap();
    let re_vtec    = Regex::new(r"([A-Z0]\.[A-Z]{3}\.[A-Z]{4}\.[A-Z]{2}\.[A-Z]\.[0-9]{4}\.[0-9]{6}T[0-9]{4}Z-[0-9]{6}T[0-9]{4}Z)").unwrap();

    let compact = html.replace(' ', "");
    let re_poly = Regex::new(r#""coordinates":\[\[(.*?)\]\]\}"#).unwrap();

    fn collect<'a>(re: &Regex, text: &'a str) -> Vec<&'a str> {
        re.captures_iter(text)
            .filter_map(|c| c.get(1).map(|m| m.as_str()))
            .collect()
    }

    let urls = collect(&re_url, &html);
    let areas = collect(&re_area, &html);
    let effs = collect(&re_eff, &html);
    let exps = collect(&re_exp, &html);
    let events = collect(&re_event, &html);
    let senders = collect(&re_sender, &html);
    let polys = collect(&re_poly, &compact);
    let vtecs = collect(&re_vtec, &html);

    let n = urls.len();
    let mut warnings = Vec::with_capacity(n);
    for i in 0..n {
        warnings.push(Warning::new(
            get(i, &urls),
            get(i, &areas),
            get(i, &effs),
            get(i, &exps),
            get(i, &events),
            get(i, &senders),
            get(i, &polys),
            get(i, &vtecs),
        ));
    }
    Ok(warnings)
}

fn get(i: usize, v: &[&str]) -> String {
    v.get(i).copied().unwrap_or("").to_string()
}

// ── SPC products ──────────────────────────────────────────────────────────────

const SPC_BASE: &str = "https://www.spc.noaa.gov";

/// SPC Convective Outlook days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlookDay {
    Day1,
    Day2,
    Day3,
    Day4,
    Day5,
    Day6,
    Day7,
    Day8,
}

impl OutlookDay {
    pub fn image_url(self, category: &str) -> String {
        // E.g. https://www.spc.noaa.gov/products/outlook/day1otlk_cat.gif
        match self {
            OutlookDay::Day1 => format!("{SPC_BASE}/products/outlook/day1otlk_{category}.gif"),
            OutlookDay::Day2 => format!("{SPC_BASE}/products/outlook/day2otlk_{category}.gif"),
            OutlookDay::Day3 => format!("{SPC_BASE}/products/outlook/day3otlk_{category}.gif"),
            d => {
                let n = d as usize + 1;
                format!("{SPC_BASE}/products/exper/day4-8/day{n}prob.gif")
            }
        }
    }
}

/// Fetch the SPC active watches page and return a list of watch numbers in effect.
pub async fn fetch_spc_watches(client: &Client) -> Result<String> {
    let url = format!("{SPC_BASE}/products/watch/");
    let html = client
        .get(&url)
        .send()
        .await
        .context("SPC watches request failed")?
        .text()
        .await?;
    Ok(html)
}

/// URL for an SPC MCD (Mesoscale Convective Discussion) image.
pub fn mcd_url(num: u32) -> String {
    format!("{SPC_BASE}/products/md/mcd{num:04}.png")
}

/// URL for the SPC surface analysis/meso analysis parameter image.
pub fn meso_image_url(param: &str) -> String {
    format!("{SPC_BASE}/exper/mesoanalysis/s/{param}/{param}.gif")
}

/// URL for SPC storm reports (today).
pub fn storm_reports_url() -> String {
    format!("{SPC_BASE}/climo/reports/today.gif")
}

/// URL for a specific SPC convective outlook image.
pub fn outlook_url(day: u8, outlook_type: &str) -> String {
    if day <= 2 {
        format!("{SPC_BASE}/products/outlook/day{day}otlk_{outlook_type}.gif")
    } else if day == 3 {
        format!("{SPC_BASE}/products/outlook/day3otlk_{outlook_type}.gif")
    } else {
        format!("{SPC_BASE}/products/exper/day4-8/day{day}prob.gif")
    }
}
