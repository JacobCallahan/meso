/*
 * NWS text product fetching via api.weather.gov.
 *
 * Supports AFD (Area Forecast Discussion), HWO (Hazardous Weather Outlook),
 * ZFP (Zone Forecast Product), and LSR (Local Storm Report).
 */

use crate::geo::sites;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

// ── Product types ─────────────────────────────────────────────────────────────

pub const PRODUCT_TYPES: &[(&str, &str)] = &[
    ("AFD", "Area Forecast Discussion"),
    ("HWO", "Hazardous Weather Outlook"),
    ("ZFP", "Zone Forecast Product"),
    ("LSR", "Local Storm Report"),
    ("NOW", "Short-Term Forecast"),
    ("PNS", "Public Information Statement"),
];

// ── Data structures ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TextProduct {
    pub product_code: String,
    pub wfo: String,
    pub issuance_time: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductListResponse {
    #[serde(rename = "@graph")]
    graph: Vec<ProductItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductItem {
    #[serde(rename = "@id")]
    id: String,
    #[allow(dead_code)]
    issuance_time: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductResponse {
    product_code: Option<String>,
    issuing_office: Option<String>,
    issuance_time: Option<String>,
    product_text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NwsPointsResponse {
    properties: NwsPointsProperties,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NwsPointsProperties {
    cwa: Option<String>,
}

// ── Fetch functions ───────────────────────────────────────────────────────────

/// Fetch the latest text product for a given type and WFO.
/// `wfo` should be the 3-letter office identifier (e.g. "OUN" not "KOUN").
pub async fn fetch_latest_text(
    client: &Client,
    product_type: &str,
    wfo: &str,
) -> Result<TextProduct> {
    let wfo_upper = wfo.trim().to_uppercase();
    let list_url = format!(
        "https://api.weather.gov/products/types/{}/locations/{}",
        product_type, wfo_upper
    );

    let list: ProductListResponse = client
        .get(&list_url)
        .send()
        .await
        .context("NWS product list fetch failed")?
        .json()
        .await
        .context("NWS product list parse failed")?;

    let first = list
        .graph
        .first()
        .ok_or_else(|| anyhow::anyhow!("No {} products found for {}", product_type, wfo_upper))?;

    let product: ProductResponse = client
        .get(&first.id)
        .send()
        .await
        .context("NWS product fetch failed")?
        .json()
        .await
        .context("NWS product parse failed")?;

    Ok(TextProduct {
        product_code: product
            .product_code
            .unwrap_or_else(|| product_type.to_string()),
        wfo: product.issuing_office.unwrap_or_else(|| wfo_upper.clone()),
        issuance_time: product.issuance_time.unwrap_or_default(),
        text: product
            .product_text
            .unwrap_or_else(|| "(no text)".to_string()),
    })
}

fn normalize_radar_site(site: &str) -> String {
    let s = site.trim().to_uppercase();
    if s.len() == 4 && s.starts_with('K') {
        s[1..].to_string()
    } else {
        s
    }
}

/// Resolve WFO from radar site using weather.gov points metadata (`properties.cwa`),
/// mirroring the approach used in wX.
pub async fn resolve_wfo_from_radar_site(client: &Client, site: &str) -> Result<String> {
    let site_norm = normalize_radar_site(site);
    let ll = sites::site_latlon(&site_norm)
        .ok_or_else(|| anyhow::anyhow!("Unknown radar site: {}", site_norm))?;
    let url = format!("https://api.weather.gov/points/{:.4},{:.4}", ll.lat, ll.lon);
    let resp: NwsPointsResponse = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("NWS points fetch failed: {url}"))?
        .json()
        .await
        .context("NWS points JSON parse failed")?;
    let cwa = resp
        .properties
        .cwa
        .unwrap_or_default()
        .trim()
        .to_uppercase();
    if cwa.is_empty() {
        anyhow::bail!("No CWA returned for radar site {}", site_norm);
    }
    Ok(cwa)
}

/// Derive a best-effort WFO code from radar site ID.
/// This is just a fallback default; for correctness use
/// `resolve_wfo_from_radar_site()` when possible.
pub fn wfo_from_radar_site(site: &str) -> String {
    normalize_radar_site(site)
}
