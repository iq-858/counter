//! Central resource manager for multi-video playback.
//!
//! A Reels-style app opens a feed of clips but only ONE should actively decode
//! and play sound. Without a manager every player owns its own decode thread
//! and frame pool, so memory and threads grow with the number of pages. This
//! module bounds them explicitly and gives the app a priority-driven policy.
//!
//! Pure Rust: no platform SDK, no threads spawned here. The player decides how
//! to react to `should_suspend` (stop reading / drop its decoder).

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// Playback priority. Higher is more important.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Off-screen or background — first to be suspended.
    Background = 0,
    /// A nearby preloaded clip.
    Nearby = 1,
    /// The next clip that will become visible.
    Next = 2,
    /// The one clip currently visible and audible.
    Active = 3,
}

impl Default for Priority {
    fn default() -> Self {
        Self::Background
    }
}

/// Hard limits for the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum number of simultaneously decoding players.
    pub max_decoders: usize,
    /// Maximum total bytes held in all frame pools.
    pub max_frame_bytes: u64,
    /// Maximum concurrent prefetch operations.
    pub max_prefetches: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_decoders: 2, // active + one prepared next clip
            max_frame_bytes: 96 * 1024 * 1024,
            max_prefetches: 1,
        }
    }
}

/// One registered player's live state.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    priority: Priority,
    frame_bytes: u64,
}

/// Shared inner state.
#[derive(Default)]
struct Inner {
    entries: HashMap<u64, Entry>,
    prefetches: usize,
}

/// A cloneable handle to the central manager.
#[derive(Clone, Default)]
pub struct VideoResourceManager {
    inner: Arc<Mutex<Inner>>,
    limits: ResourceLimits,
    next_id: Arc<AtomicU64>,
}

impl VideoResourceManager {
    /// Creates a manager with the given hard limits.
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            limits,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Registers a player and returns its id.
    pub fn register(&self, priority: Priority) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut inner) = self.inner.lock() {
            inner.entries.insert(
                id,
                Entry {
                    priority,
                    frame_bytes: 0,
                },
            );
        }
        id
    }

    /// Removes a player, releasing its budget.
    pub fn unregister(&self, id: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.entries.remove(&id);
        }
    }

    /// Updates a player's priority (e.g. when the active page changes).
    pub fn set_priority(&self, id: u64, priority: Priority) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(entry) = inner.entries.get_mut(&id) {
                entry.priority = priority;
            }
        }
    }

    /// Reports how many bytes a player's frame pool currently holds.
    pub fn set_frame_bytes(&self, id: u64, bytes: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(entry) = inner.entries.get_mut(&id) {
                entry.frame_bytes = bytes;
            }
        }
    }

    /// Total bytes reserved across all players.
    pub fn total_frame_bytes(&self) -> u64 {
        self.inner
            .lock()
            .map(|inner| inner.entries.values().map(|e| e.frame_bytes).sum())
            .unwrap_or(0)
    }

    /// Whether reserving `additional` bytes stays within the budget.
    pub fn can_reserve(&self, additional: u64) -> bool {
        self.total_frame_bytes().saturating_add(additional) <= self.limits.max_frame_bytes
    }

    /// Ids ordered by priority (highest first), then by registration order.
    pub fn ranked(&self) -> Vec<u64> {
        let inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return Vec::new(),
        };
        let mut ids: Vec<(u64, Priority)> = inner
            .entries
            .iter()
            .map(|(id, e)| (*id, e.priority))
            .collect();
        ids.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ids.into_iter().map(|(id, _)| id).collect()
    }

    /// Ids that should be suspended so that at most `max_decoders` decode.
    pub fn suspended(&self) -> Vec<u64> {
        let ranked = self.ranked();
        ranked.into_iter().skip(self.limits.max_decoders).collect()
    }

    /// Whether a specific player should stop decoding now.
    pub fn should_suspend(&self, id: u64) -> bool {
        self.suspended().contains(&id)
    }

    /// Attempts to acquire one prefetch slot. Returns a guard that releases it.
    pub fn acquire_prefetch(&self) -> Option<PrefetchGuard> {
        {
            let mut inner = self.inner.lock().ok()?;
            if inner.prefetches >= self.limits.max_prefetches {
                return None;
            }
            inner.prefetches += 1;
        }
        Some(PrefetchGuard {
            inner: Arc::clone(&self.inner),
        })
    }

    /// Number of registered players.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|i| i.entries.len()).unwrap_or(0)
    }

    /// Whether no players are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// RAII guard for a prefetch slot; releases on drop (cancellation-safe).
pub struct PrefetchGuard {
    inner: Arc<Mutex<Inner>>,
}

impl Drop for PrefetchGuard {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.prefetches = inner.prefetches.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_are_bounded() {
        let limits = ResourceLimits::default();
        assert!(limits.max_decoders >= 1);
        assert!(limits.max_prefetches >= 1);
    }

    #[test]
    fn register_and_unregister() {
        let mgr = VideoResourceManager::new(ResourceLimits::default());
        let id = mgr.register(Priority::Active);
        assert_eq!(mgr.len(), 1);
        mgr.unregister(id);
        assert!(mgr.is_empty());
    }

    #[test]
    fn higher_priority_ranks_first() {
        let mgr = VideoResourceManager::new(ResourceLimits::default());
        let low = mgr.register(Priority::Background);
        let high = mgr.register(Priority::Active);
        assert_eq!(mgr.ranked(), vec![high, low]);
    }

    #[test]
    fn priority_can_change() {
        let mgr = VideoResourceManager::new(ResourceLimits::default());
        let a = mgr.register(Priority::Active);
        let b = mgr.register(Priority::Background);
        // Equal priorities fall back to registration order: b is now at the
        // same priority as a but was registered later.
        mgr.set_priority(b, Priority::Active);
        assert_eq!(mgr.ranked(), vec![a, b]);
        // Changing b to the unique highest priority puts it first.
        mgr.set_priority(b, Priority::Next);
        mgr.set_priority(a, Priority::Background);
        assert_eq!(mgr.ranked(), vec![b, a]);
    }

    #[test]
    fn excess_decoders_are_suspended() {
        let mgr = VideoResourceManager::new(ResourceLimits {
            max_decoders: 2,
            ..Default::default()
        });
        let a = mgr.register(Priority::Active);
        let b = mgr.register(Priority::Next);
        let c = mgr.register(Priority::Nearby);
        let d = mgr.register(Priority::Background);
        let suspended = mgr.suspended();
        assert_eq!(suspended.len(), 2);
        assert!(!suspended.contains(&a));
        assert!(!suspended.contains(&b));
        assert!(suspended.contains(&c));
        assert!(suspended.contains(&d));
        assert!(mgr.should_suspend(d));
        assert!(!mgr.should_suspend(a));
    }

    #[test]
    fn byte_budget_is_enforced() {
        let mgr = VideoResourceManager::new(ResourceLimits {
            max_frame_bytes: 100,
            ..Default::default()
        });
        let id = mgr.register(Priority::Active);
        mgr.set_frame_bytes(id, 60);
        assert!(mgr.can_reserve(40));
        assert!(!mgr.can_reserve(41));
    }

    #[test]
    fn prefetch_slot_bounded_and_released() {
        let mgr = VideoResourceManager::new(ResourceLimits {
            max_prefetches: 1,
            ..Default::default()
        });
        let guard = mgr.acquire_prefetch().expect("first slot");
        assert!(mgr.acquire_prefetch().is_none());
        drop(guard);
        assert!(mgr.acquire_prefetch().is_some());
    }
}
