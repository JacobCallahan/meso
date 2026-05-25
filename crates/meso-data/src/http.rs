/*
 * Shared HTTP client factory.
 *
 * The NWS API (api.weather.gov) blocks requests that use the default reqwest
 * User-Agent string. All outbound requests should use `wx_client()` to ensure
 * the correct User-Agent is sent.
 */

use reqwest::Client;

/// Build a shared reqwest client with a Meso User-Agent.
///
/// The NWS API requires a descriptive User-Agent; using the raw reqwest default
/// results in "Access Denied" (403) responses from the Akamai CDN in front of
/// api.weather.gov.
pub fn wx_client() -> Client {
    Client::builder()
        .user_agent("Meso/0.1 (Rust desktop weather app)")
        .build()
        .unwrap_or_default()
}
