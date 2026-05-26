/*
 * National weather products data module.
 *
 * Categories:
 *   WPC Surface Analysis  — WPC sfcobs + regional NAM analyses
 *   WPC Forecast Maps     — FMAP Day 1-7 and extended outlook GIFs
 *   WPC QPF               — quantitative precipitation forecasts
 *   NHC Tropical Outlook  — ATL / EPAC / CPAC 2-day and 7-day outlooks
 *   Upper Air Analysis    — OPC/ocean.weather.gov analysis charts
 */

use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;

use crate::cache::Cache;

// ── Product catalog ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NationalProduct {
    pub id: &'static str,
    pub label: &'static str,
    pub url: &'static str,
}

#[derive(Debug, Clone)]
pub struct NationalCategory {
    pub name: &'static str,
    pub products: &'static [NationalProduct],
}

macro_rules! prods {
    ($(($id:expr, $lbl:expr, $url:expr)),* $(,)?) => {
        &[$(NationalProduct { id: $id, label: $lbl, url: $url }),*]
    }
}

static WPC_SFC: &[NationalProduct] = prods![
    (
        "SFC_CONUS",
        "CONUS Surface Obs",
        "https://www.wpc.ncep.noaa.gov/sfc/sfcobs/large_latestsfc.gif"
    ),
    (
        "SFC_SW",
        "SW Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namswsfcwbg.gif"
    ),
    (
        "SFC_SC",
        "SC Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namscsfcwbg.gif"
    ),
    (
        "SFC_SE",
        "SE Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namsesfcwbg.gif"
    ),
    (
        "SFC_CW",
        "CW Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namcwsfcwbg.gif"
    ),
    (
        "SFC_CC",
        "C Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namccsfcwbg.gif"
    ),
    (
        "SFC_CE",
        "CE Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namcesfcwbg.gif"
    ),
    (
        "SFC_NW",
        "NW Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namnwsfcwbg.gif"
    ),
    (
        "SFC_NC",
        "NC Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namncsfcwbg.gif"
    ),
    (
        "SFC_NE",
        "NE Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namnesfcwbg.gif"
    ),
    (
        "SFC_AK",
        "AK Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namaksfcwbg.gif"
    ),
    (
        "SFC_AK2",
        "Gulf of AK Surface Analysis",
        "https://www.wpc.ncep.noaa.gov/sfc/namak2sfcwbg.gif"
    ),
    (
        "WPC_RADNAT",
        "WPC Analysis, Radar, Warnings",
        "https://www.wpc.ncep.noaa.gov/images/wwd/radnat/NATRAD_24.gif"
    ),
];

static WPC_FMAP: &[NationalProduct] = prods![
    (
        "FMAP",
        "National Forecast Day 1",
        "https://www.wpc.ncep.noaa.gov/noaa/noaad1.gif"
    ),
    (
        "FMAPD2",
        "National Forecast Day 2",
        "https://www.wpc.ncep.noaa.gov/noaa/noaad2.gif"
    ),
    (
        "FMAPD3",
        "National Forecast Day 3",
        "https://www.wpc.ncep.noaa.gov/noaa/noaad3.gif"
    ),
    (
        "FMAP12",
        "WPC Fronts/NDFD Weather Type 12hr",
        "https://www.wpc.ncep.noaa.gov/basicwx/92fwbg.gif"
    ),
    (
        "FMAP24",
        "WPC Fronts/NDFD Weather Type 24hr",
        "https://www.wpc.ncep.noaa.gov/basicwx/94fwbg.gif"
    ),
    (
        "FMAP36",
        "WPC Fronts/NDFD Weather Type 36hr",
        "https://www.wpc.ncep.noaa.gov/basicwx/96fwbg.gif"
    ),
    (
        "FMAP48",
        "WPC Fronts/NDFD Weather Type 48hr",
        "https://www.wpc.ncep.noaa.gov/basicwx/98fwbg.gif"
    ),
    (
        "FMAP72",
        "WPC Fronts 72hr",
        "https://www.wpc.ncep.noaa.gov/medr/display/wpcwx+frontsf072.gif"
    ),
    (
        "FMAP96",
        "WPC Fronts 96hr",
        "https://www.wpc.ncep.noaa.gov/medr/display/wpcwx+frontsf096.gif"
    ),
    (
        "FMAP120",
        "WPC Fronts 120hr",
        "https://www.wpc.ncep.noaa.gov/medr/display/wpcwx+frontsf120.gif"
    ),
    (
        "FMAP144",
        "WPC Fronts 144hr",
        "https://www.wpc.ncep.noaa.gov/medr/display/wpcwx+frontsf144.gif"
    ),
    (
        "FMAP168",
        "WPC Fronts 168hr",
        "https://www.wpc.ncep.noaa.gov/medr/display/wpcwx+frontsf168.gif"
    ),
    (
        "FMAP3D",
        "Forecast Map 3-Day",
        "https://www.wpc.ncep.noaa.gov/medr/9jhwbg_conus.gif"
    ),
    (
        "FMAP4D",
        "Forecast Map 4-Day",
        "https://www.wpc.ncep.noaa.gov/medr/9khwbg_conus.gif"
    ),
    (
        "FMAP5D",
        "Forecast Map 5-Day",
        "https://www.wpc.ncep.noaa.gov/medr/9lhwbg_conus.gif"
    ),
    (
        "FMAP6D",
        "Forecast Map 6-Day",
        "https://www.wpc.ncep.noaa.gov/medr/9mhwbg_conus.gif"
    ),
];

