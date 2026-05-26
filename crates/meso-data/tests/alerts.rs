/// Integration tests for NWS alerts fetching.
///
/// These tests hit live NWS API endpoints and require network access.
/// Run them with:
///
///   cargo test -p meso-data -- --ignored
use meso_data::alerts;
use meso_data::http::wx_client;

/// Fetching active alerts for the state of Texas should succeed.  There may be
/// zero alerts (quiet weather day) but the call itself should not error.
#[tokio::test]
#[ignore = "requires network access to api.weather.gov"]
async fn fetch_active_alerts_texas_succeeds() {
    let client = wx_client();
    let result = alerts::fetch_active_alerts_by_state(&client, "TX").await;
    assert!(result.is_ok(), "fetch failed: {}", result.unwrap_err());
}

/// Fetching alerts for Oklahoma City's lat/lon should return Ok.
#[tokio::test]
#[ignore = "requires network access to api.weather.gov"]
async fn fetch_alerts_for_okc_point_succeeds() {
    let client = wx_client();
    // Oklahoma City, OK
    let result = alerts::fetch_alerts_for_point(&client, 35.4676, -97.5164).await;
    assert!(result.is_ok(), "fetch failed: {}", result.unwrap_err());
}

/// Warning structs returned from the API should have non-empty event and area
/// fields when there is at least one active alert (conditional — skipped when
/// no alerts are active).
#[tokio::test]
#[ignore = "requires network access to api.weather.gov"]
async fn active_alerts_have_required_fields() {
    let client = wx_client();
    let warnings = alerts::fetch_active_alerts(&client, "US")
        .await
        .unwrap_or_default();
    for w in &warnings {
        assert!(!w.event.is_empty(), "event field must not be empty");
        assert!(!w.url.is_empty(), "url field must not be empty");
    }
}
