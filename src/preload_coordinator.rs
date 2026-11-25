//! Layered predictive preloading for a video feed.
//!
//! The coordinator decides *how far* to prepare each item. It never downloads
//! or opens decoders: it emits source keys per level so an application can do
//! that work with its own HTTP client and player backend. It uses the existing
//! `PreloadManager` for generation-safe chunk tracking.

use std::collections::{HashMap, HashSet};

use crate::PreloadManager;
#[cfg(test)]
use crate::PreloadStage;

/// How far ahead the feed prepares at each stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreloadConfig {
    /// Items whose metadata and poster are fetched.
    pub metadata_ahead: usize,
    /// Items whose first bytes are downloaded.
    pub bytes_ahead: usize,
    /// Items with an opened player slot.
    pub prepared_players_ahead: usize,
    /// Items whose first frame is decoded behind a poster.
    pub first_frame_ahead: usize,
    /// Parallel download budget reported to the application.
    pub max_parallel_downloads: usize,
}

impl Default for PreloadConfig {
    fn default() -> Self {
        Self {
            metadata_ahead: 3,
            bytes_ahead: 2,
            prepared_players_ahead: 1,
            first_frame_ahead: 1,
            max_parallel_downloads: 2,
        }
    }
}

/// The preparation level a key reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PreloadLevel {
    /// Nothing started.
    None,
    /// Metadata and poster.
    Metadata,
    /// Initial bytes.
    Bytes,
    /// Media source attached.
    Source,
    /// Decoder opened.
    Decoder,
    /// First frame ready.
    FirstFrame,
    /// Player promoted and ready.
    Prepared,
}

/// One scheduled item and its current level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreloadTask {
    /// Stable source key.
    pub key: String,
    /// Distance from the selected item.
    pub distance: usize,
    /// The level this item reached.
    pub level: PreloadLevel,
}

/// Source-keyed coordinator with cancellation-safe generations.
pub struct PreloadCoordinator {
    config: PreloadConfig,
    manager: PreloadManager,
    tasks: HashMap<String, PreloadTask>,
    generations: HashMap<String, u64>,
}

impl PreloadCoordinator {
    /// A coordinator with `config`.
    pub fn new(config: PreloadConfig) -> Self {
        Self {
            config,
            manager: PreloadManager::new(),
            tasks: HashMap::new(),
            generations: HashMap::new(),
        }
    }

    /// The configuration.
    pub fn config(&self) -> PreloadConfig {
        self.config
    }

    /// The generation-safe chunk manager behind this coordinator.
    pub fn manager(&self) -> &PreloadManager {
        &self.manager
    }

    /// The task for `key`, if scheduled.
    pub fn task(&self, key: &str) -> Option<&PreloadTask> {
        self.tasks.get(key)
    }

