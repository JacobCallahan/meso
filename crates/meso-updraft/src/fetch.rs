/*
 * Updraft fetch logic — one wake cycle.
 *
 * For each radar subscription: fetches the latest L3 product into the shared
 * radar/l3 cache using the same TTL the main app uses.
 *
 * For each satellite subscription: fetches the latest GOES image into the
 * shared goes cache.
 */

use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};

use meso_data::goes;
use meso_data::radar::{download::RadarDownloader, products::RadarProduct};
use meso_data::updraft::{RadarSubscription, SatSubscription};

pub async fn fetch_radar(client: &Client, sub: &RadarSubscription) -> Result<()> {
    let product = RadarProduct::from_code(&sub.product)
        .ok_or_else(|| anyhow::anyhow!("Unknown radar product: {}", sub.product))?;
    let dl = RadarDownloader::new(client.clone());
    if product.is_level2() {
        let (fname, base) = dl.latest_level2_filename(&sub.station).await?;
        let url = format!("{base}{fname}");
        let bytes = dl
            .fetch_level2_decompressed(&sub.station, &product, &url)
            .await?;
        info!(
            "updraft: cached radar {}/{} (decompressed, {} bytes, file={})",
            sub.station,
            sub.product,
            bytes.len(),
            fname
        );
    } else {
        let bytes = dl.fetch_level3(&sub.station, &product).await?;
        info!(
            "updraft: cached radar {}/{} ({} bytes)",
            sub.station,
            sub.product,
            bytes.len()
        );
    }
    Ok(())
}

pub async fn fetch_satellite(client: &Client, sub: &SatSubscription) -> Result<()> {
    let url = goes::image_url(&sub.sector, &sub.band);
    let bytes = goes::fetch_image(client, &url).await?;
    info!(
        "updraft: cached satellite {}/{} ({} bytes)",
        sub.sector,
        sub.band,
        bytes.len()
    );
    Ok(())
}

pub async fn run_fetch_cycle(
    client: &Client,
    radar_subs: &[RadarSubscription],
    sat_subs: &[SatSubscription],
) {
    for sub in radar_subs {
        if let Err(e) = fetch_radar(client, sub).await {
            warn!(
                "updraft: radar fetch failed for {}/{}: {e}",
                sub.station, sub.product
            );
        }
    }
    for sub in sat_subs {
        if let Err(e) = fetch_satellite(client, sub).await {
            warn!(
                "updraft: satellite fetch failed for {}/{}: {e}",
                sub.sector, sub.band
            );
        }
    }
}
