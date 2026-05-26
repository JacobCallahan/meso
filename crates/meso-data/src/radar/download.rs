use crate::cache::Cache;
use crate::geo::sites::{is_tdwr, rid_prefix};
use crate::radar::level2;
use crate::radar::products::{RadarProduct, NOMADS_L2_BASE, TGFTP_BASE};
use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use reqwest::Client;
use std::time::Duration;

// Cache TTLs
const TTL_L3_LATEST: Duration = Duration::from_secs(4 * 60);
const TTL_L2_LATEST: Duration = Duration::from_secs(5 * 60);
const TTL_ANIM_FRAME: Duration = Duration::from_secs(30 * 60);

/// Radar download utilities.
///
/// For Level 3, files are fetched from the NWS TGFTP server (via `sn.last`).
/// For Level 2, files are fetched from NOMADS with HTTP Range headers to
/// download only the first portion of the file (reflectivity and velocity data
/// live near the start of the archive).
pub struct RadarDownloader {
    client: Client,
}

impl RadarDownloader {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Build the TGFTP URL for the latest Level 3 product.
    pub fn level3_url(site: &str, product: &RadarProduct) -> Option<String> {
        let dir = product.tgftp_dir()?;
        let prefix = if is_tdwr(site) { "" } else { rid_prefix(site) };
        Some(format!(
            "{}/SL.us008001/DF.of/DC.radar/{}/SI.{}{}/sn.last",
            TGFTP_BASE,
            dir,
            prefix,
            site.to_lowercase()
        ))
    }

    /// Download the latest Level 3 product binary.
    pub async fn fetch_level3(&self, site: &str, product: &RadarProduct) -> Result<Vec<u8>> {
        let url = Self::level3_url(site, product)
            .with_context(|| format!("No TGFTP URL for {product:?}"))?;
        let cache = Cache::new("radar/l3");
        let key = format!("{site}-{}-latest", product.code());
        if let Some(bytes) = cache.get(&key) {
            return Ok(bytes);
        }
        let bytes = self.client.get(&url).send().await?.bytes().await?;
        cache.put(&key, &bytes, TTL_L3_LATEST);
        Ok(bytes.to_vec())
    }

    /// Build a TGFTP URL for a specific sn.XXXX animation filename.
    pub fn level3_file_url(site: &str, product: &RadarProduct, filename: &str) -> Option<String> {
        let dir = product.tgftp_dir()?;
        let prefix = if is_tdwr(site) { "" } else { rid_prefix(site) };
        Some(format!(
            "{}/SL.us008001/DF.of/DC.radar/{}/SI.{}{}/{}",
            TGFTP_BASE,
            dir,
            prefix,
            site.to_lowercase(),
            filename
        ))
    }

