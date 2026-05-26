/// Integration tests for NWS forecast fetching.
///
/// These tests hit live NWS API endpoints and require network access.
/// Run them with:
///
///   cargo test -p meso-data -- --ignored
use meso_data::forecast;
use meso_data::http::wx_client;

/// Resolve a known lat/lon (Oklahoma City) to NWS grid metadata and verify the
/// returned grid_id and coordinates are non-empty / reasonable.
#[tokio::test]
#[ignore = "requires network access to api.weather.gov"]
async fn resolve_point_okc() {
    let client = wx_client();
    let point = forecast::resolve_point(&client, 35.4676, -97.5164)
        .await
        .expect("resolve NWS point for OKC");

    assert!(!point.grid_id.is_empty(), "grid_id must not be empty");
    assert!(
        point.grid_x > 0 || point.grid_y > 0,
        "grid coordinates should be non-zero"
    );
    assert!(!point.forecast.is_empty(), "forecast URL must not be empty");
}

/// Resolve a point and then fetch the 7-day forecast.  At minimum we expect at
/// least one forecast period to come back.
#[tokio::test]
#[ignore = "requires network access to api.weather.gov"]
async fn fetch_7day_forecast_okc() {
    let client = wx_client();
    let point = forecast::resolve_point(&client, 35.4676, -97.5164)
        .await
        .expect("resolve NWS point");

    let periods = forecast::fetch_forecast(&client, &point.forecast)
        .await
        .expect("fetch 7-day forecast");

    assert!(
        !periods.is_empty(),
        "should have at least one forecast period"
    );
    let first = &periods[0];
    assert!(!first.name.is_empty(), "period name must not be empty");
}

/// Resolve a point close to a major city and confirm the relative_location
/// fields are populated.
#[tokio::test]
#[ignore = "requires network access to api.weather.gov"]
async fn resolve_point_has_relative_location() {
    let client = wx_client();
    // Norman, OK — home of NWS Storm Prediction Center
    let point = forecast::resolve_point(&client, 35.2226, -97.4395)
        .await
        .expect("resolve NWS point for Norman OK");

    if let Some(loc) = &point.relative_location {
        assert!(
            !loc.properties.city.is_empty(),
            "city name should not be empty"
        );
        assert!(
            !loc.properties.state.is_empty(),
            "state name should not be empty"
        );
    }
    // relative_location may be None for some grid points — that is acceptable.
}
