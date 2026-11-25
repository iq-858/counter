//! Orchestration for a vertical video feed.
//!
//! This module owns the *bookkeeping* of a feed: which item is selected, which
//! items are being prepared, which one is audible, and which preparation
//! results are stale. It deliberately does not open decoders or download bytes:
//! an application performs those operations and reports them through
//! `begin_preload` / `finish_preload`. That keeps the state machine testable on
//! every target and leaves the native backends in charge of I/O.

use std::collections::HashMap;

use crate::{PlayerPool, PooledPlayer, Priority, ResourceLimits};

/// One item in a feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedItem {
    /// Stable identity used for cache keys and stale-result rejection.
    pub id: String,
    /// Source handed to `VideoPlayer::open` or `VideoSource`.
    pub source: String,
}

impl FeedItem {
    /// An item with a stable id and a source.
    pub fn new(id: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
        }
    }
}

/// Where an item stands in the preparation pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedStage {
    /// The item has not been touched.
    Idle,
    /// Preparation was requested but has not started.
    Requested,
    /// Bytes are being downloaded by the application.
    Downloading,
    /// A decoder and surface are being prepared by the application.
    Preparing,
    /// A bounded player slot is reserved for this item.
    Prepared,
    /// The first frame is available behind a poster.
    Ready,
    /// The selected item is playing.
    Playing,
    /// A previously selected item is kept warm but silent.
    Paused,
    /// The selection is moving from one item to another.
    Transitioning,
    /// The item finished normally.
    Completed,
    /// Preparation or playback failed.
    Failed,
    /// A newer selection invalidated this item's work.
    Cancelled,
}

/// How eagerly the feed prepares items around the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreloadStrategy {
    /// Only the selected item; no preloading.
    None,
    /// One following item, prepared without network adaptation.
    Conservative,
    /// Up to `preload_next` items; the application adapts to network state.
    Adaptive,
}

/// Configuration for a feed controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFeedConfig {
    /// Keep the previous item warm.
    pub preload_previous: bool,
    /// How many following items to prepare.
    pub preload_next: usize,
    /// Maximum number of player slots in the pool.
    pub max_active_players: usize,
    /// Maximum number of decoders the pool allows to run.
    pub max_concurrent_decoders: usize,
    /// Maximum parallel downloads; reported to the application, not enforced here.
    pub max_parallel_downloads: usize,
    /// Selection strategy.
    pub preload_strategy: PreloadStrategy,
}

impl Default for VideoFeedConfig {
    fn default() -> Self {
        Self {
            preload_previous: false,
            preload_next: 2,
            max_active_players: 3,
            max_concurrent_decoders: 2,
            max_parallel_downloads: 2,
            preload_strategy: PreloadStrategy::Adaptive,
        }
    }
}

/// Deterministic state and resource ownership for one feed.
pub struct VideoFeedController {
    config: VideoFeedConfig,
    items: Vec<FeedItem>,
    stages: Vec<FeedStage>,
    generations: Vec<u64>,
    current: usize,
    pool: PlayerPool,
    active_slot: Option<u64>,
    /// The owning handle for the active slot; dropping it releases the slot.
    active_owner: Option<PooledPlayer>,
    preloaded: HashMap<usize, PooledPlayer>,
}

impl VideoFeedController {
    /// A controller over `config` with no items yet.
    pub fn new(config: VideoFeedConfig) -> Self {
        Self {
            config,
            items: Vec::new(),
            stages: Vec::new(),
            generations: Vec::new(),
            current: 0,
            pool: Self::pool_for(config),
            active_slot: None,
            active_owner: None,
            preloaded: HashMap::new(),
        }
    }

    fn pool_for(config: VideoFeedConfig) -> PlayerPool {
        PlayerPool::with_limits(ResourceLimits {
            max_decoders: config.max_concurrent_decoders,
            ..Default::default()
        })
    }

    /// The configuration this controller enforces.
    pub fn config(&self) -> VideoFeedConfig {
        self.config
    }

    /// Replaces the item list, cancelling every in-flight preparation.
    pub fn set_items(&mut self, items: Vec<FeedItem>) {
        self.cancel_all();
        self.pool = Self::pool_for(self.config);
        self.active_slot = None;
        self.active_owner = None;
        self.items = items;
        self.stages = vec![FeedStage::Idle; self.items.len()];
        self.generations = vec![0u64; self.items.len()];
        self.current = 0;
    }

    /// The items currently in the feed.
    pub fn items(&self) -> &[FeedItem] {
        &self.items
    }

    /// The selected index.
    pub fn current_index(&self) -> usize {
        self.current
    }

