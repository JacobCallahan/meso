/*
 * NEXRAD Level 3 Storm Tracking Information (STI) product decoder.
 *
 * URL: https://tgftp.nws.noaa.gov/SL.us008001/DF.of/DC.radar/DS.58sti/SI.{site}/sn.last
 *
 * The file is a binary+text hybrid. We parse the text sections with regex,
 * mirroring wX's CanvasStormInfo approach.
 */

use anyhow::{Context, Result};
use regex::Regex;

use crate::geo::sites::rid_prefix;

/// A single storm cell with its current position and motion vector.
#[derive(Debug, Clone)]
pub struct StormCell {
    pub id: String,
    pub lat: f64,
    pub lon: f64,
    pub bearing_deg: f64,
    pub speed_kt: f64,
}

const TGFTP: &str = "https://tgftp.nws.noaa.gov";

pub async fn fetch_storm_tracks(
    client: &reqwest::Client,
    site: &str,
    site_lat: f64,
    site_lon: f64,
) -> Result<Vec<StormCell>> {
    // TGFTP paths use a site prefix: "k" for CONUS, "p" for Hawaii/Pacific/Alaska, "t" for PR.
    let prefix = rid_prefix(site);
    let url = format!(
        "{TGFTP}/SL.us008001/DF.of/DC.radar/DS.58sti/SI.{}{}/sn.last",
        prefix,
        site.to_lowercase()
    );
    let bytes = client
        .get(&url)
        .send()
        .await
        .context("STI fetch failed")?
        .bytes()
        .await
        .context("STI read failed")?;

    let text = String::from_utf8_lossy(&bytes).into_owned();
    parse_sti_text(&text, site_lat, site_lon)
}

fn parse_sti_text(text: &str, site_lat: f64, site_lon: f64) -> Result<Vec<StormCell>> {
    let re_posn = Regex::new(r"AZ/RAN(.*?)(?:V\s|FCST|$)").unwrap();
    let re_mvt = Regex::new(r"(?:FCST )?MVT\s+(.*?)(?:V\s|ERR|$)").unwrap();
    let re_ids = Regex::new(r"STORM ID\s+((?:[A-Z0-9]{2}\s+)+)").unwrap();
    let re_num = Regex::new(r"\d+").unwrap();

    let flat = text.replace(['\r', '\n'], " ");

    let ids: Vec<String> = {
        let mut all = Vec::new();
        for cap in re_ids.captures_iter(&flat) {
            let row = &cap[1];
            let words: Vec<&str> = row.split_whitespace().collect();
            all.extend(words.iter().map(|s| s.to_string()));
        }
        all
    };

    let posn_nums: Vec<u32> = {
        let mut all = Vec::new();
        for cap in re_posn.captures_iter(&flat) {
            let s = cap[1].replace("NEW", "0 0").replace('/', " ");
            for m in re_num.find_iter(&s) {
                if let Ok(n) = m.as_str().parse::<u32>() {
                    all.push(n);
                }
            }
        }
        all
    };

    let mvt_nums: Vec<u32> = {
        let mut all = Vec::new();
        for cap in re_mvt.captures_iter(&flat) {
            let s = cap[1].replace("NEW", "0 0").replace('/', " ");
            for m in re_num.find_iter(&s) {
                if let Ok(n) = m.as_str().parse::<u32>() {
                    all.push(n);
                }
            }
        }
        all
    };

    if posn_nums.len() < 2 || posn_nums.len() != mvt_nums.len() {
        return Ok(Vec::new());
    }

    let mut cells = Vec::new();
    let step = posn_nums.len().min(mvt_nums.len()) / 2 * 2;
    for i in (0..step).step_by(2) {
        let az_deg = posn_nums[i] as f64;
        let range_nm = posn_nums[i + 1] as f64;
        let mot_deg = mvt_nums[i] as f64;
        let mot_kt = mvt_nums[i + 1] as f64;

        let range_m = range_nm * 1852.0;
        let (cell_lat, cell_lon) = bearing_point(site_lat, site_lon, az_deg, range_m);

        let id = ids
            .get(i / 2)
            .cloned()
            .unwrap_or_else(|| format!("?{}", i / 2));

        cells.push(StormCell {
            id,
            lat: cell_lat,
            lon: cell_lon,
            bearing_deg: mot_deg,
            speed_kt: mot_kt,
        });
    }

    Ok(cells)
}

/// Compute destination lat/lon from a starting point, bearing (degrees), and distance (meters).
/// Uses the spherical Earth approximation (Haversine inverse).
pub fn bearing_point(lat: f64, lon: f64, bearing_deg: f64, distance_m: f64) -> (f64, f64) {
    let r = 6_371_000.0_f64;
    let lat_r = lat.to_radians();
    let lon_r = lon.to_radians();
    let brng = bearing_deg.to_radians();
    let d_r = distance_m / r;

    let lat2 = (lat_r.sin() * d_r.cos() + lat_r.cos() * d_r.sin() * brng.cos()).asin();
    let lon2 =
        lon_r + (brng.sin() * d_r.sin() * lat_r.cos()).atan2(d_r.cos() - lat_r.sin() * lat2.sin());
    (lat2.to_degrees(), lon2.to_degrees())
}
