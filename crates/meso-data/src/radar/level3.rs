/*
 * NEXRAD Level 3 (NIDS) product decoder.
 *
 * Parses the standard NIDS (Nexrad Information Dissemination Service) binary
 * format served from NWS TGFTP.  Handles both 8-bit radial products (super-res
 * and legacy) and provides structured radial geometry output for the render engine.
 *
 * File format (from NWS TGFTP sn.last):
 *   [0..30]    WMO text preamble (e.g. "SDUS64 KOUN 222144\r\r\nN0QTLX\r\r\n")
 *   [30..48]   NEXRAD Message Header Block (18 bytes)
 *   [48..150]  Product Description Block (102 bytes)
 *   [150..]    Symbology block — bzip2 compressed for most modern products
 *
 * After bzip2 decompression the symbology block has standard NIDS structure:
 *   Symbology header (10 bytes) → Layer header (6 bytes) →
 *   Packet Code 0x0010 header (14 bytes) → per-radial records
 *
 * Ported from wX's NexradDecodeEightBit.kt, NexradLevel3.kt, and NexradUtil.kt.
 */

use anyhow::Result;
use byteorder::{BigEndian, ReadBytesExt};
use bzip2::read::BzDecoder;
use std::io::{Cursor, Read, Seek, SeekFrom};

// ── Gate bin sizes (km) for each product code ────────────────────────────────

pub fn bin_size_for_product(product_code: u16) -> f32 {
    match product_code {
        94 | 99 => 1.852, // N0Q / N0U legacy TGFTP format: 460 bins × 1 nm
        134 | 135 => 2.0,
        37 => 2.0,           // NCR raster
        38 | 41 | 57 => 8.0, // NCZ / ET / VIL raster
        186 => 0.590022,
        159 | 161 | 163 | 165 | 170 | 172 => 0.50,
        180..=182 => 0.295011,
        78 | 80 => 4.0,
        153 | 154 | 2153 | 2154 => 0.50,
        _ => 1.852,
    }
}

// ── Public output types ───────────────────────────────────────────────────────

/// Decoded output from a Level 3 radial product.
#[derive(Debug, Clone)]
pub struct Level3Data {
    /// Number of radials in this scan.
    pub num_radials: usize,
    /// Number of range bins per radial.
    pub num_range_bins: usize,
    /// Start azimuth for each radial (degrees 0–360), length = `num_radials`.
    pub azimuths: Vec<f32>,
    /// Raw gate data: `num_radials × num_range_bins` bytes.
    pub bins: Vec<u8>,
    /// Gate size in km.
    pub bin_size_km: f32,
    /// True when decoded from raster packet 0xBA07 rather than radial packet 0x0010.
    pub is_raster: bool,
    /// NEXRAD product code.
    pub product_code: u16,
    /// Volume Coverage Pattern.
    pub vcp: u16,
    /// Volume scan date (days since 1 Jan 1970, 1-indexed).
    pub scan_date: u16,
    /// Volume scan time (seconds past midnight UTC).
    pub scan_time_secs: u32,
}

impl Level3Data {
    /// Format the scan time as a UTC timestamp string, e.g. "2024-06-15 21:45 UTC".
    pub fn timestamp_str(&self) -> String {
        use chrono::{Local, TimeZone, Utc};
        let days = self.scan_date as i64 - 1; // 1-indexed → 0-indexed from epoch
        let secs = days * 86400 + self.scan_time_secs as i64;
        match Utc.timestamp_opt(secs, 0).single() {
            Some(dt) => dt
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M %Z")
                .to_string(),
            None => format!("Day {} {:05}s", self.scan_date, self.scan_time_secs),
        }
    }
}

// ── NIDS header ───────────────────────────────────────────────────────────────

struct NidsHeader {
    product_code: u16,
    vcp: u16,
    num_radials: u16,
    num_range_bins: u16,
    data_offset: u64,
    packet_code: u16,
    is_raster: bool,
    scan_date: u16,
    scan_time_secs: u32,
}

// ── Main decoder ─────────────────────────────────────────────────────────────

/// Decode a raw Level 3 NIDS binary (as downloaded from TGFTP `sn.last`).
///
/// Handles the WMO text preamble, bzip2-compressed symbology block, and the
/// standard 8-bit Radial Data Array packet (Packet Code 0x0010).
pub fn decode(raw: &[u8]) -> Result<Level3Data> {
    // Strip WMO preamble and decompress bzip2 symbology block.
    let data = preprocess_nids(raw)?;
    let mut cursor = Cursor::new(data.as_slice());

    let hdr = parse_nids_header(&mut cursor)?;
    let bin_size = bin_size_for_product(hdr.product_code);

    cursor.seek(SeekFrom::Start(hdr.data_offset))?;
    if hdr.is_raster {
        decode_raster(&mut cursor, &hdr, bin_size)
    } else if hdr.packet_code == 0xAF1F {
        decode_radials_4bit_rle(&mut cursor, &hdr, bin_size)
    } else {
        decode_radials(&mut cursor, &hdr, bin_size)
    }
}

