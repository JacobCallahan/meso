/*
 * NWS forecast and observations fetching.
 *
 * Fetches 7-day and hourly forecasts plus current conditions from api.weather.gov.
 * Also provides WPC surface analysis and RTMA image URLs.
 *
 * Ported from wX's misc/ package: UtilityWXOGLRadar, UtilityForecastIcon, etc.
 */

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

const NWS_API: &str = "https://api.weather.gov";
const WPC_BASE: &str = "https://www.wpc.ncep.noaa.gov";

// ── Location → Grid lookup ────────────────────────────────────────────────────

/// NWS grid metadata returned by the /points endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NwsPoint {
    pub grid_id: String,
    pub grid_x: u32,
    pub grid_y: u32,
    pub forecast: String,
    pub forecast_hourly: String,
    pub forecast_zone: String,
    pub observation_stations: String,
    pub relative_location: Option<RelativeLocation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelativeLocation {
    pub properties: RelativeLocationProps,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelativeLocationProps {
    pub city: String,
    pub state: String,
}

/// Resolve a lat/lon to NWS grid metadata.
pub async fn resolve_point(client: &Client, lat: f64, lon: f64) -> Result<NwsPoint> {
    let url = format!("{NWS_API}/points/{lat:.4},{lon:.4}");
    let resp: serde_json::Value = client
        .get(&url)
        .header("Accept", "application/geo+json")
        .send()
        .await
        .context("NWS /points request failed")?
        .json()
        .await
        .context("NWS /points JSON parse failed")?;

    let props = resp["properties"].clone();
    serde_json::from_value(props).context("NWS point properties parse failed")
}

// ── 7-day forecast ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ForecastPeriod {
    pub number: u32,
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    pub is_daytime: bool,
    pub temperature: i32,
    pub temperature_unit: String,
    pub wind_speed: String,
    pub wind_direction: String,
    pub icon: String,
    pub short_forecast: String,
    pub detailed_forecast: String,
    pub probability_of_precipitation: Option<i32>,
}

/// Fetch the 7-day forecast for a previously resolved grid.
pub async fn fetch_forecast(client: &Client, forecast_url: &str) -> Result<Vec<ForecastPeriod>> {
    let resp: serde_json::Value = client
        .get(forecast_url)
        .header("Accept", "application/geo+json")
        .send()
        .await
        .context("NWS forecast request failed")?
        .json()
        .await
        .context("NWS forecast JSON parse failed")?;

    let periods = resp["properties"]["periods"]
        .as_array()
        .context("No periods in forecast response")?;

    let mut result = Vec::with_capacity(periods.len());
    for p in periods {
        result.push(ForecastPeriod {
            number: p["number"].as_u64().unwrap_or(0) as u32,
            name: p["name"].as_str().unwrap_or("").to_string(),
            start_time: p["startTime"].as_str().unwrap_or("").to_string(),
            end_time: p["endTime"].as_str().unwrap_or("").to_string(),
            is_daytime: p["isDaytime"].as_bool().unwrap_or(true),
            temperature: p["temperature"].as_i64().unwrap_or(0) as i32,
            temperature_unit: p["temperatureUnit"].as_str().unwrap_or("F").to_string(),
            wind_speed: p["windSpeed"].as_str().unwrap_or("").to_string(),
            wind_direction: p["windDirection"].as_str().unwrap_or("").to_string(),
            icon: p["icon"].as_str().unwrap_or("").to_string(),
            short_forecast: p["shortForecast"].as_str().unwrap_or("").to_string(),
            detailed_forecast: p["detailedForecast"].as_str().unwrap_or("").to_string(),
            probability_of_precipitation: p["probabilityOfPrecipitation"]["value"]
                .as_i64()
                .map(|v| v as i32),
        });
    }
    Ok(result)
}

