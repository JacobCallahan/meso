/*
 * NEXRAD Level 2 (WSR-88D) binary decoder.
 *
 * Ported from wX's Level2.kt and Level2Record.kt, which are derived from
 * UCAR/Unidata's Level2VolumeScan.java (© 1998–2009 UCAR, used under their
 * open-source terms).  This Rust implementation is independently rewritten.
 *
 * Decodes message types 1 (legacy) and 31 (high-resolution super-res) to
 * extract 720 radials of reflectivity or velocity gate data at 916 bins each.
 */

use anyhow::{bail, Context, Result};
use byteorder::{BigEndian, ReadBytesExt};
use bzip2::read::BzDecoder;
use std::io::{Cursor, Read, Seek, SeekFrom};

// ── Constants ────────────────────────────────────────────────────────────────

const FILE_HEADER_SIZE: u64 = 24;
const RADAR_DATA_SIZE: u64 = 2432;
const CTM_HEADER_SIZE: i64 = 12;
const MESSAGE_HEADER_SIZE: u64 = 28;
const REFLECTIVITY_HIGH: u8 = 5;
const VELOCITY_HIGH: u8 = 6;
const NUM_RADIALS: usize = 720;
pub const NUM_RANGE_BINS: usize = 916;

// ── Public output types ───────────────────────────────────────────────────────

/// The decoded output of a Level 2 radial scan.
#[derive(Debug, Clone)]
pub struct Level2Data {
    /// Start azimuth for each radial (degrees, 0–360).
    pub azimuths: Vec<f32>,
    /// Raw gate data: n_radials × NUM_RANGE_BINS bytes.
    pub bins: Vec<u8>,
    /// Julian date (days since 1 Jan 1970).
    pub julian_date: i16,
    /// Collection time in milliseconds since midnight UTC.
    pub millis: i32,
    /// Gate size in km.
    pub bin_size_km: f32,
    /// Number of radials decoded (may be < 720 on higher tilts).
    pub num_radials: usize,
    /// Elevation angle (degrees) for the decoded tilt.
    pub elevation_angle_deg: f32,
}

/// A single elevation tilt available in a Level 2 scan.
#[derive(Debug, Clone)]
pub struct TiltInfo {
    /// Sequential elevation number within the VCP (1-based).
    pub elevation_num: u8,
    /// Elevation angle in degrees above horizontal.
    pub angle_deg: f32,
}

impl Level2Data {
    /// Format the scan time as a UTC timestamp string, e.g. "2024-06-15 21:45 UTC".
    pub fn timestamp_str(&self) -> String {
        use chrono::{Local, TimeZone, Utc};
        // Level 2 julian_date is 1-indexed days from 1 Jan 1970.
        let days = self.julian_date as i64 - 1;
        let secs = days * 86400 + self.millis as i64 / 1000;
        match Utc.timestamp_opt(secs, 0).single() {
            Some(dt) => dt
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M %Z")
                .to_string(),
            None => format!("Day {} {:08}ms", self.julian_date, self.millis),
        }
    }
}

// ── BZ2 decompressor ─────────────────────────────────────────────────────────

/// Decompress a NEXRAD Level 2 file in-memory.
///
/// Level 2 archives consist of a 24-byte file header followed by a series
/// of BZ2-compressed records.  Each record is preceded by a 4-byte signed
/// big-endian integer whose absolute value is the compressed block length.
pub fn decompress_level2(compressed: &[u8]) -> Result<Vec<u8>> {
    let mut decompressed = Vec::new();
    // First 24 bytes are the uncompressed file header — copy as-is.
    if compressed.len() < FILE_HEADER_SIZE as usize {
        bail!("Level 2 file too short for header");
    }
    decompressed.extend_from_slice(&compressed[..FILE_HEADER_SIZE as usize]);

    let mut cursor = Cursor::new(compressed);
    cursor.seek(SeekFrom::Start(FILE_HEADER_SIZE))?;

    loop {
        // Each block starts with a signed i32 big-endian block size.
        let block_size = match cursor.read_i32::<BigEndian>() {
            Ok(v) => v,
            Err(_) => break,
        };
        let block_len = block_size.unsigned_abs() as usize;
        if block_len == 0 {
            break;
        }

        let pos = cursor.position() as usize;
        let end = pos + block_len;
        if end > compressed.len() {
            break;
        }

        let block = &compressed[pos..end];
        let mut decoder = BzDecoder::new(block);
        decoder
            .read_to_end(&mut decompressed)
            .context("BZ2 decompression failed")?;
        cursor.seek(SeekFrom::Start(end as u64))?;
    }

    Ok(decompressed)
}

// ── Main decode ──────────────────────────────────────────────────────────────

