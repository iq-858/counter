//! A small, byte-bounded disk cache for media segments.
//!
//! Keys are encoded into safe file names, writes are atomic (temp file then
//! rename), and a byte budget is enforced after each write. No HTTP client is
//! involved; the caller supplies bytes.
//!
//! Entries can carry an expiry and an integrity hash. A payload whose hash no
//! longer matches is treated as a miss and removed, so a truncated or
//! half-written segment never reaches a decoder. Hit, miss and eviction counts
//! are kept for diagnostics.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

/// A disk-backed cache under `root`.
#[derive(Debug, Clone)]
pub struct DiskCache {
    root: PathBuf,
    max_bytes: u64,
    hits: std::sync::Arc<AtomicU64>,
    misses: std::sync::Arc<AtomicU64>,
    evictions: std::sync::Arc<AtomicU64>,
}

/// Counters for one cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheMetrics {
    /// Reads that returned a valid payload.
    pub hits: u64,
    /// Reads that found nothing, expired, or failed the integrity check.
    pub misses: u64,
    /// Files removed to stay inside the byte budget.
    pub evictions: u64,
}

fn encode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 8);
    for byte in key.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(byte as char),
            _ => out.push_str(&format!("_{byte:02x}")),
        }
    }
    if out.is_empty() {
        out.push_str("_empty");
    }
    out
}

