/*
 * meso-updraft — background caching daemon for Meso.
 *
 * Periodically wakes, reads the subscriptions file, fetches the latest frame
 * for each subscribed radar and satellite product into the shared cache, then
 * runs a cache purge to respect retention policies.
 *
 * Designed to run as a systemd user service. See data/meso-updraft.service.
 *
 * Wake interval and enabled flag are read from the Meso config on each cycle
 * so changes take effect without restarting the daemon.
 */

mod fetch;

use anyhow::Result;
use directories::ProjectDirs;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tracing::info;

use meso_data::cache::Cache;
use meso_data::updraft::load_subscriptions;

// ── Minimal config subset the daemon needs ────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
struct DaemonConfig {
    #[serde(default = "default_true")]
    updraft_enabled: bool,
    #[serde(default = "default_interval")]
    updraft_interval_secs: u64,
    #[serde(default = "default_cache_radar_hours")]
    cache_radar_hours: u32,
    #[serde(default = "default_cache_sat_hours")]
    cache_sat_hours: u32,
}

fn default_true() -> bool {
    true
}
fn default_interval() -> u64 {
    300
}
fn default_cache_radar_hours() -> u32 {
    24
}
fn default_cache_sat_hours() -> u32 {
    24
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            updraft_enabled: true,
            updraft_interval_secs: default_interval(),
            cache_radar_hours: default_cache_radar_hours(),
            cache_sat_hours: default_cache_sat_hours(),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "Meso").map(|p| p.config_dir().join("config.toml"))
}

fn load_daemon_config() -> DaemonConfig {
    let path = match config_path() {
        Some(p) => p,
        None => return DaemonConfig::default(),
    };
    if !path.exists() {
        return DaemonConfig::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_default(),
        Err(_) => DaemonConfig::default(),
    }
}

// ── Main loop ─────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "meso_updraft=info".parse().unwrap()),
        )
        .init();

    info!("meso-updraft starting");

    let client = Client::builder()
        .user_agent("meso-updraft/0.1")
        .timeout(Duration::from_secs(30))
        .build()?;

    loop {
        let cfg = load_daemon_config();

        if !cfg.updraft_enabled {
            info!(
                "updraft disabled in config; sleeping {} s",
                cfg.updraft_interval_secs
            );
            tokio::time::sleep(Duration::from_secs(cfg.updraft_interval_secs)).await;
            continue;
        }

        let subs = load_subscriptions();
        let n_radar = subs.radar.len();
        let n_sat = subs.satellite.len();

        info!(
            "updraft: wake cycle — {} radar, {} satellite subscriptions",
            n_radar, n_sat
        );

        fetch::run_fetch_cycle(&client, &subs.radar, &subs.satellite).await;

        // Purge stale cache entries based on configured retention
        let max_age =
            Duration::from_secs(cfg.cache_radar_hours.max(cfg.cache_sat_hours) as u64 * 3600);
        Cache::purge_old_global(max_age);
        info!(
            "updraft: cache purge complete (max_age={}h)",
            max_age.as_secs() / 3600
        );

        tokio::time::sleep(Duration::from_secs(cfg.updraft_interval_secs)).await;
    }
}