/// Decode a decompressed Level 2 binary into radial data.
///
/// `velocity`  — if true, decode velocity moment; otherwise reflectivity.
/// `tilt_idx`  — which elevation tilt to decode (0 = lowest).  If the index
///               exceeds the number of available tilts, the highest is used.
pub fn decode(data: &[u8], velocity: bool, tilt_idx: usize) -> Result<Level2Data> {
    let moment_type = if velocity {
        VELOCITY_HIGH
    } else {
        REFLECTIVITY_HIGH
    };

    let mut cursor = Cursor::new(data);
    let mut records_ref: Vec<RecordMeta> = Vec::new();
    let mut records_vel: Vec<RecordMeta> = Vec::new();
    let mut vcp_seen = false;
    let mut vcp: i16 = 0;
    let mut extra_offset: u64 = 0;
    let mut record_number: u64 = 0;

    loop {
        let msg_offset = record_number * RADAR_DATA_SIZE + FILE_HEADER_SIZE + extra_offset;
        if msg_offset >= data.len() as u64 {
            break;
        }
        cursor.seek(SeekFrom::Start(msg_offset))?;

        // CTM header
        cursor.seek(SeekFrom::Current(CTM_HEADER_SIZE))?;
        let message_size = cursor.read_i16::<BigEndian>().unwrap_or(0);
        cursor.seek(SeekFrom::Current(1))?; // channel
        let message_type = cursor.read_u8().unwrap_or(0);
        cursor.seek(SeekFrom::Current(12))?; // skip seq/date/time/seg counts

        if message_type == 31 {
            extra_offset += (message_size as i32 * 2 + 12 - 2432).max(0) as u64;
        }

        if message_type != 1 && message_type != 31 {
            record_number += 1;
            continue;
        }

        let meta = parse_record_meta(&mut cursor, message_type, msg_offset)?;

        if !vcp_seen {
            vcp = meta.vcp;
            vcp_seen = true;
        }

        if message_type == 31 {
            if meta.has_ref {
                records_ref.push(meta.clone());
            }
            if meta.has_vel {
                records_vel.push(meta);
            }
        }
        // message type 1 not needed for high-res decode

        record_number += 1;
    }

    let all_records = if velocity { &records_vel } else { &records_ref };
    if all_records.is_empty() {
        bail!("No {} records found", if velocity { "velocity" } else { "reflectivity" });
    }

    // Group records by elevation_num, preserving scan order
    let mut tilts: Vec<(i16, Vec<&RecordMeta>)> = Vec::new();
    for rec in all_records {
        if let Some(entry) = tilts.iter_mut().find(|(n, _)| *n == rec.elevation_num) {
            entry.1.push(rec);
        } else {
            tilts.push((rec.elevation_num, vec![rec]));
        }
    }
    // Sort by elevation_num ascending (lowest tilt first)
    tilts.sort_by_key(|(n, _)| *n);

    let tilt_idx = tilt_idx.min(tilts.len() - 1);
    let (_, records) = &tilts[tilt_idx];
    let elevation_angle_deg = records
        .first()
        .map(|r| r.elevation_angle_deg)
        .unwrap_or(0.0);

    let n_radials = records.len().min(NUM_RADIALS);
    let mut azimuths = Vec::with_capacity(n_radials);
    let mut bins = vec![0u8; n_radials * NUM_RANGE_BINS];
    let mut julian_date: i16 = 0;
    let mut millis: i32 = 0;

    for (i, rec) in records[..n_radials].iter().enumerate() {
        azimuths.push(rec.azimuth);
        if i == 1 {
            julian_date = rec.julian_date;
            millis = rec.millis;
        }

        let data_offset = if velocity {
            rec.velocity_offset
        } else {
            rec.reflect_offset
        };
        let abs_offset = rec.message_offset + MESSAGE_HEADER_SIZE + data_offset as u64;
        let slice = &mut bins[i * NUM_RANGE_BINS..(i + 1) * NUM_RANGE_BINS];
        let end = (abs_offset + NUM_RANGE_BINS as u64) as usize;
        if end <= data.len() {
            slice.copy_from_slice(&data[abs_offset as usize..end]);
        }
    }

    // Pad azimuths to n_radials if needed
    azimuths.resize(n_radials, 0.0);

    Ok(Level2Data {
        azimuths,
        bins,
        julian_date,
        millis,
        bin_size_km: 0.25,
        num_radials: n_radials,
        elevation_angle_deg,
    })
}

