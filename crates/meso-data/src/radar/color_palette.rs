use anyhow::{bail, Result};
use std::collections::HashMap;

// ── Embedded wX palette files ─────────────────────────────────────────────────

const PAL_REF_CODENH: &str = include_str!("../../data/palettes/ref_codenh.txt");
const PAL_REF_EAK: &str = include_str!("../../data/palettes/ref_eak.txt");
const PAL_REF_AF: &str = include_str!("../../data/palettes/ref_af.txt");
const PAL_REF_NSSL: &str = include_str!("../../data/palettes/ref_nssl.txt");
const PAL_REF_NWSD: &str = include_str!("../../data/palettes/ref_nwsd.txt");
const PAL_REF_NWS: &str = include_str!("../../data/palettes/ref_nws.txt");
const PAL_REF_MENH: &str = include_str!("../../data/palettes/ref_menh.txt");
const PAL_REF_DKENH: &str = include_str!("../../data/palettes/ref_dkenh.txt");

const PAL_VEL_CODENH: &str = include_str!("../../data/palettes/vel_codenh.txt");
const PAL_VEL_EAK: &str = include_str!("../../data/palettes/vel_eak.txt");
const PAL_VEL_AF: &str = include_str!("../../data/palettes/vel_af.txt");

/// Available reflectivity palette names (in display order).
pub const REF_PALETTE_NAMES: &[&str] = &[
    "CODENH", "EAK", "AF", "NSSL", "NWSD", "NWS", "MENH", "DKenh",
];

/// Available velocity palette names.
pub const VEL_PALETTE_NAMES: &[&str] = &["CODENH", "EAK", "AF"];

// ── Core palette type ─────────────────────────────────────────────────────────

/// A lookup table mapping gate index (0–255) → [R, G, B].
#[derive(Debug, Clone)]
pub struct ColorPalette {
    pub table: [[u8; 3]; 256],
}

impl Default for ColorPalette {
    fn default() -> Self {
        ColorPalette {
            table: [[0u8; 3]; 256],
        }
    }
}

impl ColorPalette {
    pub fn color_for_gate(&self, gate: u8) -> [u8; 3] {
        self.table[gate as usize]
    }

    /// Alias kept for compatibility with geometry.rs callers.
    pub fn color(&self, gate: u8) -> (u8, u8, u8) {
        let [r, g, b] = self.table[gate as usize];
        (r, g, b)
    }

    /// Build a palette by interpolating between control points.
    /// Each control point is `(gate_index_0_255, R, G, B)`.
    pub fn from_control_points(points: &[(u8, u8, u8, u8)]) -> Self {
        let mut table = [[0u8; 3]; 256];
        let n = points.len();
        if n == 0 {
            return ColorPalette { table };
        }
        for i in 0..n.saturating_sub(1) {
            let (g0, r0, g0c, b0) = points[i];
            let (g1, r1, g1c, b1) = points[i + 1];
            let range = (g1 as i32 - g0 as i32).max(1);
            for g in g0..=g1 {
                let t = (g as i32 - g0 as i32) as f32 / range as f32;
                table[g as usize] = [lerp(r0, r1, t), lerp(g0c, g1c, t), lerp(b0, b1, t)];
            }
        }
        ColorPalette { table }
    }

    /// Parse a wX-style dBZ palette text file.
    ///
    /// Line format: `Color,VALUE,R,G,B[,...]`
    ///
    /// For reflectivity: gate = ((dbz + 32) * 2).round().clamp(0, 255)
    /// For velocity:     gate = (vel + 127).round().clamp(0, 255)
    pub fn from_dbz_pal_text(text: &str, is_velocity: bool) -> Result<Self> {
        let mut entries: Vec<(u8, [u8; 3])> = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if !line.to_lowercase().starts_with("color") {
                continue;
            }
            let parts: Vec<&str> = line.splitn(6, ',').collect();
            if parts.len() < 5 {
                continue;
            }
            let value: f64 = match parts[1].trim().parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let r: u8 = match parts[2].trim().parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let g: u8 = match parts[3].trim().parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            // Strip trailing comma that some files include
            let b_str = parts[4].trim().trim_end_matches(',');
            let b: u8 = match b_str.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            let gate = if is_velocity {
                (value + 127.0).round().clamp(0.0, 255.0) as u8
            } else {
                ((value + 32.0) * 2.0).round().clamp(0.0, 255.0) as u8
            };
            entries.push((gate, [r, g, b]));
        }
        if entries.is_empty() {
            bail!("no valid Color entries in palette");
        }
        entries.sort_by_key(|(gate, _)| *gate);