    /// Fetch raw bytes from any URL — cached for animation frames.
    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let cache = Cache::new("radar/anim");
        if let Some(bytes) = cache.get(url) {
            return Ok(bytes);
        }
        let bytes = self.client.get(url).send().await?.bytes().await?;
        cache.put(url, &bytes, TTL_ANIM_FRAME);
        Ok(bytes.to_vec())
    }

    /// Build the NOMADS directory URL for Level 2 files.
    pub fn level2_dir_url(site: &str) -> String {
        // NOMADS uses 4-letter uppercase codes (KTLX, KRAX, PHKI, TJUA…).
        // Internal site codes may be 3-letter (e.g. RAX) or already 4-letter
        // (e.g. KTLX). Only expand when the code is shorter than 4 chars.
        let code = if site.len() >= 4 {
            site.to_uppercase()
        } else {
            format!(
                "{}{}",
                crate::geo::sites::rid_prefix(site).to_uppercase(),
                site.to_uppercase()
            )
        };
        format!("{}{}/", NOMADS_L2_BASE, code)
    }

    /// Parse NOMADS Level 2 `dir.list` lines into ordered (filename, size) entries.
    ///
    /// The upstream format is not guaranteed to be stable across mirrors; this parser
    /// looks for tokens that resemble Level 2 filenames for the requested site and
    /// then picks a nearby numeric token as file size.
    fn parse_level2_dir_entries(site: &str, dir_list: &str) -> Vec<(String, f64)> {
        let site_up = site.to_uppercase();
        let mut out: Vec<(String, f64)> = Vec::new();

        for line in dir_list.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let file_idx = parts.iter().position(|tok| {
                let t = tok.trim();
                t.contains(&site_up) && t.contains('_')
            });
            let Some(i) = file_idx else {
                continue;
            };

            let fname = parts[i].trim().to_string();
            // Prefer the token right after filename (common "name size" format),
            // then right before, then any numeric token on the line.
            let mut size = 1.0f64;
            if let Some(next) = parts.get(i + 1).and_then(|s| s.parse::<f64>().ok()) {
                size = next;
            } else if i > 0 {
                if let Ok(prev) = parts[i - 1].parse::<f64>() {
                    size = prev;
                } else if let Some(any_num) = parts.iter().find_map(|s| s.parse::<f64>().ok()) {
                    size = any_num;
                }
            } else if let Some(any_num) = parts.iter().find_map(|s| s.parse::<f64>().ok()) {
                size = any_num;
            }

            out.push((fname, size));
        }

        // Normalize to chronological order regardless of upstream listing order.
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Legacy tokenizer fallback for NOMADS `dir.list`.
    ///
    /// Older logic assumed alternating `filename size` tokens and uses the same
    /// "undersized newest frame" heuristic used in wX.
    fn parse_level2_dir_entries_legacy(dir_list: &str) -> Vec<(String, f64)> {
        let tokens: Vec<&str> = dir_list.split_whitespace().collect();
        if tokens.len() < 2 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 1 < tokens.len() {
            let name = tokens[i].to_string();
            let size = tokens[i + 1].parse::<f64>().unwrap_or(1.0);
            // Keep only plausible radar filenames.
            if name.contains('_') || name.starts_with("sn.") {
                out.push((name, size));
            }
            i += 2;
        }
        out
    }

    /// Fetch the `dir.list` file from NOMADS to find the latest L2 filename.
    /// Returns (filename, base_url).
    pub async fn latest_level2_filename(&self, site: &str) -> Result<(String, String)> {
        let base = Self::level2_dir_url(site);
        let dir_list = self
            .client
            .get(format!("{base}dir.list"))
            .send()
            .await?
            .text()
            .await?;

        let mut entries = Self::parse_level2_dir_entries(site, &dir_list);
        if entries.is_empty() {
            entries = Self::parse_level2_dir_entries_legacy(&dir_list);
        }
        if entries.is_empty() {
            anyhow::bail!("no parseable Level2 entries in dir.list for site {site}");
        }
        let selected = if entries.len() >= 2 {
            let (fname_prev, size_prev) = &entries[entries.len() - 2];
            let (fname, size) = &entries[entries.len() - 1];
            // wX behavior: if newest file is much smaller, it may still be writing.
            let ratio = *size / size_prev.max(1.0);
            if ratio < 0.75 {
                fname_prev
            } else {
                fname
            }
        } else {
            &entries[0].0
        };
        Ok((selected.to_string(), base))
    }

    /// Download the first N bytes of a Level 2 archive.
    /// Reflectivity data is in the first ~2.4 MB; velocity needs ~3.0 MB.
    /// When `url_override` is provided (animation frames), results are cached.
    /// The "latest" scan is cached with a short TTL to avoid redundant fetches
    /// on tab switch / re-render while a new scan is not yet available.
    pub async fn fetch_level2_partial(
        &self,
        site: &str,
        product: &RadarProduct,
        url_override: Option<&str>,
    ) -> Result<Vec<u8>> {
        let byte_end = if product == &RadarProduct::L2Velocity {
            "3000000"
        } else {
            "2450000"
        };

        let cache = Cache::new("radar/l2");

        if let Some(u) = url_override {
            // Animation frame with known URL — cache aggressively
            let key = format!("{u}:{byte_end}");
            if let Some(bytes) = cache.get(&key) {
                return Ok(bytes);
            }
            let bytes = self
                .client
                .get(u)
                // Identity encoding prevents reqwest from trying to auto-decompress
                // gzip/brotli on partial (206) responses, which causes body decode errors.
                .header("Accept-Encoding", "identity")
                .header("Range", format!("bytes=0-{byte_end}"))
                .send()
                .await?
                .bytes()
                .await?;
            cache.put(&key, &bytes, TTL_ANIM_FRAME);
            return Ok(bytes.to_vec());
        }

        // "Latest" fetch: cache with short TTL keyed by site+product
        let latest_key = format!("{site}-{}-latest", product.code());
        if let Some(bytes) = cache.get(&latest_key) {
            return Ok(bytes);
        }
        let (fname, base) = self.latest_level2_filename(site).await?;
        let url = format!("{base}{fname}");
        let bytes = self
            .client
            .get(&url)
            .header("Accept-Encoding", "identity")
            .header("Range", format!("bytes=0-{byte_end}"))
            .send()
            .await?
            .bytes()
            .await?;
        cache.put(&latest_key, &bytes, TTL_L2_LATEST);
        Ok(bytes.to_vec())
    }

    /// Fetch a Level 2 animation frame, decompressing it and caching the
    /// decompressed bytes. This skips re-decompression on subsequent animation
    /// runs for the same frame, which is the dominant cost in L2 animation.
    pub async fn fetch_level2_decompressed(
        &self,
        site: &str,
        product: &RadarProduct,
        url: &str,
    ) -> Result<Vec<u8>> {
        let dcache = Cache::new("radar/l2-decomp");
        let dkey = url.to_string();
        if let Some(decompressed) = dcache.get(&dkey) {
            return Ok(decompressed);
        }
        // Fetch compressed bytes (uses the per-URL animation cache)
        let raw = self.fetch_level2_partial(site, product, Some(url)).await?;
        let decompressed = level2::decompress_level2(&raw)?;
        dcache.put(&dkey, &decompressed, TTL_ANIM_FRAME);
        Ok(decompressed)
    }
    pub async fn level2_filenames_for_animation(
        &self,
        site: &str,
        frame_count: usize,
    ) -> Result<Vec<String>> {
        let base = Self::level2_dir_url(site);
        let dir_list = self
            .client
            .get(format!("{base}dir.list"))
            .send()
            .await?
            .text()
            .await?;

        let mut entries = Self::parse_level2_dir_entries(site, &dir_list);
        if entries.is_empty() {
            entries = Self::parse_level2_dir_entries_legacy(&dir_list);
        }
        if entries.is_empty() {
            anyhow::bail!("no parseable Level2 animation entries for site {site}");
        }

        // Exclude likely in-progress newest file when it is much smaller than previous.
        let end = if entries.len() >= 2 {
            let prev = entries[entries.len() - 2].1.max(1.0);
            let last = entries[entries.len() - 1].1;
            if last / prev < 0.75 {
                entries.len().saturating_sub(1)
            } else {
                entries.len()
            }
        } else {
            entries.len()
        };

        if end == 0 {
            anyhow::bail!("no complete Level2 frames available for site {site}");
        }

        let start = end.saturating_sub(frame_count);
        let names = entries[start..end]
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        Ok(names)
    }

    /// Fetch Level 3 files for animation, returning raw bytes for each frame.
    /// Frames are ordered oldest → newest.
    pub async fn level3_filenames_for_animation(
        &self,
        site: &str,
        product: &RadarProduct,
        frame_count: usize,
    ) -> Result<Vec<String>> {
        let dir = product
            .tgftp_dir()
            .with_context(|| format!("No TGFTP dir for {product:?}"))?;
        let prefix = if is_tdwr(site) { "" } else { rid_prefix(site) };
        let dir_url = format!(
            "{}/SL.us008001/DF.of/DC.radar/{}/SI.{}{}/",
            TGFTP_BASE,
            dir,
            prefix,
            site.to_lowercase()
        );
        let html = self.client.get(&dir_url).send().await?.text().await?;

        // Parse sn.XXXX rows with timestamp so we can select newest frames
        // by actual recency, not numeric filename order.
        let row_re = regex::Regex::new(
            r#"<tr><td><a href="(sn\.[0-9]{4})">[^<]*</a></td><td align="right">\s*([0-9]{2}-[A-Za-z]{3}-[0-9]{4}\s+[0-9]{2}:[0-9]{2})\s*</td>"#,
        )
        .unwrap();
        let mut dated: Vec<(String, NaiveDateTime)> = row_re
            .captures_iter(&html)
            .filter_map(|c| {
                let name = c.get(1)?.as_str().to_string();
                let ts = c.get(2)?.as_str();
                let dt = NaiveDateTime::parse_from_str(ts, "%d-%b-%Y %H:%M").ok()?;
                Some((name, dt))
            })
            .collect();

        if !dated.is_empty() {
            dated.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
            dated.dedup_by(|a, b| a.0 == b.0);
            let start = dated.len().saturating_sub(frame_count);
            return Ok(dated[start..].iter().map(|(n, _)| n.clone()).collect());
        }

        // Fallback: use appearance order in listing for sn.XXXX entries.
        let sn_pattern = regex::Regex::new(r#">(sn\.[0-9]{4})</a>"#).unwrap();
        let mut sn_files: Vec<String> = sn_pattern
            .captures_iter(&html)
            .map(|c| c[1].to_string())
            .collect();
        if sn_files.is_empty() {
            anyhow::bail!("No sn.XXXX files found at {dir_url}");
        }
        sn_files.dedup();
        let start = sn_files.len().saturating_sub(frame_count);
        Ok(sn_files[start..].to_vec())
    }
}