/// FNV-1a over the payload, used as the integrity tag.
///
/// A cryptographic hash is not needed here: the tag only has to catch a
/// truncated or overwritten segment, and FNV keeps the cache dependency-free.
fn integrity(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl DiskCache {
    /// Opens or creates a cache under `root`.
    pub fn open(root: impl AsRef<Path>, max_bytes: u64) -> std::io::Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        Ok(Self {
            root: root.as_ref().to_path_buf(),
            max_bytes,
            hits: Default::default(),
            misses: Default::default(),
            evictions: Default::default(),
        })
    }

    /// The directory holding cached payloads.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The byte budget.
    pub fn limit(&self) -> u64 {
        self.max_bytes
    }

    /// Total bytes held in the cache.
    pub fn bytes(&self) -> u64 {
        fs::read_dir(&self.root)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_none_or(|ext| ext != "meta"))
            .filter_map(|entry| entry.metadata().ok())
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len())
            .sum()
    }

    /// The hit/miss/eviction counters.
    pub fn metrics(&self) -> CacheMetrics {
        CacheMetrics {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }

    /// Reads a cached payload.
    ///
    /// Returns `None` when the key is absent, its sidecar says it expired, or
    /// its integrity tag does not match the bytes on disk.
    pub fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        let path = self.root.join(encode_key(key));
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if let Some(meta) = self.read_meta(key)? {
            if meta.expired() {
                let _ = fs::remove_file(&path);
                let _ = fs::remove_file(self.meta_path(key));
                self.misses.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
            if meta.integrity != integrity(&bytes) {
                let _ = fs::remove_file(&path);
                let _ = fs::remove_file(self.meta_path(key));
                self.misses.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
        }
        self.hits.fetch_add(1, Ordering::Relaxed);
        Ok(Some(bytes))
    }

    /// Writes a payload atomically, then evicts oldest files until it fits.
    pub fn put(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.put_with_expiry(key, bytes, None)
    }

    /// Writes a payload that expires after `ttl`.
    pub fn put_expiring(&self, key: &str, bytes: &[u8], ttl: Duration) -> std::io::Result<()> {
        self.put_with_expiry(key, bytes, Some(ttl))
    }

    fn put_with_expiry(&self, key: &str, bytes: &[u8], ttl: Option<Duration>) -> std::io::Result<()> {
        if bytes.len() as u64 > self.max_bytes {
            return Ok(());
        }
        let final_path = self.root.join(encode_key(key));
        let temp_path = self.root.join(format!("{}.tmp", encode_key(key)));
        fs::write(&temp_path, bytes)?;
        fs::rename(&temp_path, &final_path)?;
        let meta = EntryMeta {
            integrity: integrity(bytes),
            expires_at: ttl.map(|ttl| {
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    + ttl.as_secs()
            }),
        };
        fs::write(self.meta_path(key), meta.encode())?;
        self.evict()?;
        Ok(())
    }

    /// Removes one key and its sidecar.
    pub fn remove(&self, key: &str) -> std::io::Result<()> {
        let _ = fs::remove_file(self.meta_path(key));
        match fs::remove_file(self.root.join(encode_key(key))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn meta_path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{}.meta", encode_key(key)))
    }

    fn read_meta(&self, key: &str) -> std::io::Result<Option<EntryMeta>> {
        match fs::read_to_string(self.meta_path(key)) {
            Ok(text) => Ok(EntryMeta::decode(&text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn evict(&self) -> std::io::Result<()> {
        let mut files: Vec<_> = fs::read_dir(&self.root)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.metadata().ok().map(|metadata| (entry.path(), metadata)))
            .filter(|(path, metadata)| {
                metadata.is_file() && path.extension().is_none_or(|ext| ext != "meta")
            })
            .collect();
        if self.bytes() <= self.max_bytes {
            return Ok(());
        }
        files.sort_by_key(|(_, metadata)| metadata.modified().ok());
        for (path, _) in files {
            if self.bytes() <= self.max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                self.evictions.fetch_add(1, Ordering::Relaxed);
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    let _ = fs::remove_file(self.root.join(format!("{name}.meta")));
                }
            }
        }
        Ok(())
    }
}

/// Sidecar describing one entry.
struct EntryMeta {
    integrity: u64,
    expires_at: Option<u64>,
}

impl EntryMeta {
    fn expired(&self) -> bool {
        let Some(expires_at) = self.expires_at else {
            return false;
        };
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now >= expires_at
    }

    fn encode(&self) -> String {
        match self.expires_at {
            Some(expires_at) => format!("{} {expires_at}\n", self.integrity),
            None => format!("{}\n", self.integrity),
        }
    }

    fn decode(text: &str) -> Option<Self> {
        let mut parts = text.split_whitespace();
        let integrity = parts.next()?.parse().ok()?;
        let expires_at = parts.next().and_then(|value| value.parse().ok());
        Some(Self {
            integrity,
            expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("cranpose-video-cache-{name}-{}", std::process::id()))
    }

    #[test]
    fn writes_and_reads_a_payload() {
        let root = temp_root("read");
        let _ = fs::remove_dir_all(&root);
        let cache = DiskCache::open(&root, 100).unwrap();
        cache.put("seg-1", &[1, 2, 3]).unwrap();
        assert_eq!(cache.get("seg-1").unwrap(), Some(vec![1, 2, 3]));
        cache.remove("seg-1").unwrap();
        assert_eq!(cache.get("seg-1").unwrap(), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn evicts_down_to_the_budget() {
        let root = temp_root("evict");
        let _ = fs::remove_dir_all(&root);
        let cache = DiskCache::open(&root, 60).unwrap();
        cache.put("a", &[1u8; 40]).unwrap();
        cache.put("b", &[2u8; 40]).unwrap();
        assert!(cache.bytes() <= 60);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unsafe_keys_become_safe_file_names() {
        let root = temp_root("keys");
        let _ = fs::remove_dir_all(&root);
        let cache = DiskCache::open(&root, 100).unwrap();
        cache.put("https://cdn/x.m4s?token=1", &[9]).unwrap();
        assert_eq!(cache.get("https://cdn/x.m4s?token=1").unwrap(), Some(vec![9]));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_corrupted_payload_is_a_miss() {
        let root = temp_root("corrupt");
        let _ = fs::remove_dir_all(&root);
        let cache = DiskCache::open(&root, 100).unwrap();
        cache.put("seg", &[1, 2, 3, 4]).unwrap();
        // Overwrite the payload behind the cache's back.
        fs::write(root.join(encode_key("seg")), [9u8, 9, 9, 9]).unwrap();
        assert_eq!(cache.get("seg").unwrap(), None);
        assert_eq!(cache.metrics().misses, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_expired_entry_is_a_miss() {
        let root = temp_root("expiry");
        let _ = fs::remove_dir_all(&root);
        let cache = DiskCache::open(&root, 100).unwrap();
        cache.put_expiring("seg", &[1, 2, 3], Duration::from_secs(0)).unwrap();
        assert_eq!(cache.get("seg").unwrap(), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn metrics_count_hits_and_evictions() {
        let root = temp_root("metrics");
        let _ = fs::remove_dir_all(&root);
        let cache = DiskCache::open(&root, 50).unwrap();
        cache.put("a", &[1u8; 40]).unwrap();
        assert_eq!(cache.get("a").unwrap(), Some(vec![1u8; 40]));
        assert_eq!(cache.metrics().hits, 1);
        cache.put("b", &[2u8; 40]).unwrap();
        assert!(cache.metrics().evictions >= 1);
        let _ = fs::remove_dir_all(&root);
    }
}
