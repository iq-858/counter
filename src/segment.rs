//! Segment-level scheduling for HLS/fMP4, independent of any HTTP client.
//!
//! A playlist is a sequence of media segments. An fMP4 variant also has an
//! initialization section (`EXT-X-MAP`) that must be fetched before the
//! fragments. This module tracks that order, bounds how far the client reads
//! ahead, and rejects completions from an old generation after an ABR switch
//! or a feed swipe.

/// Stable identity of one fetchable segment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SegmentKey {
    /// An initialization section from `EXT-X-MAP`.
    Initialization(String),
    /// A media segment by URI.
    Media(String),
    /// A media segment addressed by byte range within a URI.
    ByteRange {
        /// The URI of the containing resource.
        uri: String,
        /// First byte of the range.
        start: u64,
        /// Number of bytes in the range.
        length: u64,
    },
}

impl SegmentKey {
    /// A unique cache key string for this segment.
    pub fn cache_key(&self) -> String {
        match self {
            Self::Initialization(uri) => format!("init:{uri}"),
            Self::Media(uri) => format!("media:{uri}"),
            Self::ByteRange { uri, start, length } => {
                format!("range:{uri}:{start}:{length}")
            }
        }
    }
}

/// Where one segment stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentState {
    /// Not requested yet.
    Idle,
    /// A request is in flight.
    Fetching,
    /// Bytes are ready for the decoder.
    Ready,
    /// The request failed.
    Failed,
    /// A newer generation invalidated this request.
    Cancelled,
}

/// A bounded queue of segment requests.
pub struct SegmentScheduler {
    generation: u64,
    ahead: usize,
    init: Option<SegmentKey>,
    segments: Vec<SegmentKey>,
    state: Vec<SegmentState>,
}

impl SegmentScheduler {
    /// A scheduler that keeps at most `ahead` segments in flight or ready.
    pub fn new(ahead: usize) -> Self {
        Self {
            generation: 1,
            ahead: ahead.max(1),
            init: None,
            segments: Vec::new(),
            state: Vec::new(),
        }
    }

    /// The current generation; pass it with completions.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Sets the fMP4 initialization section for subsequent segments.
    pub fn set_initialization(&mut self, key: SegmentKey) {
        self.init = Some(key);
    }

    /// Replaces the segment list, resetting their states.
    pub fn set_segments(&mut self, segments: Vec<SegmentKey>) {
        self.segments = segments;
        self.state = vec![SegmentState::Idle; self.segments.len()];
    }

    /// The initialization key, if this variant needs one.
    pub fn initialization(&self) -> Option<&SegmentKey> {
        self.init.as_ref()
    }

    /// Requests the next segment when the in-flight and ready window allows.
    /// Returns its index and generation; `None` means the window is full.
    pub fn request_next(&mut self) -> Option<(usize, u64)> {
        let busy = self
            .state
            .iter()
            .filter(|state| **state == SegmentState::Fetching)
            .count();
        if busy >= self.ahead {
            return None;
        }
        let index = self
            .state
            .iter()
            .position(|state| *state == SegmentState::Idle)?;
        self.state[index] = SegmentState::Fetching;
        Some((index, self.generation))
    }

    /// Accepts a completion when its generation is current.
    pub fn complete(&mut self, index: usize, generation: u64) -> bool {
        if generation != self.generation {
            if let Some(state) = self.state.get_mut(index) {
                *state = SegmentState::Cancelled;
            }
            return false;
        }
        if let Some(state) = self.state.get_mut(index) {
            *state = SegmentState::Ready;
        }
        true
    }

    /// Marks a fetch as failed; the caller may retry within the same generation.
    pub fn fail(&mut self, index: usize, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        if let Some(state) = self.state.get_mut(index) {
            *state = SegmentState::Failed;
        }
        true
    }

    /// Retries a failed or cancelled segment.
    pub fn retry(&mut self, index: usize) -> bool {
        match self.state.get_mut(index) {
            Some(state)
                if *state == SegmentState::Failed || *state == SegmentState::Cancelled =>
            {
                *state = SegmentState::Idle;
                true
            }
            _ => false,
        }
    }

    /// Bumps the generation, cancelling all in-flight work (e.g. on ABR switch
    /// or a feed swipe). Returns the keys that were in flight.
    pub fn invalidate(&mut self) -> Vec<SegmentKey> {
        self.generation = self.generation.wrapping_add(1);
        let mut cancelled = Vec::new();
        for (index, state) in self.state.iter_mut().enumerate() {
            if *state == SegmentState::Fetching {
                *state = SegmentState::Cancelled;
                if let Some(key) = self.segments.get(index) {
                    cancelled.push(key.clone());
                }
            }
        }
        cancelled
    }

    /// State of one segment.
    pub fn state(&self, index: usize) -> Option<SegmentState> {
        self.state.get(index).copied()
    }

    /// Cache keys of segments that are ready for decoding.
    pub fn ready_keys(&self) -> Vec<String> {
        self.state
            .iter()
            .enumerate()
            .filter(|(_, state)| **state == SegmentState::Ready)
            .filter_map(|(index, _)| self.segments.get(index))
            .map(SegmentKey::cache_key)
            .collect()
    }

    /// Number of segments tracked.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Whether no segments are tracked.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn segments() -> Vec<SegmentKey> {
        (0..4)
            .map(|i| SegmentKey::Media(format!("seg{i}.m4s")))
            .collect()
    }

    #[test]
    fn keys_distinguish_init_media_and_ranges() {
        let init = SegmentKey::Initialization("init.mp4".into()).cache_key();
        let media = SegmentKey::Media("init.mp4".into()).cache_key();
        let range = SegmentKey::ByteRange {
            uri: "init.mp4".into(),
            start: 10,
            length: 20,
        }
        .cache_key();
        let all: HashSet<_> = [init, media, range].into_iter().collect();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn the_window_bounds_outstanding_requests() {
        let mut scheduler = SegmentScheduler::new(2);
        scheduler.set_segments(segments());
        let (a, generation) = scheduler.request_next().unwrap();
        let (b, _) = scheduler.request_next().unwrap();
        assert_ne!(a, b);
        assert!(scheduler.request_next().is_none());
        assert!(scheduler.complete(a, generation));
        assert!(scheduler.request_next().is_some());
    }

    #[test]
    fn a_stale_completion_is_rejected() {
        let mut scheduler = SegmentScheduler::new(2);
        scheduler.set_segments(segments());
        let (index, old) = scheduler.request_next().unwrap();
        let cancelled = scheduler.invalidate();
        assert_eq!(cancelled.len(), 1);
        assert!(!scheduler.complete(index, old));
        assert_eq!(scheduler.state(index), Some(SegmentState::Cancelled));
        assert!(scheduler.retry(index));
    }

    #[test]
    fn ready_keys_only_include_completed_segments() {
        let mut scheduler = SegmentScheduler::new(2);
        scheduler.set_segments(segments());
        let (index, generation) = scheduler.request_next().unwrap();
        assert!(scheduler.ready_keys().is_empty());
        scheduler.complete(index, generation);
        assert_eq!(scheduler.ready_keys(), vec![format!("media:seg{index}.m4s")]);
    }

    #[test]
    fn a_failure_can_be_retried_in_the_same_generation() {
        let mut scheduler = SegmentScheduler::new(1);
        scheduler.set_segments(segments());
        let (index, generation) = scheduler.request_next().unwrap();
        assert!(scheduler.fail(index, generation));
        assert!(scheduler.retry(index));
        assert_eq!(scheduler.state(index), Some(SegmentState::Idle));
    }
}