// ── Current observations ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CurrentConditions {
    pub station_id: String,
    pub station_name: String,
    pub timestamp: String,
    pub text_description: String,
    pub temperature_c: Option<f64>,
    pub dewpoint_c: Option<f64>,
    pub wind_direction: Option<f64>,
    pub wind_speed_ms: Option<f64>,
    pub wind_gust_ms: Option<f64>,
    pub barometric_pressure_pa: Option<f64>,
    pub sea_level_pressure_pa: Option<f64>,
    pub visibility_m: Option<f64>,
    pub relative_humidity: Option<f64>,
    pub wind_chill_c: Option<f64>,
    pub heat_index_c: Option<f64>,
    pub raw_message: String,
}

impl CurrentConditions {
    /// Temperature in Fahrenheit.
    pub fn temperature_f(&self) -> Option<f64> {
        self.temperature_c.map(|c| c * 9.0 / 5.0 + 32.0)
    }

    /// Dewpoint in Fahrenheit.
    pub fn dewpoint_f(&self) -> Option<f64> {
        self.dewpoint_c.map(|c| c * 9.0 / 5.0 + 32.0)
    }

    /// Wind speed in knots.
    pub fn wind_speed_kts(&self) -> Option<f64> {
        self.wind_speed_ms.map(|ms| ms * 1.944)
    }
}

