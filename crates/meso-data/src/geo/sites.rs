use crate::geo::latlon::LatLon;

/// NEXRAD WSR-88D and TDWR radar site list.
/// Ported from wX `RadarSites.kt`.
/// Format: (code, description, lat, lon)
pub static RADAR_SITES: &[(&str, &str, f64, f64)] = &[
    ("ABC", "ABC", 60.792, -161.876),
    ("ABR", "ABR", 45.456, -98.413),
    ("ABX", "ABX", 35.15, -106.824),
    ("ACG", "ACG", 56.853, -135.528),
    ("AEC", "AEC", 64.512, -165.293),
    ("AHG", "AHG", 60.726, -151.351),
    ("AIH", "AIH", 59.46, -146.303),
    ("AKC", "AKC", 58.68, -156.627),
    ("AKQ", "VA, Norfolk/Richmond", 36.984, -77.008),
    ("AMA", "AMA", 35.233, -101.709),
    ("AMX", "FL, Miami", 25.611, -80.413),
    ("APD", "APD", 65.035, -147.501),
    ("APX", "MI, Gaylord", 44.906, -84.72),
    ("ARX", "WI, La Crosse", 43.823, -91.191),
    ("ATX", "ATX", 48.195, -122.496),
    ("BBX", "BBX", 39.496, -121.632),
    ("BGM", "NY, Binghamton", 42.2, -75.985),
    ("BHX", "BHX", 40.499, -124.292),
    ("BIS", "BIS", 46.771, -100.76),
    ("BLX", "BLX", 45.854, -108.607),
    ("BMX", "AL, Birmingham", 33.171, -86.77),
    ("BOX", "MA, Boston", 41.956, -71.137),
    ("BRO", "BRO", 25.916, -97.419),
    ("BUF", "NY, Buffalo", 42.949, -78.737),
    ("BYX", "FL, Key West", 24.597, -81.703),
    ("CAE", "SC, Columbia", 33.949, -81.119),
    ("CBW", "ME, Loring AFB", 46.039, -67.806),
    ("CBX", "CBX", 43.49, -116.236),
    ("CCX", "PA, State College", 40.923, -78.004),
    ("CLE", "OH, Cleveland", 41.413, -81.86),
    ("CLX", "SC, Charleston", 32.655, -81.042),
    ("CRP", "CRP", 27.784, -97.511),
    ("CXX", "VT, Burlington", 44.511, -73.166),
    ("CYS", "CYS", 41.152, -104.806),
    ("DAX", "DAX", 38.501, -121.678),
    ("DDC", "DDC", 37.761, -99.969),
    ("DFX", "DFX", 29.273, -100.28),
    ("DGX", "MS, Brandon/Jackson", 32.28, -89.984),
    ("DIX", "PA, Philadelphia", 39.947, -74.411),
    ("DLH", "MN, Duluth", 46.837, -92.21),
    ("DMX", "IA, Des Moines", 41.731, -93.723),
    ("DOX", "DE, Dover AFB", 38.826, -75.44),
    ("DTX", "MI, Detroit/Pontiac", 42.7, -83.472),
    ("DVN", "IA, Davenport", 41.612, -90.581),
    ("DYX", "DYX", 32.538, -99.254),
    ("EAX", "MO, Kansas City", 38.81, -94.264),
    ("EMX", "EMX", 31.894, -110.63),
    ("ENX", "NY, Albany", 42.586, -74.064),
    ("EOX", "AL, Fort Rucker", 31.46, -85.459),
    ("EPZ", "EPZ", 31.873, -106.698),
    ("ESX", "ESX", 35.701, -114.891),
    ("EVX", "FL, Eglin AFB", 30.565, -85.922),
    ("EWX", "EWX", 29.704, -98.029),
    ("EYX", "EYX", 35.098, -117.561),
    ("FCX", "VA, Roanoke", 37.024, -80.274),
    ("FDR", "FDR", 34.362, -98.977),
    ("FDX", "FDX", 34.634, -103.619),
    ("FFC", "GA, Atlanta", 33.363, -84.566),
    ("FSD", "FSD", 43.588, -96.729),
    ("FSX", "FSX", 34.574, -111.198),
    ("FTG", "FTG", 39.786, -104.546),
    ("FWS", "FWS", 32.573, -97.303),
    ("GGW", "GGW", 48.206, -106.625),
    ("GJX", "GJX", 39.062, -108.214),
    ("GLD", "GLD", 39.367, -101.7),
    ("GRB", "WI, Green Bay", 44.499, -88.111),
    ("GRK", "GRK", 30.722, -97.383),
    ("GRR", "MI, Grand Rapids", 42.894, -85.545),
    ("GSP", "SC, Greer", 34.883, -82.22),
    ("GUA", "GUA", 13.456, -144.811),
    ("GWX", "MS, Columbus AFB", 33.897, -88.329),
    ("GYX", "ME, Portland", 43.891, -70.256),
    ("HDC", "LA, Hammond", 30.519, -90.407),
    ("HDX", "HDX", 33.077, -106.12),
    ("HGX", "HGX", 29.472, -95.079),
    ("HKI", "HKI", 21.894, -159.552),
    ("HKM", "HKM", 20.125, -155.778),
    ("HMO", "HMO", 21.133, -157.18),
    ("HNX", "HNX", 36.314, -119.632),
    ("HPX", "KY, Fort Campbell", 36.737, -87.285),
    ("HTX", "AL, Huntsville", 34.931, -86.084),
    ("HWA", "HWA", 19.095, -155.569),
    ("ICT", "ICT", 37.654, -97.443),
    ("ICX", "ICX", 37.591, -112.862),
    ("ILN", "OH, Wilmington", 39.42, -83.822),
    ("ILX", "IL, Lincoln", 40.15, -89.337),
    ("IND", "IN, Indianapolis", 39.708, -86.28),
    ("INX", "INX", 36.175, -95.564),
    ("IWA", "IWA", 33.289, -111.67),
    ("IWX", "IN, North Webster", 41.359, -85.7),
    ("JAX", "FL, Jacksonville", 30.485, -81.702),
    ("JGX", "GA, Robins AFB", 32.675, -83.351),
    ("JKL", "KY, Jackson", 37.591, -83.313),
    ("JUA", "PR, San Juan", 18.116, -66.078),
    ("LBB", "LBB", 33.654, -101.814),
    ("LCH", "LA, Lake Charles", 30.125, -93.216),
    ("LGX", "LGX", 47.116, -124.107),
    ("LNX", "LNX", 41.958, -100.576),
    ("LOT", "IL, Chicago", 41.604, -88.085),
    ("LRX", "LRX", 40.74, -116.803),
    ("LSX", "MO, St. Louis", 38.699, -90.683),
    ("LTX", "NC, Wilmington", 33.989, -78.429),
    ("LVX", "KY, Louisville", 37.975, -85.944),
    ("LWX", "VA, Sterling", 38.976, -77.487),
    ("LZK", "AR, Little Rock", 34.836, -92.262),
    ("MAF", "MAF", 31.943, -102.189),
    ("MAX", "MAX", 42.081, -122.717),
    ("MBX", "MBX", 48.393, -100.864),
    ("MHX", "NC, Morehead City", 34.776, -76.876),
    ("MKX", "WI, Milwaukee", 42.968, -88.551),
    ("MLB", "FL, Melbourne", 28.113, -80.654),
    ("MOB", "AL, Mobile", 30.679, -88.24),
    ("MPX", "MN, Minneapolis/St. Paul", 44.849, -93.565),
    ("MQT", "MI, Marquette", 46.531, -87.548),
    ("MRX", "TN, Knoxville/Tri Cities", 36.168, -83.402),
    ("MSX", "MSX", 47.041, -113.986),
    ("MTX", "MTX", 41.263, -112.448),
    ("MUX", "MUX", 37.155, -121.898),
    ("MVX", "MVX", 47.528, -97.325),
    ("MXX", "AL, Maxwell AFB", 32.537, -85.79),
    ("NKX", "NKX", 32.919, -117.041),
    ("NQA", "TN, Memphis", 35.345, -89.873),
    ("OAX", "OAX", 41.32, -96.367),
    ("OHX", "TN, Nashville", 36.247, -86.563),
    ("OKX", "NY, New York City", 40.865, -72.864),
    ("OTX", "OTX", 47.681, -117.626),
    ("PAH", "KY, Paducah", 37.068, -88.772),
    ("PBZ", "PA, Pittsburgh", 40.532, -80.218),
    ("PDT", "PDT", 45.691, -118.853),
    ("POE", "LA, Fort Polk", 31.155, -92.976),
    ("PUX", "PUX", 38.46, -104.181),
    ("RAX", "NC, Raleigh/Durham", 35.665, -78.49),
    ("RGX", "RGX", 39.754, -119.462),
    ("RIW", "RIW", 43.066, -108.477),
    ("RLX", "WV, Charleston", 38.311, -81.723),
    ("RTX", "RTX", 45.715, -122.965),
    ("SFX", "SFX", 43.106, -112.686),
    ("SGF", "MO, Springfield", 37.235, -93.4),
    ("SHV", "SHV", 32.451, -93.841),
    ("SJT", "SJT", 31.371, -100.492),
    ("SOX", "SOX", 33.818, -117.636),
    ("SRX", "AR, Fort Smith", 35.29, -94.362),
    ("TBW", "FL, Tampa", 27.705, -82.402),
    ("TDTW", "TDTW", 42.11111, -83.515),
    ("TFX", "TFX", 47.46, -111.385),
    ("TLH", "FL, Tallahassee", 30.398, -84.329),
    ("TLX", "TLX", 35.333, -97.278),
    ("TWX", "TWX", 38.997, -96.232),
    ("TYX", "NY, Montague", 43.756, -75.68),
    ("UDX", "UDX", 44.125, -102.83),
    ("UEX", "UEX", 40.321, -98.442),
    ("VAX", "GA, Moody AFB", 30.89, -83.002),
    ("VBX", "VBX", 34.839, -120.398),
    ("VNX", "VNX", 36.741, -98.128),
    ("VTX", "VTX", 34.412, -119.179),
    ("VWX", "IN, Evansville", 38.26, -87.724),
    ("YUX", "YUX", 32.495, -114.656),
];

