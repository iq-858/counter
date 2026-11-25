//! A small, byte-bounded disk cache for media segments.
//!
//! Keys are encoded into safe file names, writes are atomic (temp file then
//! rename), and a byte budget is enforced after each write. No HTTP client is
//! involved; the caller supplies bytes.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// A disk-backed cache under `root`.
#[derive(Debug, Clone)]
pub struct DiskCache {
    root: PathBuf,
    max_bytes: u64,
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

impl DiskCache {
    /// Opens or creates a cache under `root`.
    pub fn open(root: impl AsRef<Path>, max_bytes: u64) -> std::io::Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        Ok(Self {
            root: root.as_ref().to_path_buf(),
            max_bytes,
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
            .filter_map(|entry| entry.metadata().ok())
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len())
            .sum()
    }

    /// Reads a cached payload.
    pub fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        let path = self.root.join(encode_key(key));
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Writes a payload atomically, then evicts oldest files until it fits.
    pub fn put(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        if bytes.len() as u64 > self.max_bytes {
            return Ok(());
        }
        let final_path = self.root.join(encode_key(key));
        let temp_path = self.root.join(format!("{}.tmp", encode_key(key)));
        fs::write(&temp_path, bytes)?;
        fs::rename(&temp_path, &final_path)?;
        self.evict()?;
        Ok(())
    }

    /// Removes one key.
    pub fn remove(&self, key: &str) -> std::io::Result<()> {
        match fs::remove_file(self.root.join(encode_key(key))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn evict(&self) -> std::io::Result<()> {
        let mut files: Vec<_> = fs::read_dir(&self.root)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.metadata().ok().map(|metadata| (entry.path(), metadata)))
            .filter(|(_, metadata)| metadata.is_file())
            .collect();
        if self.bytes() <= self.max_bytes {
            return Ok(());
        }
        files.sort_by_key(|(_, metadata)| metadata.modified().ok());
        for (path, _) in files {
            if self.bytes() <= self.max_bytes {
                break;
            }
            let _ = fs::remove_file(path);
        }
        Ok(())
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
}