/// Fetch current observations from the nearest station.
pub async fn fetch_observations(client: &Client, stations_url: &str) -> Result<CurrentConditions> {
    // Get list of observation stations
    let resp: serde_json::Value = client
        .get(stations_url)
        .header("Accept", "application/geo+json")
        .send()
        .await
        .context("NWS stations request failed")?
        .json()
        .await?;

    let station_id = resp["features"][0]["properties"]["stationIdentifier"]
        .as_str()
        .unwrap_or("");
    let station_name = resp["features"][0]["properties"]["name"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let obs_url = format!("{NWS_API}/stations/{station_id}/observations/latest");
    let obs: serde_json::Value = client
        .get(&obs_url)
        .header("Accept", "application/geo+json")
        .send()
        .await
        .context("NWS observations request failed")?
        .json()
        .await?;

    let props = &obs["properties"];
    Ok(CurrentConditions {
        station_id: station_id.to_string(),
        station_name,
        timestamp: props["timestamp"].as_str().unwrap_or("").to_string(),
        text_description: props["textDescription"].as_str().unwrap_or("").to_string(),
        temperature_c: props["temperature"]["value"].as_f64(),
        dewpoint_c: props["dewpoint"]["value"].as_f64(),
        wind_direction: props["windDirection"]["value"].as_f64(),
        wind_speed_ms: props["windSpeed"]["value"].as_f64(),
        wind_gust_ms: props["windGust"]["value"].as_f64(),
        barometric_pressure_pa: props["barometricPressure"]["value"].as_f64(),
        sea_level_pressure_pa: props["seaLevelPressure"]["value"].as_f64(),
        visibility_m: props["visibility"]["value"].as_f64(),
        relative_humidity: props["relativeHumidity"]["value"].as_f64(),
        wind_chill_c: props["windChill"]["value"].as_f64(),
        heat_index_c: props["heatIndex"]["value"].as_f64(),
        raw_message: props["rawMessage"].as_str().unwrap_or("").to_string(),
    })
}

// ── Hourly forecast ───────────────────────────────────────────────────────────

/// A single hour's forecast period (compact — no detailedForecast).
#[derive(Debug, Clone)]
pub struct HourlyPeriod {
    pub start_time: String,
    pub temperature: i32,
    pub temperature_unit: String,
    pub wind_speed: String,
    pub wind_direction: String,
    pub short_forecast: String,
    pub probability_of_precipitation: Option<i32>,
    pub dewpoint_c: Option<f64>,
    pub relative_humidity: Option<i32>,
}

impl HourlyPeriod {
    /// Abbreviated hour label, e.g. "2 PM" or "14:00".
    pub fn hour_label(&self) -> String {
        // startTime is like "2024-01-15T14:00:00-05:00"
        if let Some(t) = self.start_time.get(11..16) {
            // parse HH:MM and format as 12h
            if let (Ok(h), Ok(m)) = (t[..2].parse::<u8>(), t[3..5].parse::<u8>()) {
                let suffix = if h < 12 { "AM" } else { "PM" };
                let h12 = if h == 0 {
                    12
                } else if h > 12 {
                    h - 12
                } else {
                    h
                };
                if m == 0 {
                    return format!("{h12} {suffix}");
                } else {
                    return format!("{h12}:{m:02} {suffix}");
                }
            }
        }
        self.start_time.get(11..16).unwrap_or("??:??").to_string()
    }

    /// Day label for grouping, e.g. "Mon Jan 15".
    pub fn day_label(&self) -> String {
        // startTime: "2024-01-15T14:00:00-05:00"
        if let Some(date) = self.start_time.get(0..10) {
            // parse year/month/day
            let parts: Vec<&str> = date.split('-').collect();
            if parts.len() == 3 {
                let month_names = [
                    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov",
                    "Dec",
                ];
                if let (Ok(m), Ok(d)) = (parts[1].parse::<usize>(), parts[2].parse::<u32>()) {
                    if (1..=12).contains(&m) {
                        return format!("{} {}", month_names[m - 1], d);
                    }
                }
            }
        }
        self.start_time.get(0..10).unwrap_or("").to_string()
    }
}

/// Fetch the hourly forecast (next ~156 hours) for a previously resolved grid.
pub async fn fetch_hourly_forecast(
    client: &Client,
    forecast_hourly_url: &str,
) -> Result<Vec<HourlyPeriod>> {
    let resp: serde_json::Value = client
        .get(forecast_hourly_url)
        .header("Accept", "application/geo+json")
        .send()
        .await
        .context("NWS hourly forecast request failed")?
        .json()
        .await
        .context("NWS hourly forecast JSON parse failed")?;

    let periods = resp["properties"]["periods"]
        .as_array()
        .context("No periods in hourly forecast response")?;

    let mut result = Vec::with_capacity(periods.len());
    for p in periods {
        result.push(HourlyPeriod {
            start_time: p["startTime"].as_str().unwrap_or("").to_string(),
            temperature: p["temperature"].as_i64().unwrap_or(0) as i32,
            temperature_unit: p["temperatureUnit"].as_str().unwrap_or("F").to_string(),
            wind_speed: p["windSpeed"].as_str().unwrap_or("").to_string(),
            wind_direction: p["windDirection"].as_str().unwrap_or("").to_string(),
            short_forecast: p["shortForecast"].as_str().unwrap_or("").to_string(),
            probability_of_precipitation: p["probabilityOfPrecipitation"]["value"]
                .as_i64()
                .map(|v| v as i32),
            dewpoint_c: p["dewpoint"]["value"].as_f64(),
            relative_humidity: p["relativeHumidity"]["value"].as_i64().map(|v| v as i32),
        });
    }
    Ok(result)
}

// ── WPC / RTMA image URLs ─────────────────────────────────────────────────────

/// WPC surface analysis image URL.
pub fn wpc_surface_analysis_url() -> String {
    format!("{WPC_BASE}/images/noaa_logo_and_ras.gif")
}

/// WPC latest surface analysis URL (PNG).
pub fn wpc_analysis_latest() -> String {
    format!("{WPC_BASE}/sfc/sfccomps.gif")
}

/// RTMA temperature analysis URL for CONUS.
pub fn rtma_temp_url() -> String {
    "https://mag.ncep.noaa.gov/data/rtma/rtma_conus/rtma_conus_2dtemp_combo.gif".to_string()
}

/// RTMA dewpoint analysis URL.
pub fn rtma_dewpoint_url() -> String {
    "https://mag.ncep.noaa.gov/data/rtma/rtma_conus/rtma_conus_2ddewp_combo.gif".to_string()
}

/// RTMA wind speed analysis URL.
pub fn rtma_wind_url() -> String {
    "https://mag.ncep.noaa.gov/data/rtma/rtma_conus/rtma_conus_2dwind_combo.gif".to_string()
}
