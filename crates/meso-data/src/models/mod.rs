/*
 * Model data fetching — SPC SREF and NCEP MAG models.
 *
 * SREF images: https://www.spc.noaa.gov/exper/sref/gifs/latest/{PRODUCT_ID}f{HHH:03}.gif
 * NCEP images: https://mag.ncep.noaa.gov/data/{model}/{run}/...
 */

pub mod ncep;

use anyhow::{Context, Result};
use reqwest::Client;

use crate::cache::Cache;

// ── Product catalog ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct SrefProduct {
    pub id: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone)]
pub struct SrefCategory {
    pub name: &'static str,
    pub products: &'static [SrefProduct],
}

macro_rules! prods {
    ($(($id:expr, $lbl:expr)),* $(,)?) => {
        &[$(SrefProduct { id: $id, label: $lbl }),*]
    }
}

static SPC_GUIDANCE: &[SrefProduct] = prods![
    (
        "SREF_PROB_TRW_CALIBRATED_HRLY__",
        "3hr Calibrated Thunderstorm"
    ),
    ("SREF_03HR_SVR_PROBS__", "3hr Calibrated Severe Tstm"),
    (
        "SREF_03HR_SVR_PROBS_CONDITIONAL__",
        "3hr Calibrated Conditional Severe Tstm"
    ),
    (
        "SREF_PROB_TRW_CALIBRATED_12HR__",
        "12hr Calibrated Thunderstorm"
    ),
    ("SREF_12HR_SVR_PROBS__", "12hr Calibrated Severe Tstm"),
    ("SREF_24HR_SVR_PROBS__", "24hr Calibrated Severe Tstm"),
];

static OVERVIEW: &[SrefProduct] = prods![
    ("SREF_500MB-HGHT_VORT__", "500mb Height/Vorticity"),
    ("SREF_H5__", "500mb H/W/T/Isotach (Mean)"),
    ("SREF_PMSL_1000-500_THK_BLW_", "PMSL/Thickness/10m Wind"),
    ("SREF_PMSL_MEAN_SD_", "Sea Level Pressure (Mean/SD)"),
    ("SREF_H8__", "850mb H/W/T/Isotach (Mean)"),
    ("SREF_Mean_Temp_", "2m Temperature (Mean)"),
    ("SREF_meanpcp_pcvv_thck_omega_3hr_", "700-500 UVV / 3hr QPF"),
];

static MOISTURE: &[SrefProduct] = prods![
    ("SREF_2M_DWPT_F_", "2m Dewpoint (Mean)"),
    ("SREF_2M-DWPF_MEDIAN_MXMN__", "2m Dewpoint (Median/Max/Min)"),
    ("SREF_pwat_mean_", "Precipitable Water (Mean)"),
    ("SREF_prob_2mdewpt_60F__", "P(2m Dewpoint ≥60°F)"),
    ("SREF_prob_2mdewpt_65F__", "P(2m Dewpoint ≥65°F)"),
    ("SREF_prob_2mdewpt_70F__", "P(2m Dewpoint ≥70°F)"),
    ("SREF_prob_MLLCL_750__", "P(Mixed-Layer LCL ≤750m)"),
    ("SREF_prob_MLLCL_1000__", "P(Mixed-Layer LCL ≤1000m)"),
];

static INSTABILITY: &[SrefProduct] = prods![
    (
        "SREF_SFCCAPE_MEDIAN_MXMN__",
        "Surface CAPE (Median/Max/Min)"
    ),
    ("SREF_prob_sfccape_500__", "P(Surface CAPE ≥500 J/kg)"),
    ("SREF_prob_sfccape_1000__", "P(Surface CAPE ≥1000 J/kg)"),
    ("SREF_prob_sfccape_2000__", "P(Surface CAPE ≥2000 J/kg)"),
    ("SREF_mlcape_MEDIAN_MXMN__", "ML CAPE (Median/Max/Min)"),
    ("SREF_prob_mlcape_1000__", "P(ML CAPE ≥1000 J/kg)"),
    ("SREF_prob_mlcape_2000__", "P(ML CAPE ≥2000 J/kg)"),
    ("SREF_hicape_MEDIAN_MXMN__", "MU CAPE (Median/Max/Min)"),
    ("SREF_prob_hicape_500__", "P(MU CAPE ≥500 J/kg)"),
    ("SREF_prob_hicape_1000__", "P(MU CAPE ≥1000 J/kg)"),
    ("SREF_SFC_LI_", "Surface LI (Mean)"),
    ("SREF_prob_lift_0__", "P(Surface LI ≤0)"),
    ("SREF_prob_lift_4__", "P(Surface LI ≤-4)"),
];

