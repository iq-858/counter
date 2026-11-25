//! Incremental preparation of a source that is not fully downloaded yet.
//!
//! A feed often has only the first bytes of the next clip. This manager tracks
//! those partial buffers, rejects late completions from a cancelled attempt,
//! and hands a complete buffer to the caller exactly once. It does not fetch
//! bytes itself: the application reports chunks.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Where one preparation attempt stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreloadStage {
    /// No attempt started.
    Idle,
    /// The source was requested.
    Requested,
    /// Bytes are arriving.
    Downloading,
    /// The caller is opening a decoder for the complete buffer.
    Preparing,
    /// A player slot is ready.
    Prepared,
    /// The attempt failed.
    Failed,
    /// A newer selection invalidated this attempt.
    Cancelled,
}

struct Entry {
    generation: u64,
    stage: PreloadStage,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct Inner {
    next_generation: u64,
    entries: HashMap<String, Entry>,
}

/// Shared state for incremental source preparation.
#[derive(Clone, Debug)]
pub struct PreloadManager {
    inner: Arc<Mutex<Inner>>,
}

impl Default for PreloadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PreloadManager {
    /// An empty manager.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                next_generation: 1,
                entries: HashMap::new(),
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Starts an attempt and returns its generation.
    pub fn begin(&self, key: impl Into<String>) -> u64 {
        let mut inner = self.lock();
        let generation = inner.next_generation;
        inner.next_generation = inner.next_generation.wrapping_add(1);
        inner.entries.insert(
            key.into(),
            Entry {
                generation,
                stage: PreloadStage::Requested,
                bytes: Vec::new(),
            },
        );
        generation
    }

    /// Appends a chunk to an attempt, rejecting stale generations.
    pub fn offer_chunk(&self, key: &str, generation: u64, chunk: &[u8]) -> bool {
        let mut inner = self.lock();
        match inner.entries.get_mut(key) {
            Some(entry) if entry.generation == generation => {
                entry.stage = PreloadStage::Downloading;
                entry.bytes.extend_from_slice(chunk);
                true
            }
            _ => false,
        }
    }

    /// Marks the buffer complete and returns it when the generation is current.
    pub fn finish(&self, key: &str, generation: u64) -> Option<Vec<u8>> {
        let mut inner = self.lock();
        let entry = inner.entries.get_mut(key)?;
        if entry.generation != generation {
            entry.stage = PreloadStage::Cancelled;
            return None;
        }
        entry.stage = PreloadStage::Preparing;
        Some(entry.bytes.clone())
    }

    /// Marks a slot ready after a decoder was opened from the buffer.
    pub fn mark_prepared(&self, key: &str, generation: u64) -> bool {
        let mut inner = self.lock();
        match inner.entries.get_mut(key) {
            Some(entry) if entry.generation == generation => {
                entry.stage = PreloadStage::Prepared;
                true
            }
            Some(entry) => {
                entry.stage = PreloadStage::Cancelled;
                false
            }
            None => false,
        }
    }

    /// Cancels an attempt, invalidating its generation and releasing bytes.
    pub fn cancel(&self, key: &str) -> bool {
        let mut inner = self.lock();
        match inner.entries.get_mut(key) {
            Some(entry) => {
                entry.generation = entry.generation.wrapping_add(1);
                entry.stage = PreloadStage::Cancelled;
                entry.bytes.clear();
                true
            }
            None => false,
        }
    }

    /// The stage of an attempt.
    pub fn stage(&self, key: &str) -> Option<PreloadStage> {
        self.lock().entries.get(key).map(|entry| entry.stage)
    }

    /// Bytes collected so far.
    pub fn buffered(&self, key: &str) -> usize {
        self.lock()
            .entries
            .get(key)
            .map(|entry| entry.bytes.len())
            .unwrap_or(0)
    }

    /// Drops every attempt.
    pub fn clear(&self) {
        self.lock().entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_accumulate_and_finish_returns_them() {
        let manager = PreloadManager::new();
        let generation = manager.begin("clip-1");
        assert!(manager.offer_chunk("clip-1", generation, &[1, 2]));
        assert!(manager.offer_chunk("clip-1", generation, &[3]));
        assert_eq!(manager.stage("clip-1"), Some(PreloadStage::Downloading));
        assert_eq!(manager.finish("clip-1", generation), Some(vec![1, 2, 3]));
        assert!(manager.mark_prepared("clip-1", generation));
        assert_eq!(manager.stage("clip-1"), Some(PreloadStage::Prepared));
    }

    #[test]
    fn a_cancelled_attempt_rejects_late_chunks_and_finish() {
        let manager = PreloadManager::new();
        let generation = manager.begin("clip-2");
        manager.offer_chunk("clip-2", generation, &[7]);
        assert!(manager.cancel("clip-2"));
        assert!(!manager.offer_chunk("clip-2", generation, &[8]));
        assert_eq!(manager.finish("clip-2", generation), None);
        assert_eq!(manager.stage("clip-2"), Some(PreloadStage::Cancelled));
        assert_eq!(manager.buffered("clip-2"), 0);
    }

    #[test]
    fn different_attempts_are_independent() {
        let manager = PreloadManager::new();
        let a = manager.begin("a");
        let b = manager.begin("b");
        manager.offer_chunk("a", a, &[1]);
        assert!(!manager.offer_chunk("b", a, &[2]));
        assert!(manager.offer_chunk("b", b, &[2]));
        assert_eq!(manager.buffered("a"), 1);
        assert_eq!(manager.buffered("b"), 1);
        manager.clear();
        assert_eq!(manager.stage("a"), None);
    }
}
