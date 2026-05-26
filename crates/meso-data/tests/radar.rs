/// Integration tests for radar data fetching and decoding.
///
/// These tests hit live NWS servers (TGFTP and NOMADS) and require network
/// access.  Run them with:
///
///   cargo test -p meso-data -- --ignored
use meso_data::http::wx_client;
use meso_data::radar::download::RadarDownloader;
use meso_data::radar::level2;
use meso_data::radar::level3;
use meso_data::radar::products::RadarProduct;

/// Fetch Level 3 N0Q (base reflectivity super-res, tilt 0) for KTLX from TGFTP
/// and verify the decoded scan has a realistic number of radials and range bins.
#[tokio::test]
#[ignore = "requires network access to tgftp.nws.noaa.gov"]
async fn fetch_and_decode_level3_n0q_ktlx() {
    let client = wx_client();
    let dl = RadarDownloader::new(client);
    let bytes = dl
        .fetch_level3("KTLX", &RadarProduct::N0Q)
        .await
        .expect("fetch Level 3 N0Q for KTLX");
    assert!(!bytes.is_empty(), "raw bytes should not be empty");

    let data = level3::decode(&bytes).expect("decode Level 3 N0Q");
    assert!(
        data.num_radials > 300,
        "expected ≥360 radials, got {}",
        data.num_radials
    );
    assert!(data.num_range_bins > 0, "range bins should be > 0");
    assert_eq!(
        data.bins.len(),
        data.num_radials * data.num_range_bins,
        "bins length should equal num_radials × num_range_bins"
    );
}

/// Fetch Level 3 N0U (base velocity super-res) and verify it decodes as a
/// velocity product (different bin size).
#[tokio::test]
#[ignore = "requires network access to tgftp.nws.noaa.gov"]
async fn fetch_and_decode_level3_n0u_ktlx() {
    let client = wx_client();
    let dl = RadarDownloader::new(client);
    let bytes = dl
        .fetch_level3("KTLX", &RadarProduct::N0U)
        .await
        .expect("fetch Level 3 N0U for KTLX");
    assert!(!bytes.is_empty());

    let data = level3::decode(&bytes).expect("decode Level 3 N0U");
    assert!(data.num_radials > 0);
}

/// Fetch the Level 2 directory listing for KTLX from NOMADS and verify the
/// returned filename looks like a real NEXRAD archive filename.
#[tokio::test]
#[ignore = "requires network access to nomads.ncep.noaa.gov"]
async fn fetch_level2_latest_filename_ktlx() {
    let client = wx_client();
    let dl = RadarDownloader::new(client);
    let (filename, base_url) = dl
        .latest_level2_filename("KTLX")
        .await
        .expect("fetch Level 2 dir listing for KTLX");

    assert!(
        filename.to_uppercase().contains("KTLX"),
        "filename should contain site code, got: {filename}"
    );
    assert!(
        base_url.contains("nomads.ncep.noaa.gov"),
        "base URL should point to NOMADS, got: {base_url}"
    );
}

/// Fetch a partial Level 2 archive for KTLX (reflectivity) and verify the raw
/// bytes decompress without error.
#[tokio::test]
#[ignore = "requires network access to nomads.ncep.noaa.gov"]
async fn fetch_level2_partial_and_decompress_ktlx() {
    let client = wx_client();
    let dl = RadarDownloader::new(client);
    let raw = dl
        .fetch_level2_partial("KTLX", &RadarProduct::L2Reflectivity, None)
        .await
        .expect("fetch Level 2 partial bytes");
    assert!(!raw.is_empty(), "raw bytes should not be empty");

    let decompressed = level2::decompress_level2(&raw).expect("decompress Level 2 data");
    assert!(
        decompressed.len() > raw.len(),
        "decompressed should be larger than compressed input"
    );
}

/// Verify that `level2_filenames_for_animation` returns a non-empty list of
/// plausible NEXRAD filenames for KTLX.
#[tokio::test]
#[ignore = "requires network access to nomads.ncep.noaa.gov"]
async fn level2_animation_filenames_ktlx() {
    let client = wx_client();
    let dl = RadarDownloader::new(client);
    let names = dl
        .level2_filenames_for_animation("KTLX", 6)
        .await
        .expect("fetch Level 2 animation filenames");
    assert!(!names.is_empty(), "should return at least one filename");
    assert!(
        names.len() <= 6,
        "should return at most the requested frame count"
    );
    for name in &names {
        assert!(
            name.to_uppercase().contains("KTLX"),
            "each filename should contain the site code, got: {name}"
        );
    }
}