static KINEMATIC: &[SrefProduct] = prods![
    (
        "SREF_0-6KMSHR_SSB_MEDIAN_MXMN__",
        "0-6km Bulk Shear (Median/Max/Min)"
    ),
    ("SREF_prob_10m_to_6km_shear_30kt__", "P(0-6km Shear ≥30kts)"),
    ("SREF_prob_10m_to_6km_shear_40kt__", "P(0-6km Shear ≥40kts)"),
    (
        "SREF_ESHR_SSB_MEDIAN_MXMN__",
        "Effective Bulk Shear (Median/Max/Min)"
    ),
    ("SREF_prob_ESHR_30kt__", "P(Effective Shear ≥30kts)"),
    ("SREF_prob_ESHR_40kt__", "P(Effective Shear ≥40kts)"),
    (
        "SREF_1KMHEL_SSB_MEDIAN_MXMN__",
        "0-1km SRH (Median/Max/Min)"
    ),
    (
        "SREF_3KMHEL_SSB_MEDIAN_MXMN__",
        "0-3km SRH (Median/Max/Min)"
    ),
    ("SREF_prob_SSB_1kmHel_50__", "P(0-1km SRH ≥50 m²/s²)"),
    ("SREF_prob_SSB_1kmHel_100__", "P(0-1km SRH ≥100 m²/s²)"),
    ("SREF_prob_SSB_1kmHel_150__", "P(0-1km SRH ≥150 m²/s²)"),
];

static SEVERE: &[SrefProduct] = prods![
    (
        "SREF_CB_MEDIAN_MXMN__",
        "CravenBrooks Sig Severe (Median/Max/Min)"
    ),
    ("SREF_prob_cbsigsvr_10000__", "P(CravenBrooks ≥10000)"),
    ("SREF_prob_cbsigsvr_20000__", "P(CravenBrooks ≥20000)"),
    (
        "SREF_SIGTOR_MEDIAN_MXMN__",
        "Sig Tornado Parameter (Median/Max/Min)"
    ),
    ("SREF_prob_sigtor_1__", "P(Sig Tornado ≥1)"),
    ("SREF_prob_sigtor_3__", "P(Sig Tornado ≥3)"),
    ("SREF_prob_combined_sigtor__", "P(Sig Tornado Ingredients)"),
    (
        "SREF_SCCP_MEDIAN_MXMN__",
        "Supercell Composite Param (Median/Max/Min)"
    ),
    (
        "SREF_Spaghetti_SCCP_1__",
        "Supercell Composite ≥1 (Spaghetti)"
    ),
    (
        "SREF_Spaghetti_SCCP_3__",
        "Supercell Composite ≥3 (Spaghetti)"
    ),
];

static PRECIPITATION: &[SrefProduct] = prods![
    ("SREF_prob_totpcpn_0.01_3hr__", "P(3hr QPF ≥0.01 in)"),
    ("SREF_prob_totpcpn_0.25_3hr__", "P(3hr QPF ≥0.25 in)"),
    ("SREF_prob_totpcpn_0.50_3hr__", "P(3hr QPF ≥0.50 in)"),
    ("SREF_prob_totpcpn_0.01_6hr__", "P(6hr QPF ≥0.01 in)"),
    ("SREF_prob_totpcpn_0.25_6hr__", "P(6hr QPF ≥0.25 in)"),
    ("SREF_prob_totpcpn_0.50_6hr__", "P(6hr QPF ≥0.50 in)"),
    ("SREF_prob_totpcpn_0.01_12hr__", "P(12hr QPF ≥0.01 in)"),
    ("SREF_prob_totpcpn_0.25_12hr__", "P(12hr QPF ≥0.25 in)"),
    ("SREF_prob_totpcpn_1.00_12hr__", "P(12hr QPF ≥1.00 in)"),
    ("SREF_prob_totpcpn_0.01_24hr__", "P(24hr QPF ≥0.01 in)"),
    ("SREF_prob_totpcpn_1.00_24hr__", "P(24hr QPF ≥1.00 in)"),
    ("SREF_prob_totpcpn_2.00_24hr__", "P(24hr QPF ≥2.00 in)"),
];

