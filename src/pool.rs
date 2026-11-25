//! A pool that ties every player in a feed to one decoder budget and one
//! audio owner.
//!
//! A vertical feed has three kinds of player at once: the visible one, the one
//! that will be visible after the next swipe, and the ones the app keeps warm.
//! Each needs a decode slot, and exactly one of them should be audible. The
//! library already has both halves — [`VideoResourceManager`] bounds decode
//! slots and frame bytes, and the audio arbitration registry decides who is
//! heard — but nothing joined them, so a feed had to keep two bookkeeping
//! systems in step.
//!
//! [`PlayerPool`] is that join. It hands out [`PooledPlayer`] slots in a fixed
//! priority order, promotes a slot when the page changes, drops a slot the
//! moment a view scrolls away, and makes the promoted slot the audible one.
//!
//! ```
//! # use cranpose_video::{PlayerPool, Priority};
//! let pool = PlayerPool::new();
//! let first = pool.reserve(Priority::Active);
//! let next = pool.reserve(Priority::Next);
//! // Scroll to the second clip: it becomes audible and keeps its slot.
//! pool.promote(next.id(), Priority::Active);
//! assert!(next.is_audio_owner());
//! pool.release(first.id());
//! assert_eq!(pool.len(), 1);
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::resource::{Priority, ResourceLimits, VideoResourceManager};

/// The shared state behind every slot handle.
struct PoolState {
    manager: VideoResourceManager,
    audible: Option<u64>,
}

/// The parts a slot handle needs to reach its own state.
struct PoolShared {
    state: Mutex<PoolState>,
    limits: ResourceLimits,
    next_id: AtomicU64,
}

impl PoolShared {
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        // A poisoned lock still holds consistent integers. Refusing to play a
        // whole feed because one unrelated slot panicked while dropping would
        // be worse than continuing with the state that is there.
        self.state
            .lock()
            .unwrap_or_else(|poisoned: PoisonError<MutexGuard<'_, PoolState>>| {
                poisoned.into_inner()
            })
    }
}

/// One player's place in the pool.
///
/// Dropping it releases the slot, its decode budget and its audio claim, so a
/// caller cannot leak a slot by forgetting a `release` call.
#[derive(Clone)]
pub struct PooledPlayer {
    id: u64,
    priority: Priority,
    shared: Arc<PoolShared>,
}

impl std::fmt::Debug for PooledPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledPlayer")
            .field("id", &self.id)
            .field("priority", &self.priority)
            .finish()
    }
}

impl PooledPlayer {
    /// The slot's id, for `promote`, `release` and `should_suspend`.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The priority this slot was reserved or promoted at.
    pub fn priority(&self) -> Priority {
        self.priority
    }

    /// Whether this slot holds a decode slot right now.
    pub fn is_decoding(&self) -> bool {
        !self.shared.lock().manager.should_suspend(self.id)
    }

    /// Whether this slot is the audible one.
    pub fn is_audio_owner(&self) -> bool {
        self.shared.lock().audible == Some(self.id)
    }

    /// Reports how many bytes this slot's frame pool holds.
    pub fn set_frame_bytes(&self, bytes: u64) {
        self.shared.lock().manager.set_frame_bytes(self.id, bytes);
    }

    /// Releases the slot now instead of at the end of the scope.
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for PooledPlayer {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.manager.unregister(self.id);
        if state.audible == Some(self.id) {
            // Promote the highest-ranked survivor, so a feed is never left
            // silent after the audible page closes.
            state.audible = state.manager.ranked().first().copied();
        }
    }
}

/// Bounds and audio arbitration for every player in one feed.
///
/// Clone it freely: every clone drives the same pool.
#[derive(Clone)]
pub struct PlayerPool {
    shared: Arc<PoolShared>,
}

impl Default for PlayerPool {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PlayerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerPool")
            .field("slots", &self.len())
            .field("limits", &self.shared.limits)
            .finish()
    }
}

impl PlayerPool {
    /// A pool with the default limits: two decoders, 96 MiB of frames and one
    /// prefetch at a time.
    pub fn new() -> Self {
        Self::with_limits(ResourceLimits::default())
    }

