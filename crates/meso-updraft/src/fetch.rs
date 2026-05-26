/*
 * Updraft fetch logic — one wake cycle.
 *
 * For each radar subscription: fetches all animation frames into the shared
 * cache using the same keys and TTLs the main app uses, so that triggering
 * animation in the UI hits the cache instead of re-fetching.
 *
 * For each satellite subscription: fetches all animation frame images into the
 * shared goes cache.
 */

use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};

use meso_data::goes;
use meso_data::radar::{download::RadarDownloader, products::RadarProduct};
use meso_data::updraft::{RadarSubscription, SatSubscription};

pub async fn fetch_radar(
    client: &Client,
    sub: &RadarSubscription,
    frame_count: usize,
) -> Result<()> {
    let product = RadarProduct::from_code(&sub.product)
        .ok_or_else(|| anyhow::anyhow!("Unknown radar product: {}", sub.product))?;
    let dl = RadarDownloader::new(client.clone());

    if product.is_level2() {
        let base = RadarDownloader::level2_dir_url(&sub.station);
        let fnames = dl
            .level2_filenames_for_animation(&sub.station, frame_count)
            .await?;
        let mut cached = 0usize;
        for fname in &fnames {
            let url = format!("{base}{fname}");
            match dl
                .fetch_level2_decompressed(&sub.station, &product, &url)
                .await
            {
                Ok(bytes) => {
                    cached += 1;
                    info!(
                        "updraft: cached L2 frame {}/{} {} ({} bytes)",
                        sub.station,
                        sub.product,
                        fname,
                        bytes.len()
                    );
                }
                Err(e) => warn!(
                    "updraft: L2 frame fetch failed {}/{} {}: {e}",
                    sub.station, sub.product, fname
                ),
            }
        }
        info!(
            "updraft: L2 {}/{} — {}/{} frames cached",
            sub.station,
            sub.product,
            cached,
            fnames.len()
        );
    } else {
        let fnames = dl
            .level3_filenames_for_animation(&sub.station, &product, frame_count)
            .await?;
        let mut cached = 0usize;
        for fname in &fnames {
            let url = match RadarDownloader::level3_file_url(&sub.station, &product, fname) {
                Some(u) => u,
                None => continue,
            };
            match dl.fetch_bytes(&url).await {
                Ok(bytes) => {
                    cached += 1;
                    info!(
                        "updraft: cached L3 frame {}/{} {} ({} bytes)",
                        sub.station,
                        sub.product,
                        fname,
                        bytes.len()
                    );
                }
                Err(e) => warn!(
                    "updraft: L3 frame fetch failed {}/{} {}: {e}",
                    sub.station, sub.product, fname
                ),
            }
        }
        info!(
            "updraft: L3 {}/{} — {}/{} frames cached",
            sub.station,
            sub.product,
            cached,
            fnames.len()
        );
    }
    Ok(())
}

pub async fn fetch_satellite(
    client: &Client,
    sub: &SatSubscription,
    frame_count: usize,
) -> Result<()> {
    let urls = goes::animation_urls(client, &sub.sector, &sub.band, frame_count).await?;
    let total = urls.len();
    let mut cached = 0usize;
    for url in &urls {
        match goes::fetch_image(client, url).await {
            Ok(bytes) => {
                cached += 1;
                info!(
                    "updraft: cached satellite {}/{} frame ({} bytes)",
                    sub.sector,
                    sub.band,
                    bytes.len()
                );
            }
            Err(e) => warn!(
                "updraft: satellite frame fetch failed {}/{}: {e}",
                sub.sector, sub.band
            ),
        }
    }
    info!(
        "updraft: satellite {}/{} — {}/{} frames cached",
        sub.sector, sub.band, cached, total
    );
    Ok(())
}

pub async fn run_fetch_cycle(
    client: &Client,
    radar_subs: &[RadarSubscription],
    sat_subs: &[SatSubscription],
    radar_frame_count: usize,
    sat_frame_count: usize,
) {
    for sub in radar_subs {
        if let Err(e) = fetch_radar(client, sub, radar_frame_count).await {
            warn!(
                "updraft: radar fetch failed for {}/{}: {e}",
                sub.station, sub.product
            );
        }
    }
    for sub in sat_subs {
        if let Err(e) = fetch_satellite(client, sub, sat_frame_count).await {
            warn!(
                "updraft: satellite fetch failed for {}/{}: {e}",
                sub.sector, sub.band
            );
        }
    }
}
