/// All available Level 3 (NIDS) radar products and their TGFTP directory strings.
/// Ported from wX `GlobalDictionaries.kt`.
///
/// The TGFTP URL pattern is:
///   `https://tgftp.nws.noaa.gov/SL.us008001/DF.of/DC.radar/{dir}/SI.{prefix}{site}/sn.last`
pub const TGFTP_BASE: &str = "https://tgftp.nws.noaa.gov";
pub const NOMADS_L2_BASE: &str = "https://nomads.ncep.noaa.gov/pub/data/nccf/radar/nexrad_level2/";
pub const NWS_API_BASE: &str = "https://api.weather.gov";
pub const NWS_SPC_BASE: &str = "https://www.spc.noaa.gov";
pub const NWS_WPC_BASE: &str = "https://www.wpc.ncep.noaa.gov";
pub const GOES_CDN_BASE: &str = "https://cdn.star.nesdis.noaa.gov";
pub const GOES_ANIM_BASE: &str = "https://www.star.nesdis.noaa.gov";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RadarProduct {
    // Level 2
    L2Reflectivity,
    L2Velocity,

    // Level 3 Base Reflectivity (super-res)
    N0Q,
    N1Q,
    N2Q,
    N3Q,
    // Level 3 Base Reflectivity (legacy)
    N0R,
    N1R,
    N2R,
    N3R,
    // Level 3 Velocity (super-res)
    N0U,
    N1U,
    N2U,
    N3U,
    // Level 3 Velocity (legacy)
    N0V,
    N1V,
    N2V,
    N3V,
    // Level 3 Spectrum Width
    N0S,
    N1S,
    N2S,
    N3S,
    // Dual-pol
    N0X,
    N1X,
    N2X,
    N3X, // ZDR
    N0C,
    N1C,
    N2C,
    N3C, // CC
    N0K,
    N1K,
    N2K,
    N3K, // KDP
    H0C,
    H1C,
    H2C,
    H3C, // HC
    // Derived products
    VIL,
    DVL,
    EET,
    ET,
    NCR, // Composite reflectivity
    NCZ, // Composite reflectivity (Z)
    // QPE
    N1P, // 1-hr precip
    NTP, // Storm total precip
    DAA, // Digital accumulation
    DSA, // Digital storm total
    // Severe
    HI,  // Hail index
    STI, // Storm track info
    TVS, // Tornado vortex signature
    VWP, // Wind profile
    // TDWR
    TR0,
    TR1,
    TR2,
    TR3, // TDWR reflectivity
    TV0,
    TV1,
    TV2,
    TV3, // TDWR velocity
    TZ0,
    TZ1,
    TZ2, // TDWR super-res Z
    TZL, // TDWR super-res long range
}