static FIRE_WX: &[SrefProduct] = prods![
    (
        "SREF_FOSBERG_MEDIAN_MXMN__",
        "Fosberg Fire Wx Index (Median/Max/Min)"
    ),
    ("SREF_prob_FIRE_fosb_50__", "P(Fosberg ≥50)"),
    ("SREF_prob_FIRE_fosb_60__", "P(Fosberg ≥60)"),
    ("SREF_prob_FIRE_fosb_70__", "P(Fosberg ≥70)"),
    ("SREF_HAINES_MEDIAN_MXMN__", "Haines Index (Median/Max/Min)"),
    ("SREF_prob_FIRE_HAINES_5__", "P(Haines ≥5)"),
    ("SREF_COMBO_WSPD20_RH15__", "P(Wind ≥20kt & RH ≤15%)"),
    ("SREF_COMBO_WSPD20_RH20__", "P(Wind ≥20kt & RH ≤20%)"),
];

static AVIATION: &[SrefProduct] = prods![
    ("SREF_maxtop_max_", "Convective Cloud Top (Max)"),
    ("SREF_maxtop_", "Convective Cloud Top (Mean)"),
    ("SREF_maxtop_totalprob_low_", "P(CCT ≤31 KFT)"),
    ("SREF_maxtop_totalprob_mid_", "P(CCT 31–37 KFT)"),
    ("SREF_maxtop_totalprob_high_", "P(CCT >37 KFT)"),
];

static CATEGORIES: &[SrefCategory] = &[
    SrefCategory {
        name: "SPC Guidance",
        products: SPC_GUIDANCE,
    },
    SrefCategory {
        name: "Overview",
        products: OVERVIEW,
    },
    SrefCategory {
        name: "Moisture",
        products: MOISTURE,
    },
    SrefCategory {
        name: "Instability",
        products: INSTABILITY,
    },
    SrefCategory {
        name: "Kinematic",
        products: KINEMATIC,
    },
    SrefCategory {
        name: "Severe",
        products: SEVERE,
    },
    SrefCategory {
        name: "Precipitation",
        products: PRECIPITATION,
    },
    SrefCategory {
        name: "Fire Weather",
        products: FIRE_WX,
    },
    SrefCategory {
        name: "Aviation",
        products: AVIATION,
    },
];

/// Return all SREF product categories (hardcoded from SPC SREF HTML nav).
pub fn sref_categories() -> &'static [SrefCategory] {
    CATEGORIES
}

// ── URL helpers ───────────────────────────────────────────────────────────────

const SREF_BASE: &str = "https://www.spc.noaa.gov/exper/sref/gifs/latest/";

/// Build the URL for a single SREF frame.
/// `hour` should be a multiple of 3 in the range 0–87.
pub fn sref_frame_url(product_id: &str, hour: u16) -> String {
    format!("{SREF_BASE}{product_id}f{hour:03}.gif")
}

/// All forecast hours for SREF animation: 0, 3, 6, ..., 87 (30 frames).
pub fn sref_all_hours() -> Vec<u16> {
    (0u16..=87).step_by(3).collect()
}

// ── Data fetching ─────────────────────────────────────────────────────────────

/// Fetch a single SREF frame GIF, with 1-hour disk cache.
pub async fn fetch_sref_frame(client: &Client, product_id: &str, hour: u16) -> Result<Vec<u8>> {
    let url = sref_frame_url(product_id, hour);
    let cache = Cache::new("models/sref");
    if let Some(bytes) = cache.get(&url) {
        return Ok(bytes);
    }
    let bytes = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("SREF fetch failed: {url}"))?
        .bytes()
        .await
        .with_context(|| format!("SREF read failed: {url}"))?
        .to_vec();
    cache.put(&url, &bytes, std::time::Duration::from_secs(3600));
    Ok(bytes)
}

/// Fetch the initialization time of the current SREF run by parsing the SPC SREF page.
/// Returns UTC time of the model run (e.g. 2026-05-23 09:00 UTC for "2026052309z").
pub async fn fetch_sref_init_time(client: &Client) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    use regex::Regex;

    let url = "https://www.spc.noaa.gov/exper/sref/sref.php?run=latest&id=SREF_CB_MEDIAN_MXMN__";
    let text = client.get(url).send().await.ok()?.text().await.ok()?;

    // Find e.g. run=2026052309 in the 'current' table row
    let re = Regex::new(r"class='current'.*?run=(\d{10})").ok()?;
    let cap = re.captures(&text)?;
    let run_str = cap.get(1)?.as_str();

    let year: i32 = run_str[0..4].parse().ok()?;
    let month: u32 = run_str[4..6].parse().ok()?;
    let day: u32 = run_str[6..8].parse().ok()?;
    let hour: u32 = run_str[8..10].parse().ok()?;

    chrono::Utc
        .with_ymd_and_hms(year, month, day, hour, 0, 0)
        .single()
}
