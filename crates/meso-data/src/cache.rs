/*
 * Disk-based byte cache for fetched weather data.
 *
 * Cache files live in `~/.cache/meso/<namespace>/`.
 * Each entry is a file named by a URL-safe hash of its key string.
 * A sidecar `.ttl` file records the Unix expiry timestamp.
 *
 * Usage:
 *   let cache = Cache::new("radar");
 *   if let Some(bytes) = cache.get("KTLX-L2-latest") { ... }
 *   cache.put("KTLX-L2-latest", &bytes, Duration::from_secs(300));
 */

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    /// Create a cache namespace under `~/.cache/meso/<namespace>/`.
    pub fn new(namespace: &str) -> Self {
        let dir = cache_dir().join(namespace);
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    /// Retrieve cached bytes for `key` if they exist and have not expired.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let hash = hash_key(key);
        let data_path = self.dir.join(&hash);
        let ttl_path = self.dir.join(format!("{hash}.ttl"));

        // Check TTL
        if let Ok(ttl_str) = std::fs::read_to_string(&ttl_path) {
            let expiry: u64 = ttl_str.trim().parse().unwrap_or(0);
            let now = unix_now();
            if now > expiry {
                // Expired — remove both files
                let _ = std::fs::remove_file(&data_path);
                let _ = std::fs::remove_file(&ttl_path);
                return None;
            }
        } else {
            return None;
        }

        std::fs::read(&data_path).ok()
    }

    /// Store `bytes` for `key` with the given TTL duration.
    pub fn put(&self, key: &str, bytes: &[u8], ttl: Duration) {
        let hash = hash_key(key);
        let data_path = self.dir.join(&hash);
        let ttl_path = self.dir.join(format!("{hash}.ttl"));

        let expiry = unix_now() + ttl.as_secs();
        let _ = std::fs::write(&data_path, bytes);
        let _ = std::fs::write(&ttl_path, expiry.to_string());
    }

    /// Returns true if a non-expired entry exists for `key`, without reading the data.
    pub fn contains(&self, key: &str) -> bool {
        let hash = hash_key(key);
        let ttl_path = self.dir.join(format!("{hash}.ttl"));
        if let Ok(ttl_str) = std::fs::read_to_string(&ttl_path) {
            let expiry: u64 = ttl_str.trim().parse().unwrap_or(0);
            unix_now() <= expiry
        } else {
            false
        }
    }

    /// Remove a specific cache entry.
    pub fn invalidate(&self, key: &str) {
        let hash = hash_key(key);
        let _ = std::fs::remove_file(self.dir.join(&hash));
        let _ = std::fs::remove_file(self.dir.join(format!("{hash}.ttl")));
    }

    /// Delete all entries across ALL namespaces whose file mtime is older than `max_age`.
    /// Call at startup to evict stale weather data (data older than 24h is useless).
    pub fn purge_old_global(max_age: Duration) {
        let root = cache_root();
        let cutoff_secs = unix_now().saturating_sub(max_age.as_secs());
        // Walk one level of subdirectory (namespaces like "radar/l3", "goes", etc.)
        for depth1 in Self::read_dir_names(&root) {
            let d1 = root.join(&depth1);
            // Namespace dirs may be nested one more level (e.g. radar/l3)
            for depth2 in Self::read_dir_names(&d1) {
                let d2 = d1.join(&depth2);
                Self::purge_dir(&d2, cutoff_secs);
            }
            // Also purge files directly in depth1 (flat namespaces like "goes")
            Self::purge_dir(&d1, cutoff_secs);
        }
    }

    fn purge_dir(dir: &std::path::Path, cutoff_secs: u64) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip .ttl sidecars — they'll be cleaned up with the data file
            if path.extension().and_then(|e| e.to_str()) == Some("ttl") {
                continue;
            }
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(mtime) = meta.modified() {
                    let mtime_secs = mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if mtime_secs < cutoff_secs {
                        let _ = std::fs::remove_file(&path);
                        // Remove sidecar too
                        let ttl = path.with_extension("ttl");
                        let _ = std::fs::remove_file(&ttl);
                    }
                }
            }
        }
    }

    fn read_dir_names(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn cache_root() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("meso")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache").join("meso")
    } else {
        PathBuf::from("/tmp/meso-cache")
    }
}

fn cache_dir() -> PathBuf {
    // Respect XDG_CACHE_HOME, fall back to ~/.cache
    cache_root()
}

fn hash_key(key: &str) -> String {
    // Simple but collision-resistant enough for a cache: FNV-1a 64-bit
    let mut hash: u64 = 14695981039346656037;
    for byte in key.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    // Also encode the key suffix so filenames are somewhat human-readable
    let safe: String = key
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    format!("{safe}_{hash:016x}")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