/// Returns the [`LatLon`] for a given radar site code, or `None` if not found.
pub fn site_latlon(code: &str) -> Option<LatLon> {
    RADAR_SITES
        .iter()
        .find(|&&(c, _, _, _)| c == code)
        .map(|&(_, _, lat, lon)| LatLon::new(lat, lon))
}

/// Returns the description string for a site code.
pub fn site_name(code: &str) -> Option<&'static str> {
    RADAR_SITES
        .iter()
        .find(|&&(c, _, _, _)| c == code)
        .map(|&(_, name, _, _)| name)
}

/// Find the nearest NEXRAD/TDWR site code to a given location.
pub fn nearest_site(loc: &LatLon, include_tdwr: bool) -> &'static str {
    RADAR_SITES
        .iter()
        .filter(|&&(code, _, _, _)| include_tdwr || code.len() == 3)
        .min_by(|&&(_, _, la, lo), &&(_, _, lb, loo)| {
            let da = LatLon::new(la, lo).distance_km(loc);
            let db = LatLon::new(lb, loo).distance_km(loc);
            da.partial_cmp(&db).unwrap()
        })
        .map(|&(code, _, _, _)| code)
        .unwrap_or("TLX")
}

/// Returns true if the site is a TDWR (4-character code).
pub fn is_tdwr(code: &str) -> bool {
    code.len() == 4
}

/// Return the URL prefix character for a radar site (k, p, or t).
/// Used to construct TGFTP and NOMADS URLs.
pub fn rid_prefix(code: &str) -> &'static str {
    match code {
        "JUA" => "t",
        "HKI" | "HMO" | "HKM" | "HWA" | "APD" | "ACG" | "AIH" | "AHG" | "AKC" | "ABC" | "AEC"
        | "GUA" => "p",
        _ => "k",
    }
}

/// Returns all radar sites as (code, description) pairs.
pub fn all_sites() -> Vec<(String, String)> {
    RADAR_SITES
        .iter()
        .map(|&(code, desc, _, _)| (code.to_string(), desc.to_string()))
        .collect()
}