        let mut table = [[0u8; 3]; 256];
        // Interpolate between consecutive control points
        let n = entries.len();
        for i in 0..n.saturating_sub(1) {
            let (g0, c0) = entries[i];
            let (g1, c1) = entries[i + 1];
            let range = (g1 as i32 - g0 as i32).max(1);
            for g in g0..=g1 {
                let t = (g as i32 - g0 as i32) as f32 / range as f32;
                table[g as usize] = [
                    lerp(c0[0], c1[0], t),
                    lerp(c0[1], c1[1], t),
                    lerp(c0[2], c1[2], t),
                ];
            }
        }
        // Fill before first and after last
        for g in 0..entries[0].0 {
            table[g as usize] = entries[0].1;
        }
        let last_color = entries[n - 1].1;
        table[(entries[n - 1].0 as usize + 1)..].fill(last_color);

        Ok(ColorPalette { table })
    }

    /// Build a named reflectivity palette.
    pub fn named_ref(name: &str) -> ColorPalette {
        let src = match name {
            "EAK" => PAL_REF_EAK,
            "AF" => PAL_REF_AF,
            "NSSL" => PAL_REF_NSSL,
            "NWSD" => PAL_REF_NWSD,
            "NWS" => PAL_REF_NWS,
            "MENH" => PAL_REF_MENH,
            "DKenh" => PAL_REF_DKENH,
            _ => PAL_REF_CODENH, // default: CODENH
        };
        ColorPalette::from_dbz_pal_text(src, false).unwrap_or_else(|_| ColorPalette::default())
    }

    /// Build a named velocity palette.
    pub fn named_vel(name: &str) -> ColorPalette {
        let src = match name {
            "EAK" => PAL_VEL_EAK,
            "AF" => PAL_VEL_AF,
            _ => PAL_VEL_CODENH, // default: CODENH
        };
        ColorPalette::from_dbz_pal_text(src, true).unwrap_or_else(|_| ColorPalette::default())
    }
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t) as u8
}

// ── Registry ──────────────────────────────────────────────────────────────────

/// Holds all palettes used during rendering.
pub struct PaletteRegistry {
    palettes: HashMap<String, ColorPalette>,
}

impl Default for PaletteRegistry {
    fn default() -> Self {
        Self::with_names("CODENH", "CODENH")
    }
}

impl PaletteRegistry {
    /// Build registry with named reflectivity and velocity palettes.
    pub fn with_names(ref_name: &str, vel_name: &str) -> Self {
        let mut palettes = HashMap::new();
        palettes.insert(
            "reflectivity".to_string(),
            ColorPalette::named_ref(ref_name),
        );
        palettes.insert("velocity".to_string(), ColorPalette::named_vel(vel_name));
        palettes.insert("zdr".to_string(), build_zdr_palette());
        palettes.insert("correlation".to_string(), build_cc_palette());
        palettes.insert("kdp".to_string(), build_kdp_palette());
        palettes.insert("hca".to_string(), build_hca_palette());
        palettes.insert("echo_tops".to_string(), build_echo_tops_palette());
        palettes.insert("vil".to_string(), build_vil_palette());
        palettes.insert("generic".to_string(), build_generic_palette());
        PaletteRegistry { palettes }
    }

    pub fn get(&self, name: &str) -> &ColorPalette {
        self.palettes.get(name).unwrap_or_else(|| {
            self.palettes
                .get("generic")
                .expect("generic palette always present")
        })
    }

    /// Map a NEXRAD product code to the appropriate palette.
    /// Product codes are from the NEXRAD NIDS binary file header.
    pub fn for_product(&self, product_code: u16) -> &ColorPalette {
        match product_code {
            // Reflectivity: 19=N0R, 20=N1R, 21=N2R, 22=N3R, 37=NCR, 38=NCZ
            // 94=N0Q, 99=N1Q, 159=N2Q, 161=N3Q (super-res ref codes vary)
            19 | 20 | 21 | 22 | 37 | 38 | 94 => self.get("reflectivity"),
            // Velocity: 27=N0V, 28=N1V, 29=N2V, 30=N3V, 99=N0U, 182=N0U alt
            // 56=N0S SR-vel family, 25/81/82 legacy vel
            25 | 27 | 28 | 29 | 30 | 55 | 56 | 81 | 82 | 99 | 182 | 186 => self.get("velocity"),
            // ZDR (differential reflectivity): codes 159–162
            159..=162 => self.get("zdr"),
            // Correlation Coefficient: codes 161–164
            // Note: 161/162 overlap with ZDR above; ZDR arm wins for those codes.
            // In practice N0C=163, N1C=164, N2C=165, N3C=166 per wX/NEXRAD ICD.
            163 | 164 => self.get("correlation"),
            // KDP: codes 108–111
            108..=111 => self.get("kdp"),
            // HCA: codes 165–168
            165..=168 => self.get("hca"),
            // VIL: 57=VIL, 134=DVL (digital VIL)
            57 | 134 => self.get("vil"),
            // Echo Tops: 41=ET, 135=EET
            41 | 135 => self.get("echo_tops"),
            _ => self.get("reflectivity"),
        }
    }
}