impl RadarProduct {
    /// Returns the TGFTP directory segment for this product.
    pub fn tgftp_dir(&self) -> Option<&'static str> {
        match self {
            Self::N0R => Some("DS.p94r0"),
            Self::N1R => Some("DS.p94r1"),
            Self::N2R => Some("DS.p94r2"),
            Self::N3R => Some("DS.p94r3"),
            Self::N0Q => Some("DS.p94r0"),
            Self::N1Q => Some("DS.p94r1"),
            Self::N2Q => Some("DS.p94r2"),
            Self::N3Q => Some("DS.p94r3"),
            Self::N0V => Some("DS.p99v0"),
            Self::N1V => Some("DS.p99v1"),
            Self::N2V => Some("DS.p99v2"),
            Self::N3V => Some("DS.p99v3"),
            Self::N0U => Some("DS.p99v0"),
            Self::N1U => Some("DS.p99v1"),
            Self::N2U => Some("DS.p99v2"),
            Self::N3U => Some("DS.p99v3"),
            Self::N0S => Some("DS.56rm0"),
            Self::N1S => Some("DS.56rm1"),
            Self::N2S => Some("DS.56rm2"),
            Self::N3S => Some("DS.56rm3"),
            Self::NCR => Some("DS.p37cr"),
            Self::NCZ => Some("DS.p38cr"),
            Self::ET => Some("DS.p41et"),
            Self::VIL => Some("DS.57vil"),
            Self::STI => Some("DS.58sti"),
            Self::HI => Some("DS.p59hi"),
            Self::TVS => Some("DS.61tvs"),
            Self::DVL => Some("DS.134il"),
            Self::EET => Some("DS.135et"),
            Self::N0X => Some("DS.159x0"),
            Self::N1X => Some("DS.159x1"),
            Self::N2X => Some("DS.159x2"),
            Self::N3X => Some("DS.159x3"),
            Self::N0C => Some("DS.161c0"),
            Self::N1C => Some("DS.161c1"),
            Self::N2C => Some("DS.161c2"),
            Self::N3C => Some("DS.161c3"),
            Self::N0K => Some("DS.163k0"),
            Self::N1K => Some("DS.163k1"),
            Self::N2K => Some("DS.163k2"),
            Self::N3K => Some("DS.163k3"),
            Self::H0C => Some("DS.165h0"),
            Self::H1C => Some("DS.165h1"),
            Self::H2C => Some("DS.165h2"),
            Self::H3C => Some("DS.165h3"),
            Self::DAA => Some("DS.170aa"),
            Self::DSA => Some("DS.172dt"),
            Self::N1P => Some("DS.78ohp"),
            Self::NTP => Some("DS.80stp"),
            Self::VWP => Some("DS.48vwp"),
            Self::TR0 => Some("DS.181r0"),
            Self::TR1 => Some("DS.181r1"),
            Self::TR2 => Some("DS.181r2"),
            Self::TR3 => Some("DS.181r3"),
            Self::TV0 => Some("DS.182v0"),
            Self::TV1 => Some("DS.182v1"),
            Self::TV2 => Some("DS.182v2"),
            Self::TV3 => Some("DS.182v3"),
            Self::TZ0 => Some("DS.180z0"),
            Self::TZ1 => Some("DS.180z1"),
            Self::TZ2 => Some("DS.180z2"),
            Self::TZL => Some("DS.186zl"),
            Self::L2Reflectivity | Self::L2Velocity => None,
        }
    }

    pub fn is_level2(&self) -> bool {
        matches!(self, Self::L2Reflectivity | Self::L2Velocity)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::L2Reflectivity => "L2 Reflectivity",
            Self::L2Velocity => "L2 Velocity",
            Self::N0Q => "Base Reflectivity (0.5°)",
            Self::N1Q => "Base Reflectivity (1.5°)",
            Self::N2Q => "Base Reflectivity (2.5°)",
            Self::N3Q => "Base Reflectivity (3.5°)",
            Self::N0U => "Base Velocity (0.5°)",
            Self::N1U => "Base Velocity (1.5°)",
            Self::N2U => "Base Velocity (2.5°)",
            Self::N3U => "Base Velocity (3.5°)",
            Self::N0X => "ZDR (0.5°)",
            Self::N1X => "ZDR (1.5°)",
            Self::N2X => "ZDR (2.5°)",
            Self::N3X => "ZDR (3.5°)",
            Self::N0C => "CC (0.5°)",
            Self::N1C => "CC (1.5°)",
            Self::N2C => "CC (2.5°)",
            Self::N3C => "CC (3.5°)",
            Self::N0K => "KDP (0.5°)",
            Self::N1K => "KDP (1.5°)",
            Self::N2K => "KDP (2.5°)",
            Self::N3K => "KDP (3.5°)",
            Self::H0C => "HC (0.5°)",
            Self::H1C => "HC (1.5°)",
            Self::H2C => "HC (2.5°)",
            Self::H3C => "HC (3.5°)",
            Self::VIL => "VIL",
            Self::DVL => "Digital VIL",
            Self::EET => "Enhanced Echo Tops",
            Self::ET => "Echo Tops",
            Self::NCR => "Composite Reflectivity (124)",
            Self::HI => "Hail Index",
            Self::STI => "Storm Track Info",
            Self::TVS => "Tornado Vortex Signature",
            Self::N1P => "1-hr Precipitation",
            Self::NTP => "Storm Total Precipitation",
            Self::DAA => "Digital Accum. Array",
            Self::DSA => "Digital Storm Total",
            Self::VWP => "VAD Wind Profile",
            Self::N0S => "Storm-Relative Velocity (0.5°)",
            Self::N1S => "Storm-Relative Velocity (1.5°)",
            Self::N2S => "Storm-Relative Velocity (2.5°)",
            Self::N3S => "Storm-Relative Velocity (3.5°)",
            Self::N0R => "Base Reflectivity 0.5° (Legacy)",
            Self::N1R => "Base Reflectivity 1.5° (Legacy)",
            Self::N2R => "Base Reflectivity 2.5° (Legacy)",
            Self::N3R => "Base Reflectivity 3.5° (Legacy)",
            Self::N0V => "Base Velocity 0.5° (Legacy)",
            Self::N1V => "Base Velocity 1.5° (Legacy)",
            Self::N2V => "Base Velocity 2.5° (Legacy)",
            Self::N3V => "Base Velocity 3.5° (Legacy)",
            Self::NCZ => "Composite Reflectivity (248)",
            Self::TR0 => "TDWR Reflectivity (0°)",
            Self::TR1 => "TDWR Reflectivity (1°)",
            Self::TR2 => "TDWR Reflectivity (2°)",
            Self::TR3 => "TDWR Reflectivity (3°)",
            Self::TV0 => "TDWR Velocity (0°)",
            Self::TV1 => "TDWR Velocity (1°)",
            Self::TV2 => "TDWR Velocity (2°)",
            Self::TV3 => "TDWR Velocity (3°)",
            Self::TZ0 => "TDWR Super-res Ref (0°)",
            Self::TZ1 => "TDWR Super-res Ref (1°)",
            Self::TZ2 => "TDWR Super-res Ref (2°)",
            Self::TZL => "TDWR Super-res Ref (Long Range)",
        }
    }

    /// Returns a short scale/units description for the status bar.
    pub fn description_line(&self) -> &'static str {
        match self {
            Self::L2Reflectivity
            | Self::N0Q
            | Self::N1Q
            | Self::N2Q
            | Self::N3Q
            | Self::N0R
            | Self::N1R
            | Self::N2R
            | Self::N3R
            | Self::NCR
            | Self::NCZ => "Reflectivity (dBZ): −32 to +94.5",
            Self::L2Velocity
            | Self::N0U
            | Self::N1U
            | Self::N2U
            | Self::N3U
            | Self::N0V
            | Self::N1V
            | Self::N2V
            | Self::N3V => "Velocity (kt): −128 to +127",
            Self::N0S | Self::N1S | Self::N2S | Self::N3S => {
                "Storm-Relative Velocity (kt): −128 to +127"
            }
            Self::N0X | Self::N1X | Self::N2X | Self::N3X => "ZDR (dB): −7.9 to +7.9",
            Self::N0C | Self::N1C | Self::N2C | Self::N3C => "Correlation Coefficient: 0.0 – 1.05",
            Self::N0K | Self::N1K | Self::N2K | Self::N3K => "KDP (°/km): −3 to +20",
            Self::H0C | Self::H1C | Self::H2C | Self::H3C => {
                "Hydrometeor Class: ND / Bio / AP / IC / DS / WS / RA / HR / BD / GR / HA"
            }
            Self::DVL | Self::VIL => "Digital VIL (kg/m²): 0 – 75",
            Self::EET | Self::ET => "Echo Tops (kft): 0 – 70",
            Self::N1P | Self::DAA => "1-hr Precip (in): 0 – 8",
            Self::NTP | Self::DSA => "Storm Total Precip (in): 0 – 20",
            Self::HI => "Hail Index: POSH / POH / Max Hail Size",
            Self::TVS => "Tornado Vortex Signatures",
            Self::STI => "Storm Cell Tracks",
            Self::VWP => "VAD Wind Profile",
            _ => "",
        }
    }

    /// Short code string (e.g. "N0Q", "L2Reflectivity").
    pub fn code(&self) -> &'static str {
        match self {
            Self::L2Reflectivity => "L2Reflectivity",
            Self::L2Velocity => "L2Velocity",
            Self::N0Q => "N0Q",
            Self::N1Q => "N1Q",
            Self::N2Q => "N2Q",
            Self::N3Q => "N3Q",
            Self::N0R => "N0R",
            Self::N1R => "N1R",
            Self::N2R => "N2R",
            Self::N3R => "N3R",
            Self::N0U => "N0U",
            Self::N1U => "N1U",
            Self::N2U => "N2U",
            Self::N3U => "N3U",
            Self::N0V => "N0V",
            Self::N1V => "N1V",
            Self::N2V => "N2V",
            Self::N3V => "N3V",
            Self::N0S => "N0S",
            Self::N1S => "N1S",
            Self::N2S => "N2S",
            Self::N3S => "N3S",
            Self::N0X => "N0X",
            Self::N1X => "N1X",
            Self::N2X => "N2X",
            Self::N3X => "N3X",
            Self::N0C => "N0C",
            Self::N1C => "N1C",
            Self::N2C => "N2C",
            Self::N3C => "N3C",
            Self::N0K => "N0K",
            Self::N1K => "N1K",
            Self::N2K => "N2K",
            Self::N3K => "N3K",
            Self::H0C => "H0C",
            Self::H1C => "H1C",
            Self::H2C => "H2C",
            Self::H3C => "H3C",
            Self::VIL => "VIL",
            Self::DVL => "DVL",
            Self::EET => "EET",
            Self::ET => "ET",
            Self::NCR => "NCR",
            Self::NCZ => "NCZ",
            Self::N1P => "N1P",
            Self::NTP => "NTP",
            Self::DAA => "DAA",
            Self::DSA => "DSA",
            Self::HI => "HI",
            Self::STI => "STI",
            Self::TVS => "TVS",
            Self::VWP => "VWP",
            Self::TR0 => "TR0",
            Self::TR1 => "TR1",
            Self::TR2 => "TR2",
            Self::TR3 => "TR3",
            Self::TV0 => "TV0",
            Self::TV1 => "TV1",
            Self::TV2 => "TV2",
            Self::TV3 => "TV3",
            Self::TZ0 => "TZ0",
            Self::TZ1 => "TZ1",
            Self::TZ2 => "TZ2",
            Self::TZL => "TZL",
        }
    }

    /// Parse from short code string.
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "L2Reflectivity" => Some(Self::L2Reflectivity),
            "L2Velocity" => Some(Self::L2Velocity),
            "N0Q" => Some(Self::N0Q),
            "N1Q" => Some(Self::N1Q),
            "N2Q" => Some(Self::N2Q),
            "N3Q" => Some(Self::N3Q),
            "N0R" => Some(Self::N0R),
            "N1R" => Some(Self::N1R),
            "N2R" => Some(Self::N2R),
            "N3R" => Some(Self::N3R),
            "N0U" => Some(Self::N0U),
            "N1U" => Some(Self::N1U),
            "N2U" => Some(Self::N2U),
            "N3U" => Some(Self::N3U),
            "N0V" => Some(Self::N0V),
            "N1V" => Some(Self::N1V),
            "N2V" => Some(Self::N2V),
            "N3V" => Some(Self::N3V),
            "N0S" => Some(Self::N0S),
            "N1S" => Some(Self::N1S),
            "N2S" => Some(Self::N2S),
            "N3S" => Some(Self::N3S),
            "N0X" => Some(Self::N0X),
            "N1X" => Some(Self::N1X),
            "N2X" => Some(Self::N2X),
            "N3X" => Some(Self::N3X),
            "N0C" => Some(Self::N0C),
            "N1C" => Some(Self::N1C),
            "N2C" => Some(Self::N2C),
            "N3C" => Some(Self::N3C),
            "N0K" => Some(Self::N0K),
            "N1K" => Some(Self::N1K),
            "N2K" => Some(Self::N2K),
            "N3K" => Some(Self::N3K),
            "H0C" => Some(Self::H0C),
            "H1C" => Some(Self::H1C),
            "H2C" => Some(Self::H2C),
            "H3C" => Some(Self::H3C),
            "VIL" => Some(Self::VIL),
            "DVL" => Some(Self::DVL),
            "EET" => Some(Self::EET),
            "ET" => Some(Self::ET),
            "NCR" => Some(Self::NCR),
            "NCZ" => Some(Self::NCZ),
            "N1P" => Some(Self::N1P),
            "NTP" => Some(Self::NTP),
            "DAA" => Some(Self::DAA),
            "DSA" => Some(Self::DSA),
            "HI" => Some(Self::HI),
            "STI" => Some(Self::STI),
            "TVS" => Some(Self::TVS),
            "VWP" => Some(Self::VWP),
            "TR0" => Some(Self::TR0),
            "TR1" => Some(Self::TR1),
            "TR2" => Some(Self::TR2),
            "TR3" => Some(Self::TR3),
            "TV0" => Some(Self::TV0),
            "TV1" => Some(Self::TV1),
            "TV2" => Some(Self::TV2),
            "TV3" => Some(Self::TV3),
            "TZ0" => Some(Self::TZ0),
            "TZ1" => Some(Self::TZ1),
            "TZ2" => Some(Self::TZ2),
            "TZL" => Some(Self::TZL),
            _ => None,
        }
    }

    /// All Level 3 products (excludes L2Reflectivity and L2Velocity).
    pub fn all_level3() -> Vec<Self> {
        vec![
            Self::N0Q,
            Self::N1Q,
            Self::N2Q,
            Self::N3Q,
            Self::N0U,
            Self::N1U,
            Self::N2U,
            Self::N3U,
            Self::N0X,
            Self::N1X,
            Self::N2X,
            Self::N3X,
            Self::N0C,
            Self::N1C,
            Self::N2C,
            Self::N3C,
            Self::N0K,
            Self::N1K,
            Self::N2K,
            Self::N3K,
            Self::H0C,
            Self::H1C,
            Self::H2C,
            Self::H3C,
            Self::N0S,
            Self::N1S,
            Self::N2S,
            Self::N3S,
            Self::N0R,
            Self::N1R,
            Self::N2R,
            Self::N3R,
            Self::N0V,
            Self::N1V,
            Self::N2V,
            Self::N3V,
            Self::VIL,
            Self::DVL,
            Self::EET,
            Self::ET,
            Self::NCR,
            Self::NCZ,
            Self::N1P,
            Self::NTP,
            Self::DAA,
            Self::DSA,
            Self::HI,
            Self::STI,
            Self::TVS,
            Self::VWP,
            Self::TR0,
            Self::TR1,
            Self::TR2,
            Self::TR3,
            Self::TV0,
            Self::TV1,
            Self::TV2,
            Self::TV3,
            Self::TZ0,
            Self::TZ1,
            Self::TZ2,
            Self::TZL,
        ]
    }

    /// All products — L2 first, then all Level 3.
    pub fn all_products() -> Vec<Self> {
        let mut v = vec![Self::L2Reflectivity, Self::L2Velocity];
        v.extend(Self::all_level3());
        v
    }

    /// Products that use velocity color scale.
    pub fn is_velocity(&self) -> bool {
        matches!(
            self,
            Self::N0V
                | Self::N1V
                | Self::N2V
                | Self::N3V
                | Self::N0U
                | Self::N1U
                | Self::N2U
                | Self::N3U
                | Self::N0S
                | Self::N1S
                | Self::N2S
                | Self::N3S
                | Self::TV0
                | Self::TV1
                | Self::TV2
                | Self::TV3
                | Self::L2Velocity
        )
    }

    /// True when the product is supported as a map-rendered radar layer.
    pub fn is_map_supported(&self) -> bool {
        !matches!(
            self,
            Self::N1P
                | Self::NTP
                | Self::DAA
                | Self::DSA
                | Self::HI
                | Self::STI
                | Self::TVS
                | Self::VWP
        )
    }

    /// Returns the UI group name for this product.
    pub fn group_name(&self) -> &'static str {
        match self {
            Self::L2Reflectivity | Self::L2Velocity => "Level 2",
            Self::N0Q
            | Self::N1Q
            | Self::N2Q
            | Self::N3Q
            | Self::N0R
            | Self::N1R
            | Self::N2R
            | Self::N3R => "Base Reflectivity",
            Self::N0U
            | Self::N1U
            | Self::N2U
            | Self::N3U
            | Self::N0V
            | Self::N1V
            | Self::N2V
            | Self::N3V => "Base Velocity",
            Self::N0S | Self::N1S | Self::N2S | Self::N3S => "Storm-Relative Velocity",
            Self::N0X
            | Self::N1X
            | Self::N2X
            | Self::N3X
            | Self::N0C
            | Self::N1C
            | Self::N2C
            | Self::N3C
            | Self::N0K
            | Self::N1K
            | Self::N2K
            | Self::N3K
            | Self::H0C
            | Self::H1C
            | Self::H2C
            | Self::H3C => "Dual-pol",
            Self::VIL | Self::DVL | Self::EET | Self::ET | Self::NCR | Self::NCZ => {
                "Derived / VIL / QPE"
            }
            _ => "Other",
        }
    }

    /// Returns products belonging to the named UI group (excludes TDWR).
    pub fn for_group(group: &str) -> Vec<Self> {
        match group {
            "Level 2" => vec![Self::L2Reflectivity, Self::L2Velocity],
            "Base Reflectivity" => vec![
                Self::N0Q,
                Self::N1Q,
                Self::N2Q,
                Self::N3Q,
                Self::N0R,
                Self::N1R,
                Self::N2R,
                Self::N3R,
            ],
            "Base Velocity" => vec![
                Self::N0U,
                Self::N1U,
                Self::N2U,
                Self::N3U,
                Self::N0V,
                Self::N1V,
                Self::N2V,
                Self::N3V,
            ],
            "Storm-Relative Velocity" => vec![Self::N0S, Self::N1S, Self::N2S, Self::N3S],
            "Dual-pol" => vec![
                Self::N0X,
                Self::N1X,
                Self::N2X,
                Self::N3X,
                Self::N0C,
                Self::N1C,
                Self::N2C,
                Self::N3C,
                Self::N0K,
                Self::N1K,
                Self::N2K,
                Self::N3K,
                Self::H0C,
                Self::H1C,
                Self::H2C,
                Self::H3C,
            ],
            "Derived / VIL / QPE" => vec![
                Self::NCR,
                Self::NCZ,
                Self::DVL,
                Self::VIL,
                Self::EET,
                Self::ET,
            ],
            _ => Vec::new(),
        }
    }

    /// All product groups in display order (excluding TDWR/Other).
    pub const PRODUCT_GROUPS: &'static [&'static str] = &[
        "Level 2",
        "Base Reflectivity",
        "Base Velocity",
        "Storm-Relative Velocity",
        "Dual-pol",
        "Derived / VIL / QPE",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_code_roundtrip() {
        for product in RadarProduct::all_products() {
            let code = product.code();
            assert_eq!(
                RadarProduct::from_code(code),
                Some(product),
                "from_code({code}) round-trip failed"
            );
        }
    }

    #[test]
    fn from_code_unknown_returns_none() {
        assert_eq!(RadarProduct::from_code("BOGUS"), None);
        assert_eq!(RadarProduct::from_code(""), None);
    }

    #[test]
    fn is_level2_flags() {
        assert!(RadarProduct::L2Reflectivity.is_level2());
        assert!(RadarProduct::L2Velocity.is_level2());
        assert!(!RadarProduct::N0Q.is_level2());
        assert!(!RadarProduct::N0U.is_level2());
    }

    #[test]
    fn tgftp_dir_level3_has_value() {
        assert!(RadarProduct::N0Q.tgftp_dir().is_some());
        assert!(RadarProduct::N0U.tgftp_dir().is_some());
        assert!(RadarProduct::NCR.tgftp_dir().is_some());
    }

    #[test]
    fn tgftp_dir_level2_is_none() {
        assert!(RadarProduct::L2Reflectivity.tgftp_dir().is_none());
        assert!(RadarProduct::L2Velocity.tgftp_dir().is_none());
    }

    #[test]
    fn is_velocity_covers_velocity_products() {
        for p in [
            RadarProduct::N0U,
            RadarProduct::N0V,
            RadarProduct::N0S,
            RadarProduct::L2Velocity,
        ] {
            assert!(p.is_velocity(), "{p:?} should be velocity");
        }
        for p in [
            RadarProduct::N0Q,
            RadarProduct::L2Reflectivity,
            RadarProduct::NCR,
        ] {
            assert!(!p.is_velocity(), "{p:?} should not be velocity");
        }
    }

    #[test]
    fn for_group_level2_returns_two_products() {
        let group = RadarProduct::for_group("Level 2");
        assert_eq!(group.len(), 2);
        assert!(group.contains(&RadarProduct::L2Reflectivity));
        assert!(group.contains(&RadarProduct::L2Velocity));
    }

    #[test]
    fn for_group_unknown_is_empty() {
        assert!(RadarProduct::for_group("Nonexistent").is_empty());
    }

    #[test]
    fn all_products_non_empty_and_has_l2() {
        let all = RadarProduct::all_products();
        assert!(!all.is_empty());
        assert!(all.contains(&RadarProduct::L2Reflectivity));
        assert!(all.contains(&RadarProduct::N0Q));
    }
}
