/*
 * SPC (Storm Prediction Center) data fetching.
 *
 * Provides outlook images (Day 1–3) and today's storm reports (tornado, hail, wind).
 */

use anyhow::{Context, Result};
use reqwest::Client;

// ── Outlook images ────────────────────────────────────────────────────────────

/// Fetch the SPC convective outlook PNG for a given day (1, 2, or 3).
pub async fn fetch_outlook_image(client: &Client, day: u8) -> Result<Vec<u8>> {
    let day = day.clamp(1, 3);
    let url = format!("https://www.spc.noaa.gov/products/outlook/day{day}otlk.png");
    let bytes = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("SPC Day {day} outlook fetch failed"))?
        .bytes()
        .await
        .with_context(|| format!("SPC Day {day} outlook read failed"))?;
    Ok(bytes.to_vec())
}

// ── Storm reports ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ReportType {
    Tornado,
    Hail,
    Wind,
}

impl ReportType {
    pub fn label(&self) -> &'static str {
        match self {
            ReportType::Tornado => "Tornado",
            ReportType::Hail => "Hail",
            ReportType::Wind => "Wind",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StormReport {
    pub time: String,
    pub magnitude: String,
    pub location: String,
    pub county: String,
    pub state: String,
    pub lat: f64,
    pub lon: f64,
    pub comments: String,
    pub report_type: ReportType,
}

/// Fetch today's SPC storm reports (tornado + hail + wind combined).
pub async fn fetch_storm_reports(client: &Client) -> Result<Vec<StormReport>> {
    let mut reports = Vec::new();

    // Tornado reports
    let tor = fetch_reports_csv(
        client,
        "https://www.spc.noaa.gov/climo/reports/today_torn.csv",
        ReportType::Tornado,
    )
    .await
    .unwrap_or_default();
    reports.extend(tor);

    // Large hail reports
    let hail = fetch_reports_csv(
        client,
        "https://www.spc.noaa.gov/climo/reports/today_hail.csv",
        ReportType::Hail,
    )
    .await
    .unwrap_or_default();
    reports.extend(hail);

    // Damaging wind reports
    let wind = fetch_reports_csv(
        client,
        "https://www.spc.noaa.gov/climo/reports/today_wind.csv",
        ReportType::Wind,
    )
    .await
    .unwrap_or_default();
    reports.extend(wind);

    Ok(reports)
}

async fn fetch_reports_csv(
    client: &Client,
    url: &str,
    rtype: ReportType,
) -> Result<Vec<StormReport>> {
    let text = client
        .get(url)
        .send()
        .await
        .context("SPC storm report fetch failed")?
        .text()
        .await
        .context("SPC storm report read failed")?;

    parse_storm_reports_csv(&text, rtype)
}

/// Parse SPC storm report CSV.
/// Format: Time,F_Scale,Location,County,State,Lat,Lon,Comments
fn parse_storm_reports_csv(csv: &str, rtype: ReportType) -> Result<Vec<StormReport>> {
    let mut reports = Vec::new();
    let mut lines = csv.lines();
    // Skip header line
    let _ = lines.next();

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Handle quoted fields with commas inside
        let fields = split_csv_line(line);
        if fields.len() < 8 {
            continue;
        }

        let lat = fields[5].parse::<f64>().unwrap_or(0.0);
        let lon = fields[6].parse::<f64>().unwrap_or(0.0);
        if lat == 0.0 && lon == 0.0 {
            continue;
        }

        reports.push(StormReport {
            time: fields[0].trim().to_string(),
            magnitude: fields[1].trim().to_string(),
            location: fields[2].trim().to_string(),
            county: fields[3].trim().to_string(),
            state: fields[4].trim().to_string(),
            lat,
            lon,
            comments: fields[7].trim().to_string(),
            report_type: rtype.clone(),
        });
    }
    Ok(reports)
}

/// Naive CSV line splitter that handles double-quoted fields.
fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(current.clone());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    fields.push(current);
    fields
}