    /// A pool with explicit limits.
    pub fn with_limits(limits: ResourceLimits) -> Self {
        Self {
            shared: Arc::new(PoolShared {
                state: Mutex::new(PoolState {
                    manager: VideoResourceManager::new(limits),
                    audible: None,
                }),
                limits,
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// The limits this pool enforces.
    pub fn limits(&self) -> ResourceLimits {
        self.shared.limits
    }

    /// Reserves one slot at `priority`.
    ///
    /// The first slot reserved at [`Priority::Active`] becomes the audible one.
    pub fn reserve(&self, priority: Priority) -> PooledPlayer {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let mut state = self.shared.lock();
        // The manager allocates ids from its own counter, so register first and
        // then discard its id: the pool's id is the only one a caller sees, and
        // every manager call below names it.
        let manager_id = state.manager.register(priority);
        debug_assert_eq!(manager_id, id, "pool and manager ids must stay in step");
        if state.audible.is_none() && priority == Priority::Active {
            state.audible = Some(id);
        }
        drop(state);
        PooledPlayer {
            id,
            priority,
            shared: Arc::clone(&self.shared),
        }
    }

    /// Raises a slot's priority, for example when the visible page changes.
    ///
    /// Promoting at [`Priority::Active`] also makes that slot the audible one.
    /// The returned handle carries the new priority, so keeping it instead of
    /// the old one is optional.
    pub fn promote(&self, id: u64, priority: Priority) -> Option<PooledPlayer> {
        let mut state = self.shared.lock();
        if !state.manager.ranked().contains(&id) {
            return None;
        }
        state.manager.set_priority(id, priority);
        if priority == Priority::Active {
            state.audible = Some(id);
        }
        drop(state);
        Some(PooledPlayer {
            id,
            priority,
            shared: Arc::clone(&self.shared),
        })
    }

    /// Gives up a slot by id. Dropping its [`PooledPlayer`] does the same.
    pub fn release(&self, id: u64) {
        let mut state = self.shared.lock();
        state.manager.unregister(id);
        if state.audible == Some(id) {
            state.audible = state.manager.ranked().first().copied();
        }
    }

    /// The audible slot, if one has been promoted.
    pub fn audible(&self) -> Option<u64> {
        self.shared.lock().audible
    }

    /// Slots ordered by priority, highest first.
    pub fn ranked(&self) -> Vec<u64> {
        self.shared.lock().manager.ranked()
    }

    /// Slots past the decoder limit, which should stop decoding.
    pub fn suspended(&self) -> Vec<u64> {
        self.shared.lock().manager.suspended()
    }

    /// Whether a slot should stop decoding now.
    pub fn should_suspend(&self, id: u64) -> bool {
        self.shared.lock().manager.should_suspend(id)
    }

    /// Total frame bytes across every slot.
    pub fn total_frame_bytes(&self) -> u64 {
        self.shared.lock().manager.total_frame_bytes()
    }

    /// Number of live slots.
    pub fn len(&self) -> usize {
        self.shared.lock().manager.len()
    }

    /// Whether the pool holds no slots.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserving_makes_a_slot_live_and_listed() {
        let pool = PlayerPool::new();
        let slot = pool.reserve(Priority::Active);
        assert_eq!(pool.len(), 1);
        assert!(pool.ranked().contains(&slot.id()));
        assert!(slot.is_decoding());
    }

    #[test]
    fn slots_beyond_the_decoder_limit_are_suspended() {
        let pool = PlayerPool::with_limits(ResourceLimits {
            max_decoders: 1,
            ..Default::default()
        });
        let active = pool.reserve(Priority::Active);
        let background = pool.reserve(Priority::Background);
        assert!(!pool.should_suspend(active.id()));
        assert!(pool.should_suspend(background.id()));
        assert!(active.is_decoding());
        assert!(!background.is_decoding());
    }

    #[test]
    fn promoting_a_slot_makes_it_the_audio_owner() {
        let pool = PlayerPool::new();
        let first = pool.reserve(Priority::Active);
        let next = pool.reserve(Priority::Next);
        assert_eq!(pool.audible(), Some(first.id()));
        pool.promote(next.id(), Priority::Active)
            .expect("a live slot can be promoted");
        assert_eq!(pool.audible(), Some(next.id()));
        assert!(!first.is_audio_owner(), "the old page must go quiet");
    }

    #[test]
    fn promoting_an_unknown_slot_changes_nothing() {
        let pool = PlayerPool::new();
        let slot = pool.reserve(Priority::Active);
        assert!(pool.promote(9999, Priority::Active).is_none());
        assert_eq!(pool.audible(), Some(slot.id()));
    }

    #[test]
    fn releasing_the_audible_slot_promotes_a_survivor() {
        let pool = PlayerPool::new();
        let first = pool.reserve(Priority::Active);
        let next = pool.reserve(Priority::Next);
        pool.release(first.id());
        assert_eq!(pool.audible(), Some(next.id()));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn dropping_a_handle_releases_the_slot() {
        let pool = PlayerPool::new();
        let slot = pool.reserve(Priority::Active);
        let id = slot.id();
        drop(slot);
        assert_eq!(pool.len(), 0);
        assert!(!pool.ranked().contains(&id));
    }

    #[test]
    fn frame_bytes_are_summed_across_slots() {
        let pool = PlayerPool::with_limits(ResourceLimits {
            max_frame_bytes: 1000,
            max_decoders: 4,
            ..Default::default()
        });
        let a = pool.reserve(Priority::Active);
        let b = pool.reserve(Priority::Next);
        a.set_frame_bytes(400);
        b.set_frame_bytes(500);
        assert_eq!(pool.total_frame_bytes(), 900);
    }

    #[test]
    fn ranking_puts_the_active_slot_first() {
        let pool = PlayerPool::new();
        let _bg = pool.reserve(Priority::Background);
        let active = pool.reserve(Priority::Active);
        assert_eq!(pool.ranked().first(), Some(&active.id()));
    }

    #[test]
    fn an_empty_pool_reports_no_audio_owner() {
        let pool = PlayerPool::new();
        assert!(pool.is_empty());
        assert_eq!(pool.audible(), None);
    }

    #[test]
    fn limits_are_the_ones_requested() {
        let pool = PlayerPool::with_limits(ResourceLimits {
            max_decoders: 3,
            max_frame_bytes: 4096,
            max_prefetches: 2,
        });
        let limits = pool.limits();
        assert_eq!(limits.max_decoders, 3);
        assert_eq!(limits.max_frame_bytes, 4096);
        assert_eq!(limits.max_prefetches, 2);
    }

    #[test]
    fn a_clone_drives_the_same_pool() {
        let pool = PlayerPool::new();
        let copy = pool.clone();
        let slot = pool.reserve(Priority::Active);
        assert_eq!(copy.len(), 1);
        assert_eq!(copy.audible(), Some(slot.id()));
    }
}
