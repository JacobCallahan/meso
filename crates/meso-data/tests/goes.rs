/// Integration tests for GOES satellite data.
///
/// These tests hit live NESDIS CDN endpoints and require network access.
/// Run them with:
///
///   cargo test -p meso-data -- --ignored
use meso_data::goes;
use meso_data::http::wx_client;

/// Build the GOES-East CONUS Band-2 (visible) image URL and verify it returns
/// an HTTP 200 response with non-empty image bytes.
#[tokio::test]
#[ignore = "requires network access to cdn.star.nesdis.noaa.gov"]
async fn goes_east_conus_band02_url_returns_200() {
    let client = wx_client();
    let url = goes::image_url("CONUS", "02");
    assert!(!url.is_empty(), "image URL must not be empty");
    assert!(
        url.contains("nesdis.noaa.gov"),
        "URL should point to NESDIS CDN, got: {url}"
    );

    let bytes = goes::fetch_image(&client, &url)
        .await
        .expect("fetch GOES CONUS Band-2 image");
    assert!(!bytes.is_empty(), "image bytes should not be empty");
}

/// Verify `find_sector` returns Some for known sector codes.
#[test]
fn find_sector_known_codes() {
    assert!(
        goes::find_sector("CONUS").is_some(),
        "CONUS should be a known sector"
    );
    assert!(
        goes::find_sector("FD").is_some(),
        "FD should be a known sector"
    );
}

/// Verify `find_sector` returns None for unknown codes.
#[test]
fn find_sector_unknown_code() {
    assert!(goes::find_sector("BOGUS_SECTOR_XYZ").is_none());
}

/// `nearest_sector` for a point inside the CONUS bounding box should return
/// a sector code that is not empty.
#[test]
fn nearest_sector_for_okc_is_non_empty() {
    use meso_data::geo::latlon::LatLon;
    let okc = LatLon::new(35.4676, -97.5164);
    let code = goes::nearest_sector(&okc);
    assert!(!code.is_empty(), "nearest sector code should not be empty");
}

/// Fetch GOES animation URLs for CONUS/Band-2 and verify at least one URL is
/// returned with the expected CDN hostname.
#[tokio::test]
#[ignore = "requires network access to www.star.nesdis.noaa.gov"]
async fn goes_conus_animation_urls_non_empty() {
    let client = wx_client();
    let urls = goes::animation_urls(&client, "CONUS", "02", 6)
        .await
        .expect("fetch GOES animation URLs");
    assert!(
        !urls.is_empty(),
        "should return at least one animation frame URL"
    );
    for url in &urls {
        assert!(
            url.contains("nesdis.noaa.gov"),
            "URL should point to NESDIS, got: {url}"
        );
    }
}