    /// The selected item, when the feed holds one.
    pub fn current_item(&self) -> Option<&FeedItem> {
        self.items.get(self.current)
    }

    /// The stage of an item.
    pub fn stage(&self, index: usize) -> Option<FeedStage> {
        self.stages.get(index).copied()
    }

    /// The resource pool this feed owns.
    pub fn pool(&self) -> &PlayerPool {
        &self.pool
    }

    /// Selects an item and promotes it to the audible slot.
    ///
    /// Returns `false` for an out-of-range index. Every other in-flight
    /// preparation is cancelled and its generation bumped, so a late
    /// completion from the old selection cannot replace the new one.
    pub fn set_current_index(&mut self, index: usize) -> bool {
        if index >= self.items.len() {
            return false;
        }
        for other in 0..self.items.len() {
            if other != index {
                self.cancel_preload(other);
                if matches!(
                    self.stages[other],
                    FeedStage::Playing | FeedStage::Transitioning
                ) {
                    self.stages[other] = FeedStage::Paused;
                }
            }
        }
        self.current = index;
        self.stages[index] = FeedStage::Playing;

        let owner = match self.preloaded.remove(&index) {
            Some(slot) => slot,
            None => self.pool.reserve(Priority::Active),
        };
        let id = owner.id();
        // Keep the owning handle: a temporary from `reserve` would release the
        // slot as soon as it was dropped. `promote` returns a non-owning view.
        let _ = self.pool.promote(id, Priority::Active);
        self.active_slot = Some(id);
        self.active_owner = Some(owner);
        true
    }

    /// The slot currently marked audible.
    pub fn active_slot(&self) -> Option<u64> {
        self.active_slot
    }

    /// Begins preparing an item and returns the generation a completion must
    /// carry to be accepted. `None` means the item cannot be prepared now.
    pub fn begin_preload(&mut self, index: usize) -> Option<u64> {
        match self.stages.get(index).copied()? {
            FeedStage::Idle | FeedStage::Completed | FeedStage::Failed => {}
            _ => return None,
        }
        self.stages[index] = FeedStage::Requested;
        Some(self.generations[index])
    }

    /// Marks preparation progress. A stale generation is rejected and puts the
    /// item in `Cancelled` instead of letting old work replace the selection.
    pub fn set_preload_stage(&mut self, index: usize, generation: u64, stage: FeedStage) -> bool {
        if self.generations.get(index).copied() != Some(generation) {
            if let Some(current) = self.stages.get_mut(index) {
                *current = FeedStage::Cancelled;
            }
            return false;
        }
        if let Some(current) = self.stages.get_mut(index) {
            *current = stage;
        }
        true
    }

    /// Accepts a completed preparation, reserving a bounded player slot.
    ///
    /// Returns `false` for a stale or invalid completion. No decoder is opened
    /// here — the caller owns that work and only reports its outcome.
    pub fn finish_preload(&mut self, index: usize, generation: u64) -> bool {
        if self.generations.get(index).copied() != Some(generation) {
            if let Some(stage) = self.stages.get_mut(index) {
                *stage = FeedStage::Cancelled;
            }
            return false;
        }
        if index >= self.items.len() || self.preloaded.contains_key(&index) {
            return false;
        }
        self.preloaded.insert(index, self.pool.reserve(Priority::Next));
        self.stages[index] = FeedStage::Prepared;
        true
    }

    /// Cancels one preparation, releasing its slot and invalidating its
    /// generation. Returns whether anything was cancelled.
    pub fn cancel_preload(&mut self, index: usize) -> bool {
        if index >= self.items.len() {
            return false;
        }
        self.generations[index] = self.generations[index].wrapping_add(1);
        let was_pending = self.preloaded.remove(&index).is_some()
            || matches!(
                self.stages[index],
                FeedStage::Requested | FeedStage::Downloading | FeedStage::Preparing
            );
        self.stages[index] = FeedStage::Cancelled;
        was_pending
    }

    /// Cancels every pending preparation.
    pub fn cancel_all(&mut self) {
        for index in 0..self.items.len() {
            self.cancel_preload(index);
        }
    }

    /// Prepares the following item according to the configured strategy.
    ///
    /// Returns its index when a preparation was started. This only starts the
    /// bookkeeping stage; the caller performs the actual I/O.
    pub fn preload_next(&mut self) -> Option<usize> {
        if self.config.preload_strategy == PreloadStrategy::None || self.config.preload_next == 0 {
            return None;
        }
        let next = self.current.checked_add(1)?;
        if next >= self.items.len() || self.preloaded.contains_key(&next) {
            return None;
        }
        self.begin_preload(next)?;
        Some(next)
    }