/// Skip the WMO text preamble and decompress the bzip2-compressed symbology block
/// if present.  Returns a buffer that starts with the NEXRAD message header (18 bytes)
/// followed by the Product Description Block (102 bytes) and then the (decompressed)
/// symbology block data.
fn preprocess_nids(raw: &[u8]) -> Result<Vec<u8>> {
    // WMO abbreviated heading ends with the second "\r\r\n" pattern, after which
    // the NEXRAD binary starts.  Typical form: "SDUS64 KOUN DDHHMM\r\r\nN0QTLX\r\r\n"
    let nids_start = find_nids_start(raw);
    let raw = &raw[nids_start..];

    // NEXRAD message header (18 bytes) + Product Description Block (102 bytes) = 120 bytes
    // that precede the symbology block.  The symbology block is bzip2-compressed for most
    // modern products (starts with "BZ" magic bytes).
    const HEADERS_LEN: usize = 120;

    if raw.len() > HEADERS_LEN + 2 && raw[HEADERS_LEN] == b'B' && raw[HEADERS_LEN + 1] == b'Z' {
        let mut decomp = Vec::new();
        BzDecoder::new(&raw[HEADERS_LEN..])
            .read_to_end(&mut decomp)
            .map_err(|e| anyhow::anyhow!("bzip2 decompress failed: {e}"))?;
        let mut result = raw[..HEADERS_LEN].to_vec();
        result.extend(decomp);
        return Ok(result);
    }

    // No bzip2 compression detected — return stripped data as-is.
    Ok(raw.to_vec())
}