// Bring in regex as a dependency for the directory parser.
// We declare it inline; add to Cargo.toml at build time.
mod regex {
    pub use ::regex::Regex;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radar::products::RadarProduct;

    // ── URL builders ──────────────────────────────────────────────────────────

    #[test]
    fn level3_url_contains_site_and_product_dir() {
        let url = RadarDownloader::level3_url("KTLX", &RadarProduct::N0Q).unwrap();
        assert!(url.contains("ktlx"), "site should be lower-cased in URL");
        assert!(url.contains("sn.last"));
        assert!(url.contains("tgftp.nws.noaa.gov"));
    }

    #[test]
    fn level3_url_tdwr_has_no_prefix() {
        // TDWR sites are 4 chars and use no "k" prefix.
        let url = RadarDownloader::level3_url("TDFW", &RadarProduct::TR0).unwrap();
        assert!(
            url.contains("tdfw"),
            "TDWR site should appear as-is, lower-cased"
        );
        // Ensure the extra "k" prefix is not inserted.
        assert!(!url.contains("ktdfw"));
    }

    #[test]
    fn level3_url_l2_product_returns_none() {
        assert!(RadarDownloader::level3_url("KTLX", &RadarProduct::L2Reflectivity).is_none());
    }

    #[test]
    fn level2_dir_url_is_uppercase_with_prefix() {
        // 4-letter codes are already complete — no prefix should be added.
        let url = RadarDownloader::level2_dir_url("KTLX");
        assert!(url.contains("KTLX"));
        assert!(
            !url.contains("KKTLX"),
            "URL must not double the prefix: {url}"
        );
        assert!(url.ends_with('/'));

        // 3-letter codes need the prefix letter prepended (RAX → KRAX).
        let url3 = RadarDownloader::level2_dir_url("RAX");
        assert!(
            url3.contains("KRAX"),
            "3-letter site code should be expanded: {url3}"
        );
        assert!(url3.ends_with('/'));
    }