static WPC_QPF: &[NationalProduct] = prods![
    (
        "QPF1",
        "QPF Day 1",
        "https://www.wpc.ncep.noaa.gov/qpf/fill_94qwbg.gif"
    ),
    (
        "QPF2",
        "QPF Day 2",
        "https://www.wpc.ncep.noaa.gov/qpf/fill_98qwbg.gif"
    ),
    (
        "QPF3",
        "QPF Day 3",
        "https://www.wpc.ncep.noaa.gov/qpf/fill_99qwbg.gif"
    ),
    (
        "QPF1_2",
        "QPF Days 1-2",
        "https://www.wpc.ncep.noaa.gov/qpf/d12_fill.gif"
    ),
    (
        "QPF1_3",
        "QPF Days 1-3",
        "https://www.wpc.ncep.noaa.gov/qpf/d13_fill.gif"
    ),
    (
        "QPF4_5",
        "QPF Days 4-5",
        "https://www.wpc.ncep.noaa.gov/qpf/95ep48iwbg_fill.gif"
    ),
    (
        "QPF6_7",
        "QPF Days 6-7",
        "https://www.wpc.ncep.noaa.gov/qpf/97ep48iwbg_fill.gif"
    ),
    (
        "QPF1_5",
        "QPF Days 1-5",
        "https://www.wpc.ncep.noaa.gov/qpf/p120i.gif"
    ),
    (
        "QPF1_7",
        "QPF Days 1-7",
        "https://www.wpc.ncep.noaa.gov/qpf/p168i.gif"
    ),
];

static NHC: &[NationalProduct] = prods![
    (
        "NHC2ATL",
        "ATL 2-Day Tropical Outlook",
        "https://www.nhc.noaa.gov/xgtwo/two_atl_2d0.png"
    ),
    (
        "NHC5ATL",
        "ATL 7-Day Tropical Outlook",
        "https://www.nhc.noaa.gov/xgtwo/two_atl_7d0.png"
    ),
    (
        "NHC2EPAC",
        "EPAC 2-Day Tropical Outlook",
        "https://www.nhc.noaa.gov/xgtwo/two_pac_2d0.png"
    ),
    (
        "NHC5EPAC",
        "EPAC 7-Day Tropical Outlook",
        "https://www.nhc.noaa.gov/xgtwo/two_pac_7d0.png"
    ),
    (
        "NHC2CPAC",
        "CPAC 2-Day Tropical Outlook",
        "https://www.nhc.noaa.gov/xgtwo/two_cpac_2d0.png"
    ),
    (
        "NHC5CPAC",
        "CPAC 5-Day Tropical Outlook",
        "https://www.nhc.noaa.gov/xgtwo/two_cpac_5d0.png"
    ),
];