    /// Schedules the keys from `current` onward, skipping known keys and the
    /// selected item itself. Returns the keys that should start at level 1.
    pub fn schedule(&mut self, sources: &[String], current: usize) -> Vec<String> {
        let mut started = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            if index <= current {
                continue;
            }
            if self.tasks.contains_key(source) {
                continue;
            }
            let distance = index - current;
            let generation = self.manager.begin(source);
            self.generations.insert(source.clone(), generation);
            self.tasks.insert(
                source.clone(),
                PreloadTask {
                    key: source.clone(),
                    distance,
                    level: PreloadLevel::Metadata,
                },
            );
            started.push(source.clone());
        }
        started
    }

    /// Advances a key if the requested level is allowed at its distance.
    pub fn advance(&mut self, key: &str, to: PreloadLevel) -> bool {
        // Check with an immutable borrow before taking the mutable one, so
        // `self.allowed(...)` does not borrow `self` twice.
        let Some(task) = self.tasks.get(key) else {
            return false;
        };
        if to <= task.level || !self.allowed(task.distance, to) {
            return false;
        }
        let generation = self.generations.get(key).copied().unwrap_or(0);
        if to >= PreloadLevel::Bytes {
            let _ = self.manager.offer_chunk(key, generation, &[]);
        }
        if let Some(task) = self.tasks.get_mut(key) {
            task.level = to;
        }
        true
    }

    /// Cancels one key, rejecting any late completion.
    pub fn cancel(&mut self, key: &str) -> bool {
        if self.tasks.remove(key).is_none() {
            return false;
        }
        self.generations.remove(key);
        self.manager.cancel(key);
        true
    }

    /// Cancels every key not in `keep`.
    pub fn cancel_obsolete(&mut self, keep: &HashSet<String>) {
        let obsolete: Vec<String> = self
            .tasks
            .keys()
            .filter(|key| !keep.contains(*key))
            .cloned()
            .collect();
        for key in obsolete {
            self.cancel(&key);
        }
    }

    /// The next key at or beyond `Prepared` level.
    pub fn next_prepared(&self) -> Option<&PreloadTask> {
        self.tasks
            .values()
            .filter(|task| task.level >= PreloadLevel::Prepared)
            .min_by_key(|task| task.distance)
    }

    fn allowed(&self, distance: usize, level: PreloadLevel) -> bool {
        let limit = match level {
            PreloadLevel::Metadata => self.config.metadata_ahead,
            PreloadLevel::Bytes => self.config.bytes_ahead,
            PreloadLevel::Source | PreloadLevel::Decoder => self.config.prepared_players_ahead,
            PreloadLevel::FirstFrame | PreloadLevel::Prepared => self.config.first_frame_ahead,
            PreloadLevel::None => return false,
        };
        distance <= limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources() -> Vec<String> {
        (0..5).map(|i| format!("clip-{i}")).collect()
    }

    #[test]
    fn scheduling_skips_the_current_item_and_duplicates() {
        let mut coordinator = PreloadCoordinator::new(PreloadConfig::default());
        let items = sources();
        let started = coordinator.schedule(&items, 0);
        assert_eq!(started.len(), 4);
        assert!(coordinator.schedule(&items, 0).is_empty());
        assert_eq!(coordinator.task("clip-0"), None);
    }

    #[test]
    fn levels_respect_distance_limits() {
        let mut coordinator = PreloadCoordinator::new(PreloadConfig::default());
        let items = sources();
        coordinator.schedule(&items, 0);
        assert!(coordinator.advance("clip-1", PreloadLevel::Bytes));
        // clip-4 is farther than the byte budget.
        assert!(!coordinator.advance("clip-4", PreloadLevel::Bytes));
    }

    #[test]
    fn cancellation_rejects_late_work() {
        let mut coordinator = PreloadCoordinator::new(PreloadConfig::default());
        let items = sources();
        coordinator.schedule(&items, 0);
        let generation = coordinator.generations["clip-1"];
        assert!(coordinator.cancel("clip-1"));
        assert!(!coordinator
            .manager()
            .offer_chunk("clip-1", generation, &[1, 2, 3]));
        assert_eq!(coordinator.manager().stage("clip-1"), Some(PreloadStage::Cancelled));
    }

    #[test]
    fn obsolete_keys_are_cancelled_on_swipe() {
        let mut coordinator = PreloadCoordinator::new(PreloadConfig::default());
        let items = sources();
        coordinator.schedule(&items, 0);
        let mut keep = HashSet::new();
        keep.insert("clip-2".to_owned());
        coordinator.cancel_obsolete(&keep);
        assert_eq!(coordinator.task("clip-1"), None);
        assert!(coordinator.task("clip-2").is_some());
    }

    #[test]
    fn a_prepared_task_is_visible_as_next() {
        let mut coordinator = PreloadCoordinator::new(PreloadConfig::default());
        let items = sources();
        coordinator.schedule(&items, 0);
        coordinator.advance("clip-1", PreloadLevel::Prepared);
        let next = coordinator.next_prepared().expect("a prepared item exists");
        assert_eq!(next.key, "clip-1");
    }
}