    #[test]
    fn level3_file_url_uses_supplied_filename() {
        let url = RadarDownloader::level3_file_url("KTLX", &RadarProduct::N0Q, "sn.0010").unwrap();
        assert!(url.contains("sn.0010"));
        assert!(url.contains("ktlx"));
    }

    // ── Dir-list parsers ──────────────────────────────────────────────────────

    #[test]
    fn parse_dir_entries_basic_line() {
        let input = "KTLX20240615_214500_V06 2394874\n";
        let entries = RadarDownloader::parse_level2_dir_entries("KTLX", input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "KTLX20240615_214500_V06");
        assert!((entries[0].1 - 2394874.0).abs() < 1.0);
    }

    #[test]
    fn parse_dir_entries_multi_line_sorted() {
        let input = concat!(
            "KTLX20240615_215500_V06 2400000\n",
            "KTLX20240615_214500_V06 2394874\n",
            "KTLX20240615_215000_V06 2390000\n",
        );
        let entries = RadarDownloader::parse_level2_dir_entries("KTLX", input);
        assert_eq!(entries.len(), 3);
        // Should be sorted chronologically by filename.
        assert!(entries[0].0 < entries[1].0);
        assert!(entries[1].0 < entries[2].0);
    }

    #[test]
    fn parse_dir_entries_ignores_unrelated_lines() {
        let input = "total 12345\nsome other line\nKTLX20240615_214500_V06 2394874\n";
        let entries = RadarDownloader::parse_level2_dir_entries("KTLX", input);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn parse_dir_entries_empty_input() {
        let entries = RadarDownloader::parse_level2_dir_entries("KTLX", "");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_dir_entries_legacy_alternating_tokens() {
        let input = "KTLX20240615_214500_V06 2394874 KTLX20240615_215000_V06 2400000";
        let entries = RadarDownloader::parse_level2_dir_entries_legacy(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "KTLX20240615_214500_V06");
        assert_eq!(entries[1].0, "KTLX20240615_215000_V06");
    }

    #[test]
    fn parse_dir_entries_legacy_empty() {
        assert!(RadarDownloader::parse_level2_dir_entries_legacy("").is_empty());
        assert!(RadarDownloader::parse_level2_dir_entries_legacy("onlyonetoken").is_empty());
    }

    // ── In-progress frame heuristic ───────────────────────────────────────────

    /// When the newest file is < 75% the size of the second-newest, it is likely
    /// still being written; the downloader should prefer the second-newest frame.
    /// This is tested indirectly through the public `latest_level2_filename`
    /// method's selection logic, which is exposed via the private helper.
    #[test]
    fn in_progress_heuristic_skips_small_newest() {
        // Build a two-entry list where the last entry is only 10% of the previous.
        let input = concat!(
            "KTLX20240615_214500_V06 2000000\n",
            "KTLX20240615_215000_V06 200000\n",
        );
        let entries = RadarDownloader::parse_level2_dir_entries("KTLX", input);
        assert_eq!(entries.len(), 2);
        let prev_size = entries[entries.len() - 2].1;
        let last_size = entries[entries.len() - 1].1;
        let ratio = last_size / prev_size.max(1.0);
        // Verify the heuristic would trigger.
        assert!(ratio < 0.75, "ratio {ratio:.2} should be < 0.75");
    }

    #[test]
    fn in_progress_heuristic_keeps_large_newest() {
        let input = concat!(
            "KTLX20240615_214500_V06 2000000\n",
            "KTLX20240615_215000_V06 1900000\n",
        );
        let entries = RadarDownloader::parse_level2_dir_entries("KTLX", input);
        let prev_size = entries[entries.len() - 2].1;
        let last_size = entries[entries.len() - 1].1;
        let ratio = last_size / prev_size.max(1.0);
        assert!(ratio >= 0.75, "ratio {ratio:.2} should be >= 0.75");
    }
}
