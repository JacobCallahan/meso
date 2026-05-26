/// Latitude/Longitude coordinate pair.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

impl LatLon {
    pub fn new(lat: f64, lon: f64) -> Self {
        Self { lat, lon }
    }

    /// Returns the haversine distance in kilometers to another point.
    pub fn distance_km(&self, other: &LatLon) -> f64 {
        const R: f64 = 6371.0;
        let dlat = (other.lat - self.lat).to_radians();
        let dlon = (other.lon - self.lon).to_radians();
        let a = (dlat / 2.0).sin().powi(2)
            + self.lat.to_radians().cos()
                * other.lat.to_radians().cos()
                * (dlon / 2.0).sin().powi(2);
        let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
        R * c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_same_point_is_zero() {
        let p = LatLon::new(35.0, -97.0);
        assert!(p.distance_km(&p) < 0.001);
    }

    #[test]
    fn distance_okc_to_tulsa_approx_157km() {
        // OKC (35.4676, -97.5164) → Tulsa (36.1540, -95.9928) ≈ 157 km.
        let okc = LatLon::new(35.4676, -97.5164);
        let tulsa = LatLon::new(36.1540, -95.9928);
        let d = okc.distance_km(&tulsa);
        assert!((d - 157.0).abs() < 5.0, "Expected ~157 km, got {d:.1} km");
    }

    #[test]
    fn distance_is_symmetric() {
        let a = LatLon::new(40.0, -90.0);
        let b = LatLon::new(35.0, -95.0);
        let diff = (a.distance_km(&b) - b.distance_km(&a)).abs();
        assert!(diff < 0.001, "distance should be symmetric, diff={diff}");
    }

    #[test]
    fn distance_roughly_111km_per_degree_latitude() {
        // One degree of latitude ≈ 111 km anywhere.
        let a = LatLon::new(35.0, -97.0);
        let b = LatLon::new(36.0, -97.0);
        let d = a.distance_km(&b);
        assert!((d - 111.0).abs() < 2.0, "Expected ~111 km, got {d:.1} km");
    }
}
