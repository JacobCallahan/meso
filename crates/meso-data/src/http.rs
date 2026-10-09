/*
 * Shared HTTP client factory.
 *
 * The NWS API (api.weather.gov) blocks requests that use the default reqwest
 * User-Agent string. All outbound requests should use `wx_client()` to ensure
 * the correct User-Agent is sent.
 */

use reqwest::Client;
use std::sync::OnceLock;

static CLIENT: OnceLock<Client> = OnceLock::new();

/// Build or return a shared reqwest client with a Meso User-Agent.
///
/// The NWS API requires a descriptive User-Agent; using the raw reqwest default
/// results in "Access Denied" (403) responses from the Akamai CDN in front of
/// api.weather.gov.
///
/// Returns a cheap clone (internally `Arc`-wrapped) of a shared singleton client
/// that preserves connection pools, HTTP keep-alive, DNS caches, and TLS sessions.
pub fn wx_client() -> Client {
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent("Meso/0.1 (Rust desktop weather app)")
                .pool_idle_timeout(std::time::Duration::from_secs(90))
                .tcp_keepalive(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default()
        })
        .clone()
}