    /// Prepares the previous item when that is enabled in the config.
    pub fn preload_previous(&mut self) -> Option<usize> {
        if !self.config.preload_previous {
            return None;
        }
        let previous = self.current.checked_sub(1)?;
        if self.preloaded.contains_key(&previous) {
            return None;
        }
        self.begin_preload(previous)?;
        Some(previous)
    }

    /// Number of items with a reserved slot.
    pub fn prepared_count(&self) -> usize {
        self.preloaded.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<FeedItem> {
        (0..4)
            .map(|index| FeedItem::new(format!("clip-{index}"), format!("clip{index}.mp4")))
            .collect()
    }

    #[test]
    fn selecting_an_item_marks_it_playing_and_audible() {
        let mut feed = VideoFeedController::new(VideoFeedConfig::default());
        feed.set_items(items());
        assert!(feed.set_current_index(0));
        assert_eq!(feed.stage(0), Some(FeedStage::Playing));
        assert!(feed.active_slot().is_some());
        assert_eq!(feed.pool().audible(), feed.active_slot());
    }

    #[test]
    fn an_out_of_range_selection_changes_nothing() {
        let mut feed = VideoFeedController::new(VideoFeedConfig::default());
        feed.set_items(items());
        assert!(feed.set_current_index(0));
        assert!(!feed.set_current_index(99));
        assert_eq!(feed.current_index(), 0);
    }

    #[test]
    fn completing_a_preload_reserves_a_slot() {
        let mut feed = VideoFeedController::new(VideoFeedConfig::default());
        feed.set_items(items());
        feed.set_current_index(0);
        let generation = feed.begin_preload(1).expect("a fresh item can be prepared");
        assert_eq!(feed.stage(1), Some(FeedStage::Requested));
        assert!(feed.finish_preload(1, generation));
        assert_eq!(feed.stage(1), Some(FeedStage::Prepared));
        assert_eq!(feed.prepared_count(), 1);
    }

    #[test]
    fn a_stale_completion_is_rejected() {
        let mut feed = VideoFeedController::new(VideoFeedConfig::default());
        feed.set_items(items());
        feed.set_current_index(0);
        let generation = feed.begin_preload(1).unwrap();
        assert!(feed.cancel_preload(1));
        assert!(!feed.finish_preload(1, generation));
        assert_eq!(feed.stage(1), Some(FeedStage::Cancelled));
        assert_eq!(feed.prepared_count(), 0);
    }

    #[test]
    fn switching_selection_promotes_the_prepared_player() {
        let mut feed = VideoFeedController::new(VideoFeedConfig::default());
        feed.set_items(items());
        feed.set_current_index(0);
        let generation = feed.begin_preload(1).unwrap();
        assert!(feed.finish_preload(1, generation));
        let prepared_slot = feed.pool().ranked().into_iter().find(|id| Some(*id) != feed.active_slot());
        let prepared_slot = prepared_slot.expect("the prepared item owns a slot");
        assert!(feed.set_current_index(1));
        assert_eq!(feed.active_slot(), Some(prepared_slot));
        assert_eq!(feed.pool().audible(), Some(prepared_slot));
        assert_eq!(feed.stage(1), Some(FeedStage::Playing));
    }

    #[test]
    fn preloading_respects_the_strategy_and_bounds() {
        let config = VideoFeedConfig {
            preload_next: 1,
            ..VideoFeedConfig::default()
        };
        let mut feed = VideoFeedController::new(config);
        feed.set_items(items());
        feed.set_current_index(0);
        assert_eq!(feed.preload_next(), Some(1));
        assert_eq!(feed.preload_next(), None, "an in-flight item is not duplicated");
        let none = VideoFeedConfig {
            preload_strategy: PreloadStrategy::None,
            ..VideoFeedConfig::default()
        };
        let mut feed = VideoFeedController::new(none);
        feed.set_items(items());
        feed.set_current_index(0);
        assert_eq!(feed.preload_next(), None);
    }

    #[test]
    fn replacing_items_cancels_everything() {
        let mut feed = VideoFeedController::new(VideoFeedConfig::default());
        feed.set_items(items());
        feed.set_current_index(0);
        feed.preload_next();
        feed.set_items(items());
        assert_eq!(feed.current_index(), 0);
        assert_eq!(feed.prepared_count(), 0);
        assert_eq!(feed.pool().len(), 0);
        assert_eq!(feed.stage(1), Some(FeedStage::Idle));
    }
}
