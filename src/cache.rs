//! A bounded, in-memory media cache for feed preloading.
//!
//! The cache stores source bytes keyed by URL or path. It is deliberately
//! independent of `VideoSource` and of any HTTP client: an application fetches
//! bytes however it likes, offers them to the cache, and can later turn a hit
//! into `VideoSource::bytes(...)`. Eviction is least-recently-used and bounded
//! by bytes, so a long feed cannot grow without limit.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// One cached entry and the time it was last used.
#[derive(Debug)]
struct Entry {
    bytes: Arc<[u8]>,
    used: u64,
}

#[derive(Debug)]
struct Inner {
    max_bytes: u64,
    bytes: u64,
    clock: u64,
    entries: HashMap<String, Entry>,
}

/// A byte-bounded LRU cache of source bytes.
///
/// Clones share the same store. `get` counts as a use and refreshes the entry's
/// position in the eviction order.
#[derive(Clone, Debug)]
pub struct MediaCache {
    inner: Arc<Mutex<Inner>>,
}

impl Default for MediaCache {
    fn default() -> Self {
        Self::with_limit(64 * 1024 * 1024)
    }
}

impl MediaCache {
    /// A cache that holds at most `max_bytes` of payload.
    pub fn with_limit(max_bytes: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                max_bytes,
                bytes: 0,
                clock: 0,
                entries: HashMap::new(),
            })),
        }
    }

    /// The byte limit this cache enforces.
    pub fn limit(&self) -> u64 {
        self.lock().max_bytes
    }

    /// Payload bytes currently held.
    pub fn bytes(&self) -> u64 {
        self.lock().bytes
    }

    /// Number of cached sources.
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// Whether the cache holds nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the bytes for `key`, refreshing its LRU position.
    pub fn get(&self, key: &str) -> Option<Arc<[u8]>> {
        let mut inner = self.lock();
        let used = inner.tick();
        let entry = inner.entries.get_mut(key)?;
        entry.used = used;
        Some(Arc::clone(&entry.bytes))
    }

    /// Stores `bytes` under `key`, evicting older entries until it fits.
    ///
    /// A payload larger than the whole limit is not stored. Replacing a key
    /// replaces its payload and frees the old bytes first.
    pub fn put(&self, key: impl Into<String>, bytes: impl Into<Arc<[u8]>>) {
        let bytes = bytes.into();
        let mut inner = self.lock();
        if bytes.len() as u64 > inner.max_bytes {
            return;
        }
        let key = key.into();
        if let Some(old) = inner.entries.remove(&key) {
            inner.bytes -= old.bytes.len() as u64;
        }
        let used = inner.tick();
        inner.bytes += bytes.len() as u64;
        inner.entries.insert(key, Entry { bytes, used });
        inner.evict();
    }

    /// Removes one key, returning its bytes if it was cached.
    pub fn remove(&self, key: &str) -> Option<Arc<[u8]>> {
        let mut inner = self.lock();
        let entry = inner.entries.remove(key)?;
        inner.bytes -= entry.bytes.len() as u64;
        Some(entry.bytes)
    }

    /// Drops every entry.
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.entries.clear();
        inner.bytes = 0;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Inner {
    fn tick(&mut self) -> u64 {
        self.clock = self.clock.wrapping_add(1);
        self.clock
    }

    fn evict(&mut self) {
        while self.bytes > self.max_bytes {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(key) => {
                    let entry = self.entries.remove(&key).expect("key came from the map");
                    self.bytes -= entry.bytes.len() as u64;
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_put_is_readable_and_counted() {
        let cache = MediaCache::with_limit(100);
        cache.put("a", vec![1u8; 40]);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.bytes(), 40);
        assert_eq!(cache.get("a").as_deref(), Some(&[1u8; 40][..]));
    }

    #[test]
    fn eviction_drops_the_least_recently_used_entry() {
        let cache = MediaCache::with_limit(100);
        cache.put("a", vec![1u8; 60]);
        cache.put("b", vec![2u8; 60]);
        // "a" is now the oldest and no longer fits alongside "b".
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert!(cache.bytes() <= 100);
    }

    #[test]
    fn a_get_refreshes_lru_position() {
        let cache = MediaCache::with_limit(100);
        cache.put("a", vec![1u8; 60]);
        cache.put("b", vec![2u8; 40]);
        // Touch "a" after "b" was inserted: it is now the newest entry.
        assert!(cache.get("a").is_some());
        cache.put("c", vec![3u8; 40]);
        assert!(cache.get("a").is_some(), "the refreshed entry survives");
        assert!(cache.get("b").is_none(), "the untouched entry is evicted");
    }

    #[test]
    fn an_oversized_payload_is_not_stored() {
        let cache = MediaCache::with_limit(4);
        cache.put("big", vec![7u8; 5]);
        assert!(cache.is_empty());
        assert_eq!(cache.bytes(), 0);
    }

    #[test]
    fn replacing_a_key_accounts_for_the_old_bytes() {
        let cache = MediaCache::with_limit(100);
        cache.put("a", vec![1u8; 80]);
        cache.put("a", vec![2u8; 40]);
        assert_eq!(cache.bytes(), 40);
        assert_eq!(cache.get("a").as_deref(), Some(&[2u8; 40][..]));
    }

    #[test]
    fn clear_and_remove_release_bytes() {
        let cache = MediaCache::with_limit(100);
        cache.put("a", vec![1u8; 30]);
        cache.put("b", vec![2u8; 30]);
        assert!(cache.remove("a").is_some());
        assert_eq!(cache.bytes(), 30);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.bytes(), 0);
    }
}