/// Find the byte offset where the NEXRAD binary starts, skipping the WMO text
/// preamble (if present).  The preamble ends at the second "\r\r\n" sequence.
fn find_nids_start(raw: &[u8]) -> usize {
    let limit = raw.len().min(80);
    let mut count = 0usize;
    let mut i = 0;
    while i + 2 < limit {
        if raw[i] == b'\r' && raw[i + 1] == b'\r' && raw[i + 2] == b'\n' {
            count += 1;
            if count == 2 {
                return i + 3;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    0 // No preamble found — assume data starts at offset 0
}

/// Parse the NIDS message/product description block header, then read the
/// symbology block and Packet Code 0x10 header to get num_bins and num_radials.
///
/// All offsets are relative to the start of the preprocessed buffer (i.e.,
/// after the WMO preamble is stripped and bzip2 is decompressed).
///
/// Layout after preprocessing:
///   [0..18]   Message Header Block
///     [0..2]   HW1: Message Code (= product code for L3 products)
///   [18..120]  Product Description Block
///     [30..32]  HW16 (PDB+12): Product Code
///     [34..36]  HW18 (PDB+16): VCP
///   [120..]    Symbology Block (decompressed)
///     [120..122] Divider = -1
///     [122..124] Block ID = 1
///     [124..128] Block Length (u32)
///     [128..130] Num Layers (u16)
///     [130..132] Layer Divider = -1
///     [132..136] Layer Length (u32)
///     [136..138] Packet Code = 0x0010
///     [138..140] Index of First Range Bin (i16)
///     [140..142] Num Range Bins (u16)
///     [142..144] I Center (i16, not used)
///     [144..146] J Center (i16, not used)
///     [146..148] Scale Factor (u16, not used)
///     [148..150] Num Radials (u16)
///   [150..]    Per-radial records
fn parse_nids_header(cursor: &mut Cursor<&[u8]>) -> Result<NidsHeader> {
    // Product code: from Message Header HW1 at offset 0.
    cursor.seek(SeekFrom::Start(0))?;
    let product_code = cursor.read_u16::<BigEndian>()?;

    // VCP: from Product Description Block at offset 34 (PDB start=18, VCP at PDB+16=34).
    // Volume scan date at PDB+22 = stripped offset 40, time at PDB+24 = offset 42 (4 bytes).
    cursor.seek(SeekFrom::Start(34))?;
    let vcp = cursor.read_u16::<BigEndian>()?;
    cursor.seek(SeekFrom::Start(40))?;
    let scan_date = cursor.read_u16::<BigEndian>()?;
    let scan_time_secs = cursor.read_u32::<BigEndian>()?;

    // Symbology block starts at offset 120.
    cursor.seek(SeekFrom::Start(120))?;

    let symb_divider = cursor.read_i16::<BigEndian>()?;
    anyhow::ensure!(
        symb_divider == -1,
        "Expected symbology block divider (-1), got {symb_divider}; \
         data may be corrupted or use an unsupported format"
    );
    let _block_id = cursor.read_u16::<BigEndian>()?; // = 1
    let _block_len = cursor.read_u32::<BigEndian>()?;
    let _num_layers = cursor.read_u16::<BigEndian>()?; // = 1

    // Layer header (6 bytes)
    let _layer_div = cursor.read_i16::<BigEndian>()?;
    let _layer_len = cursor.read_u32::<BigEndian>()?;

    // Packet header: either
    // - 0x0010 Digital Radial Data Array
    // - 0xAF1F SRM variant of digital radial array (same header fields)
    // - 0xBA07 Digital Raster Data Array (used by NCR/NCZ and some derived products)
    let packet_code = cursor.read_u16::<BigEndian>()?;
    let (num_range_bins, num_radials, data_offset, is_raster) = match packet_code {
        0x0010 | 0xAF1F => {
            let _first_bin = cursor.read_i16::<BigEndian>()?;
            let num_range_bins = cursor.read_u16::<BigEndian>()?;
            cursor.seek(SeekFrom::Current(6))?; // skip I center, J center, scale factor
            let num_radials = cursor.read_u16::<BigEndian>()?;
            (num_range_bins, num_radials, cursor.position(), false)
        }
        0xBA07 => {
            // Raster packet layout: after packet code there are 16 bytes of
            // coordinate/scale metadata before row count + packing descriptor.
            // (wX skips to this same row-count field in NexradDecodeFourBit.raster)
            cursor.seek(SeekFrom::Current(16))?;
            let num_rows = cursor.read_u16::<BigEndian>()?;
            let _packing = cursor.read_u16::<BigEndian>()?;
            (num_rows, num_rows, cursor.position(), true)
        }
        _ => anyhow::bail!(
            "Unsupported packet code {packet_code:#06x} for product code {}; \
             this product is not map-radial/raster and should use a specialized view",
            product_code
        ),
    };

    Ok(NidsHeader {
        product_code,
        vcp,
        num_radials,
        num_range_bins,
        data_offset,
        packet_code,
        is_raster,
        scan_date,
        scan_time_secs,
    })
}

/// Decode 8-bit radial data from cursor position.
fn decode_radials(
    cursor: &mut Cursor<&[u8]>,
    hdr: &NidsHeader,
    bin_size: f32,
) -> Result<Level3Data> {
    let num_radials = hdr.num_radials as usize;
    let num_bins = hdr.num_range_bins as usize;

    let mut azimuths = Vec::with_capacity(num_radials);
    let mut bins = vec![0u8; num_radials * num_bins];

    for r in 0..num_radials {
        let num_halfwords = cursor.read_u16::<BigEndian>()? as usize;
        let raw_angle = cursor.read_u16::<BigEndian>()?;
        cursor.seek(SeekFrom::Current(2))?; // delta azimuth

        // Raw angle is in tenths of degrees, clockwise from North — the standard
        // radar azimuth convention expected by the geometry renderer.
        let azimuth = raw_angle as f32 / 10.0;
        azimuths.push(azimuth);

        let bin_slice = &mut bins[r * num_bins..(r + 1) * num_bins];
        let read_count = num_halfwords.min(num_bins);
        for slot in bin_slice.iter_mut().take(read_count) {
            *slot = cursor.read_u8()?;
        }

        // Skip extra bytes if num_halfwords > num_bins to keep stream aligned.
        if num_halfwords > read_count {
            cursor.seek(SeekFrom::Current((num_halfwords - read_count) as i64))?;
        }
    }

    Ok(Level3Data {
        num_radials,
        num_range_bins: num_bins,
        azimuths,
        bins,
        bin_size_km: bin_size,
        is_raster: false,
        product_code: hdr.product_code,
        vcp: hdr.vcp,
        scan_date: hdr.scan_date,
        scan_time_secs: hdr.scan_time_secs,
    })
}

/// Decode legacy 4-bit RLE radial packet (0xAF1F), used by SRM family products.
///
/// Packet layout per-radial:
///   u16 number_of_rle_halfwords
///   u16 start_angle_tenths
///   u16 delta_angle_tenths
///   [number_of_rle_halfwords * 2] bytes of 4-bit RLE words:
///      high nibble = run length, low nibble = level
fn decode_radials_4bit_rle(
    cursor: &mut Cursor<&[u8]>,
    hdr: &NidsHeader,
    bin_size: f32,
) -> Result<Level3Data> {
    let max_radials = hdr.num_radials as usize;
    let num_bins = hdr.num_range_bins as usize;
    let mut azimuths = Vec::with_capacity(max_radials);
    let mut bins = vec![0u8; max_radials * num_bins];
    let total_len = cursor.get_ref().len() as u64;
    let mut decoded_radials = 0usize;

    for r in 0..max_radials {
        if cursor.position() + 6 > total_len {
            break;
        }
        let num_rle_halfwords = cursor.read_u16::<BigEndian>()? as usize;
        let raw_angle = cursor.read_u16::<BigEndian>()?;
        cursor.seek(SeekFrom::Current(2))?; // delta azimuth
        let azimuth = raw_angle as f32 / 10.0;
        azimuths.push(azimuth);

        let row = &mut bins[r * num_bins..(r + 1) * num_bins];
        let mut col = 0usize;
        let words = num_rle_halfwords.saturating_mul(2);
        for _ in 0..words {
            if cursor.position() >= total_len {
                break;
            }
            let packed = cursor.read_u8()?;
            let run = (packed >> 4) as usize;
            let level = packed & 0x0F;
            // Expand 4-bit class level into 8-bit space for palette lookup.
            let gate = if level == 0 {
                0
            } else {
                level.saturating_mul(17)
            };
            for _ in 0..run {
                if col < num_bins {
                    row[col] = gate;
                }
                col += 1;
            }
        }
        decoded_radials += 1;
    }

    bins.truncate(decoded_radials.saturating_mul(num_bins));
    Ok(Level3Data {
        num_radials: decoded_radials,
        num_range_bins: num_bins,
        azimuths,
        bins,
        bin_size_km: bin_size,
        is_raster: false,
        product_code: hdr.product_code,
        vcp: hdr.vcp,
        scan_date: hdr.scan_date,
        scan_time_secs: hdr.scan_time_secs,
    })
}

/// Decode 4-bit run-length encoded raster packet (0xBA07).
fn decode_raster(
    cursor: &mut Cursor<&[u8]>,
    hdr: &NidsHeader,
    bin_size: f32,
) -> Result<Level3Data> {
    anyhow::ensure!(
        hdr.packet_code == 0xBA07,
        "decode_raster called for non-raster packet {:#06x}",
        hdr.packet_code
    );

    let rows = hdr.num_radials as usize;
    let cols = hdr.num_range_bins as usize;
    let mut bins = vec![0u8; rows * cols];

    for row in 0..rows {
        let row_bytes = cursor.read_u16::<BigEndian>()? as usize;
        let mut col = 0usize;
        for _ in 0..row_bytes {
            let packed = cursor.read_u8()?;
            let run = (packed >> 4) as usize;
            let level = packed & 0x0F;
            // BA07 raster products are 4-bit class levels. Expand to 8-bit gate
            // space so the existing 0..255 palettes apply correctly.
            let gate = if level == 0 {
                0
            } else {
                level.saturating_mul(17)
            };
            for _ in 0..run {
                if col >= cols {
                    break;
                }
                bins[row * cols + col] = gate;
                col += 1;
            }
        }
    }

    Ok(Level3Data {
        num_radials: rows,
        num_range_bins: cols,
        azimuths: Vec::new(),
        bins,
        bin_size_km: bin_size,
        is_raster: true,
        product_code: hdr.product_code,
        vcp: hdr.vcp,
        scan_date: hdr.scan_date,
        scan_time_secs: hdr.scan_time_secs,
    })
}

// ── Radial geometry generation ────────────────────────────────────────────────

/// Output of the geometry builder — ready to upload to GPU vertex/color buffers.
#[derive(Debug, Clone)]
pub struct RadialGeometry {
    /// Interleaved (x0,y0, x1,y1, x2,y2, x3,y3) quad vertices for each run.
    /// Each "run" is a contiguous run of the same color-level gate values.
    pub vertices: Vec<f32>,
    /// One (R,G,B) triplet per vertex → 4 vertices × 3 colors = 12 bytes per run.
    pub colors: Vec<u8>,
    /// Total number of quads (= total_bins in wX terminology).
    pub quad_count: usize,
}

/// Generate radial quad geometry from decoded radial data.
///
/// Implements the same algorithm as `NexradDecodeEightBit.createRadials()`.
/// The geometry is in radar-relative coordinates where 1.0 = one bin-size unit.
/// The caller scales by `bin_size_km / some_reference` to get screen units.
///
/// `color_fn` — maps a gate value (0–255) to (R, G, B).
/// `black_hole_start` — minimum range (in bin-units) to skip (center blank zone).
pub fn generate_radial_geometry(
    data: &Level3Data,
    color_fn: impl Fn(u8) -> (u8, u8, u8),
    black_hole_start: f32,
    black_hole_add: f32,
) -> RadialGeometry {
    let mut vertices: Vec<f32> = Vec::new();
    let mut colors: Vec<u8> = Vec::new();
    let mut quad_count = 0usize;

    for r in 0..data.num_radials {
        let angle = data.azimuths[r];
        let angle_v = data.azimuths[(r + 1) % data.num_radials];

        let angle_rad = angle.to_radians();
        let angle_v_rad = angle_v.to_radians();

        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();
        let cos_v = angle_v_rad.cos();
        let sin_v = angle_v_rad.sin();

        let bin_slice = &data.bins[r * data.num_range_bins..(r + 1) * data.num_range_bins];

        let mut level = bin_slice[0];
        let mut level_count = 0usize;
        let mut bin_start = black_hole_start;

        for (b, &cur_level) in bin_slice.iter().enumerate() {
            if cur_level == level {
                level_count += 1;
            } else {
                emit_quad(
                    &mut vertices,
                    &mut colors,
                    &mut quad_count,
                    bin_start,
                    level_count as f32 * data.bin_size_km,
                    cos_a,
                    sin_a,
                    cos_v,
                    sin_v,
                    level,
                    &color_fn,
                );
                level = cur_level;
                bin_start = b as f32 * data.bin_size_km + black_hole_add;
                level_count = 1;
            }
        }
        // Emit last run
        if level_count > 0 {
            emit_quad(
                &mut vertices,
                &mut colors,
                &mut quad_count,
                bin_start,
                level_count as f32 * data.bin_size_km,
                cos_a,
                sin_a,
                cos_v,
                sin_v,
                level,
                &color_fn,
            );
        }
    }

    RadialGeometry {
        vertices,
        colors,
        quad_count,
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn emit_quad(
    vertices: &mut Vec<f32>,
    colors: &mut Vec<u8>,
    count: &mut usize,
    bin_start: f32,
    extent: f32,
    cos_a: f32,
    sin_a: f32,
    cos_v: f32,
    sin_v: f32,
    level: u8,
    color_fn: &impl Fn(u8) -> (u8, u8, u8),
) {
    let r0 = bin_start;
    let r1 = bin_start + extent;

    // 4 corners of the radial quad:
    // v0: near edge, next radial side
    // v1: far edge, next radial side
    // v2: far edge, current radial side
    // v3: near edge, current radial side
    vertices.push(r0 * cos_v);
    vertices.push(r0 * sin_v);
    vertices.push(r1 * cos_v);
    vertices.push(r1 * sin_v);
    vertices.push(r1 * cos_a);
    vertices.push(r1 * sin_a);
    vertices.push(r0 * cos_a);
    vertices.push(r0 * sin_a);

    let (cr, cg, cb) = color_fn(level);
    for _ in 0..4 {
        colors.push(cr);
        colors.push(cg);
        colors.push(cb);
    }
    *count += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_size_legacy_reflectivity_and_velocity() {
        // Product codes 94 (N0Q) and 99 (N0U) use 1 nautical mile bins.
        assert_eq!(bin_size_for_product(94), 1.852);
        assert_eq!(bin_size_for_product(99), 1.852);
    }

    #[test]
    fn bin_size_raster_products() {
        assert_eq!(bin_size_for_product(37), 2.0); // NCR
        assert_eq!(bin_size_for_product(38), 8.0); // NCZ
        assert_eq!(bin_size_for_product(57), 8.0); // VIL
    }

    #[test]
    fn bin_size_super_res_products() {
        assert_eq!(bin_size_for_product(186), 0.590022); // N0B (super-res ref)
        assert_eq!(bin_size_for_product(159), 0.50); // dual-pol
        assert_eq!(bin_size_for_product(180), 0.295011); // TAB
    }

    #[test]
    fn bin_size_qpe() {
        assert_eq!(bin_size_for_product(78), 4.0);
        assert_eq!(bin_size_for_product(80), 4.0);
    }

    #[test]
    fn bin_size_unknown_falls_back_to_default() {
        assert_eq!(bin_size_for_product(0), 1.852);
        assert_eq!(bin_size_for_product(999), 1.852);
    }
}
