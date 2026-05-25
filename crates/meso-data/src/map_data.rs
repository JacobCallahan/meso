/*
 * Map geometry data: county lines, state lines, lakes/rivers, major roads, cities.
 *
 * Binary files are ported directly from the wX Android app (GPLv3).
 * Format: consecutive big-endian f32 pairs (lat, lon_positive_west).
 * Each group of 4 floats is one line segment: (lat1, lon1, lat2, lon2).
 * Longitudes are stored as positive-west values; we negate them on load.
 */

/// A single geographic line segment (lat/lon, standard sign convention).
#[derive(Debug, Clone, Copy)]
pub struct GeoSegment {
    pub lat1: f32,
    pub lon1: f32,
    pub lat2: f32,
    pub lon2: f32,
}

/// A city with name, state, coordinates, and population.
#[derive(Debug, Clone)]
pub struct City {
    pub name: String,
    pub state: String,
    pub lat: f32,
    pub lon: f32,
    pub population: u32,
}

/// All embedded map layers, parsed once at startup.
pub struct MapData {
    pub counties: Vec<GeoSegment>,
    pub states: Vec<GeoSegment>,
    pub lakes: Vec<GeoSegment>,
    pub roads_major: Vec<GeoSegment>,
    pub cities: Vec<City>,
}

// Embedded binary data from wX (GPLv3): big-endian f32 pairs (lat, +west_lon)
static COUNTY_BIN: &[u8] = include_bytes!("../data/maps/county.bin");
static STATES_BIN: &[u8] = include_bytes!("../data/maps/states.bin");
static LAKES_BIN: &[u8] = include_bytes!("../data/maps/lakes.bin");
static ROADS_MAJOR_BIN: &[u8] = include_bytes!("../data/maps/hwv4.bin");
static CITIES_TXT: &str = include_str!("../data/maps/cities.txt");

impl MapData {
    pub fn load() -> Self {
        MapData {
            counties: parse_segments(COUNTY_BIN),
            states: parse_segments(STATES_BIN),
            lakes: parse_segments(LAKES_BIN),
            roads_major: parse_segments(ROADS_MAJOR_BIN),
            cities: parse_cities(CITIES_TXT),
        }
    }
}

/// Parse a wX-format binary geometry file into line segments.
/// Each 16-byte block: (lat1_be_f32, lon1_pos_be_f32, lat2_be_f32, lon2_pos_be_f32).
fn parse_segments(data: &[u8]) -> Vec<GeoSegment> {
    let n_floats = data.len() / 4;
    let n_segs = n_floats / 4;
    let mut segs = Vec::with_capacity(n_segs);

    for i in 0..n_segs {
        let base = i * 16;
        if base + 15 >= data.len() {
            break;
        }
        let lat1 = f32::from_be_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]);
        let lon1 = f32::from_be_bytes([
            data[base + 4],
            data[base + 5],
            data[base + 6],
            data[base + 7],
        ]);
        let lat2 = f32::from_be_bytes([
            data[base + 8],
            data[base + 9],
            data[base + 10],
            data[base + 11],
        ]);
        let lon2 = f32::from_be_bytes([
            data[base + 12],
            data[base + 13],
            data[base + 14],
            data[base + 15],
        ]);

        // Skip clearly invalid/sentinel values
        if !lat1.is_finite() || !lon1.is_finite() || !lat2.is_finite() || !lon2.is_finite() {
            continue;
        }
        // Longitudes stored as positive-west; negate for standard convention
        segs.push(GeoSegment {
            lat1,
            lon1: -lon1,
            lat2,
            lon2: -lon2,
        });
    }
    segs
}

/// Parse cityall.txt: "STATE, City Name, lat, lon, population" per line.
fn parse_cities(text: &str) -> Vec<City> {
    let mut cities = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(5, ',').collect();
        if parts.len() < 5 {
            continue;
        }
        let state = parts[0].trim().to_string();
        let name = parts[1].trim().to_string();
        let lat = match parts[2].trim().parse::<f32>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let lon = match parts[3].trim().parse::<f32>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let pop = match parts[4].trim().parse::<u32>() {
            Ok(v) => v,
            Err(_) => 0,
        };
        cities.push(City {
            name,
            state,
            lat,
            lon,
            population: pop,
        });
    }
    // Sort descending by population so we always draw largest cities first
    cities.sort_by(|a, b| b.population.cmp(&a.population));
    cities
}