static UPPER_AIR: &[NationalProduct] = prods![
    (
        "UA_CONUS",
        "Continental USA",
        "https://ocean.weather.gov/UA/Conus.gif"
    ),
    (
        "UA_WEST_COAST",
        "West Coast",
        "https://ocean.weather.gov/UA/West_coast.gif"
    ),
    (
        "UA_USA_WEST",
        "USA West",
        "https://ocean.weather.gov/UA/USA_West.gif"
    ),
    (
        "UA_MIDWEST",
        "USA Mid West",
        "https://ocean.weather.gov/UA/USA_Mid_West.gif"
    ),
    (
        "UA_USA_EAST",
        "USA East",
        "https://ocean.weather.gov/UA/USA_East.gif"
    ),
    (
        "UA_EAST_COAST",
        "East Coast",
        "https://ocean.weather.gov/UA/East_coast.gif"
    ),
    (
        "UA_HAWAII",
        "Hawaii",
        "https://ocean.weather.gov/UA/Hawaii.gif"
    ),
    (
        "UA_ALASKA",
        "Alaska",
        "https://ocean.weather.gov/UA/Alaska.gif"
    ),
    (
        "UA_CANADA",
        "Canada",
        "https://ocean.weather.gov/UA/Canada.gif"
    ),
    (
        "UA_USA_SOUTH",
        "USA South",
        "https://ocean.weather.gov/UA/USA_South.gif"
    ),
    (
        "UA_MEXICO",
        "Gulf of Mexico",
        "https://ocean.weather.gov/UA/Mexico.gif"
    ),
    (
        "UA_OPC_PAC",
        "Pacific Ocean",
        "https://ocean.weather.gov/UA/OPC_PAC.gif"
    ),
    (
        "UA_PAC_TROP",
        "Pacific Tropical",
        "https://ocean.weather.gov/UA/Pac_Tropics.gif"
    ),
    (
        "UA_PAC_DIFAX",
        "Pacific Ocean Difax",
        "https://ocean.weather.gov/UA/Pac_Difax.gif"
    ),
    (
        "UA_OPC_ATL",
        "Atlantic Ocean",
        "https://ocean.weather.gov/UA/OPC_ATL.gif"
    ),
    (
        "UA_ATL_TROP",
        "Atlantic Tropical",
        "https://ocean.weather.gov/UA/Atl_Tropics.gif"
    ),
    (
        "UA_ATL_DIFAX",
        "Atlantic Ocean Difax",
        "https://ocean.weather.gov/UA/Atl_Difax.gif"
    ),
];

pub static CATEGORIES: &[NationalCategory] = &[
    NationalCategory {
        name: "WPC Surface Analysis",
        products: WPC_SFC,
    },
    NationalCategory {
        name: "WPC Forecast Maps",
        products: WPC_FMAP,
    },
    NationalCategory {
        name: "WPC QPF",
        products: WPC_QPF,
    },
    NationalCategory {
        name: "NHC Tropical",
        products: NHC,
    },
    NationalCategory {
        name: "Upper Air Analysis",
        products: UPPER_AIR,
    },
];

/// Look up a product's URL by ID.
pub fn product_url(id: &str) -> Option<&'static str> {
    for cat in CATEGORIES {
        for prod in cat.products {
            if prod.id == id {
                return Some(prod.url);
            }
        }
    }
    None
}

// ── Fetch ─────────────────────────────────────────────────────────────────────

pub async fn fetch_product(client: &Client, product_id: &str) -> Result<Vec<u8>> {
    let url = product_url(product_id)
        .ok_or_else(|| anyhow::anyhow!("unknown national product: {product_id}"))?;
    fetch_url(client, url).await
}

pub async fn fetch_url(client: &Client, url: &str) -> Result<Vec<u8>> {
    let cache = Cache::new("national");
    if let Some(b) = cache.get(url) {
        return Ok(b);
    }
    let bytes = client
        .get(url)
        .send()
        .await
        .context("national product fetch failed")?
        .bytes()
        .await
        .context("national product body read")?;
    cache.put(url, &bytes, Duration::from_secs(15 * 60));
    Ok(bytes.to_vec())
}