// ── Specialty palettes (ZDR, CC, KDP) ────────────────────────────────────────

fn build_zdr_palette() -> ColorPalette {
    let points: &[(u8, u8, u8, u8)] = &[
        (0, 0, 0, 128),
        (80, 0, 0, 255),
        (128, 0, 200, 200),
        (160, 0, 255, 0),
        (200, 255, 255, 0),
        (230, 255, 0, 0),
        (255, 200, 0, 200),
    ];
    ColorPalette::from_control_points(points)
}

fn build_cc_palette() -> ColorPalette {
    let points: &[(u8, u8, u8, u8)] = &[
        (0, 60, 0, 60),
        (100, 80, 0, 80),
        (180, 0, 0, 255),
        (210, 0, 200, 200),
        (230, 0, 255, 0),
        (240, 255, 255, 0),
        (250, 255, 0, 0),
        (255, 255, 255, 255),
    ];
    ColorPalette::from_control_points(points)
}

fn build_kdp_palette() -> ColorPalette {
    let points: &[(u8, u8, u8, u8)] = &[
        (0, 0, 0, 128),
        (100, 0, 0, 255),
        (140, 0, 200, 200),
        (170, 0, 255, 0),
        (200, 255, 255, 0),
        (230, 255, 100, 0),
        (255, 255, 0, 0),
    ];
    ColorPalette::from_control_points(points)
}

fn build_hca_palette() -> ColorPalette {
    // Categorical: step palette for HCA class indices 0–10.
    // Classes: 0=ND, 1=Bio, 2=AP/GC, 3=IC, 4=DS, 5=WS, 6=RA, 7=HR, 8=BD, 9=GR, 10=HA
    let points: &[(u8, u8, u8, u8)] = &[
        (0, 0, 0, 0), // 0 = ND (transparent/black)
        (23, 0, 0, 0),
        (24, 0, 160, 0), // 1 = Biological (dark green)
        (47, 0, 160, 0),
        (48, 139, 90, 43), // 2 = AP/Ground Clutter (brown)
        (71, 139, 90, 43),
        (72, 180, 210, 255), // 3 = Ice Crystals (light blue)
        (95, 180, 210, 255),
        (96, 240, 240, 240), // 4 = Dry Snow (white)
        (119, 240, 240, 240),
        (120, 0, 220, 220), // 5 = Wet Snow (cyan)
        (143, 0, 220, 220),
        (144, 0, 200, 0), // 6 = Rain (green)
        (167, 0, 200, 0),
        (168, 255, 220, 0), // 7 = Heavy Rain (yellow)
        (191, 255, 220, 0),
        (192, 255, 140, 0), // 8 = Big Drops (orange)
        (215, 255, 140, 0),
        (216, 0, 255, 100), // 9 = Graupel (lime)
        (239, 0, 255, 100),
        (240, 255, 0, 0), // 10 = Hail/Rain mix (red)
        (255, 255, 0, 0),
    ];
    ColorPalette::from_control_points(points)
}

fn build_echo_tops_palette() -> ColorPalette {
    // Height gradient: 0–70 kft
    let points: &[(u8, u8, u8, u8)] = &[
        (0, 0, 0, 0),         // 0 kft = transparent
        (10, 80, 0, 120),     // ~7 kft = dark purple
        (50, 0, 0, 200),      // ~28 kft = blue
        (100, 0, 180, 220),   // ~56 kft = cyan
        (140, 0, 220, 0),     // ~78 kft = green
        (180, 220, 220, 0),   // ~100 kft = yellow
        (215, 255, 140, 0),   // ~120 kft = orange
        (240, 255, 0, 0),     // ~135 kft = red
        (255, 255, 200, 255), // max = pink/white
    ];
    ColorPalette::from_control_points(points)
}

fn build_vil_palette() -> ColorPalette {
    // VIL intensity: 0–75 kg/m²
    let points: &[(u8, u8, u8, u8)] = &[
        (0, 0, 0, 0),         // 0 = transparent
        (5, 0, 0, 200),       // trace = blue
        (30, 0, 200, 0),      // low = green
        (80, 200, 200, 0),    // moderate = yellow
        (130, 255, 140, 0),   // high = orange
        (180, 255, 0, 0),     // very high = red
        (210, 180, 0, 220),   // extreme = purple
        (240, 255, 255, 255), // max = white
        (255, 255, 255, 255),
    ];
    ColorPalette::from_control_points(points)
}

fn build_generic_palette() -> ColorPalette {
    let points: &[(u8, u8, u8, u8)] = &[
        (0, 0, 0, 0),
        (30, 0, 0, 255),
        (60, 0, 255, 0),
        (90, 220, 220, 0),
        (120, 255, 120, 0),
        (150, 255, 0, 0),
        (180, 200, 0, 200),
        (210, 255, 200, 255),
        (240, 255, 255, 255),
    ];
    ColorPalette::from_control_points(points)
}