/// Return the list of available elevation tilts in a Level 2 scan.
///
/// `velocity` — if true, list tilts that have velocity data; otherwise reflectivity.
/// Returns tilts sorted by elevation number (lowest first).
pub fn list_tilts(data: &[u8], velocity: bool) -> Result<Vec<TiltInfo>> {
    let mut cursor = Cursor::new(data);
    let mut records_ref: Vec<RecordMeta> = Vec::new();
    let mut records_vel: Vec<RecordMeta> = Vec::new();
    let mut extra_offset: u64 = 0;
    let mut record_number: u64 = 0;

    loop {
        let msg_offset = record_number * RADAR_DATA_SIZE + FILE_HEADER_SIZE + extra_offset;
        if msg_offset >= data.len() as u64 {
            break;
        }
        cursor.seek(SeekFrom::Start(msg_offset))?;
        cursor.seek(SeekFrom::Current(CTM_HEADER_SIZE))?;
        let message_size = cursor.read_i16::<BigEndian>().unwrap_or(0);
        cursor.seek(SeekFrom::Current(1))?;
        let message_type = cursor.read_u8().unwrap_or(0);
        cursor.seek(SeekFrom::Current(12))?;

        if message_type == 31 {
            extra_offset += (message_size as i32 * 2 + 12 - 2432).max(0) as u64;
        }
        if message_type != 31 {
            record_number += 1;
            continue;
        }

        let meta = parse_record_meta(&mut cursor, message_type, msg_offset)?;
        if meta.has_ref {
            records_ref.push(meta.clone());
        }
        if meta.has_vel {
            records_vel.push(meta);
        }
        record_number += 1;
    }

    let records = if velocity { &records_vel } else { &records_ref };
    // Collect unique elevation_num values, preserving first occurrence's angle
    let mut seen: Vec<(i16, f32)> = Vec::new();
    for rec in records {
        if !seen.iter().any(|(n, _)| *n == rec.elevation_num) {
            seen.push((rec.elevation_num, rec.elevation_angle_deg));
        }
    }
    seen.sort_by_key(|(n, _)| *n);

    Ok(seen
        .into_iter()
        .map(|(elevation_num, angle_deg)| TiltInfo {
            elevation_num: elevation_num as u8,
            angle_deg,
        })
        .collect())
}

// ── Internal record parser ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct RecordMeta {
    message_offset: u64,
    message_type: u8,
    millis: i32,
    julian_date: i16,
    elevation_num: i16,
    elevation_angle_deg: f32,
    azimuth: f32,
    vcp: i16,
    has_ref: bool,
    has_vel: bool,
    reflect_offset: u32,
    velocity_offset: u32,
}

fn parse_record_meta(
    cursor: &mut Cursor<&[u8]>,
    message_type: u8,
    msg_offset: u64,
) -> Result<RecordMeta> {
    let mut meta = RecordMeta {
        message_offset: msg_offset,
        message_type,
        millis: 0,
        julian_date: 0,
        elevation_num: 0,
        elevation_angle_deg: 0.0,
        azimuth: 0.0,
        vcp: 0,
        has_ref: false,
        has_vel: false,
        reflect_offset: 0,
        velocity_offset: 0,
    };

    if message_type == 1 {
        meta.millis = cursor.read_i32::<BigEndian>()?;
        meta.julian_date = cursor.read_i16::<BigEndian>()?;
        cursor.seek(SeekFrom::Current(10))?;
        meta.elevation_num = cursor.read_i16::<BigEndian>()?;
        cursor.seek(SeekFrom::Current(26))?;
        meta.vcp = cursor.read_i16::<BigEndian>()?;
    } else if message_type == 31 {
        cursor.seek(SeekFrom::Current(4))?; // ICAO
        meta.millis = cursor.read_i32::<BigEndian>()?;
        meta.julian_date = cursor.read_i16::<BigEndian>()?;
        cursor.seek(SeekFrom::Current(2))?;
        meta.azimuth = cursor.read_f32::<BigEndian>()?;
        cursor.seek(SeekFrom::Current(6))?;
        meta.elevation_num = cursor.read_u8()? as i16;
        cursor.seek(SeekFrom::Current(1))?; // cutSectorNum
        meta.elevation_angle_deg = cursor.read_f32::<BigEndian>()?;
        cursor.seek(SeekFrom::Current(4))?; // radialSpotBlanking(1) + azimuthIndexingValue(2) + dataBlockCount(2) – but 4 aligns to dbp1

        // Read data block pointers (dbp1–dbp9 in wX terminology)
        let dbp1 = cursor.read_u32::<BigEndian>()?;
        cursor.seek(SeekFrom::Current(8))?; // skip dbp2, dbp3
        let dbps = [
            cursor.read_u32::<BigEndian>()?,
            cursor.read_u32::<BigEndian>()?,
            cursor.read_u32::<BigEndian>()?,
            cursor.read_u32::<BigEndian>()?,
            cursor.read_u32::<BigEndian>()?,
            cursor.read_u32::<BigEndian>()?,
        ];

        // Read VCP from volume header block at dbp1
        if dbp1 > 0 {
            let vcp_pos = msg_offset + MESSAGE_HEADER_SIZE + dbp1 as u64 + 40;
            if (vcp_pos + 2) as usize <= cursor.get_ref().len() {
                cursor.seek(SeekFrom::Start(vcp_pos))?;
                meta.vcp = cursor.read_i16::<BigEndian>()?;
            }
        }

        // Identify REF/VEL data blocks
        for &dbp in &dbps {
            if dbp == 0 {
                continue;
            }
            let name_pos = msg_offset + MESSAGE_HEADER_SIZE + dbp as u64 + 1;
            if (name_pos + 3) as usize > cursor.get_ref().len() {
                continue;
            }
            cursor.seek(SeekFrom::Start(name_pos))?;
            let mut name = [0u8; 3];
            cursor.read_exact(&mut name)?;
            if &name == b"REF" {
                meta.has_ref = true;
                meta.reflect_offset = dbp + 28;
            } else if &name == b"VEL" {
                meta.has_vel = true;
                meta.velocity_offset = dbp + 28;
            }
        }
    }

    Ok(meta)
}
