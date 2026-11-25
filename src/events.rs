//! Bounded playback event delivery with position coalescing.
//!
//! The engine publishes state changes here; applications poll with a sequence
//! number or read the recent history. Position events are coalesced so a
//! 60 fps clock does not push 60 events a second unless the application asks
//! for every tick.

use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

/// The kinds of changes an application may observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackEventKind {
    /// A decoded frame is available.
    FrameAvailable,
    /// Playback started.
    PlaybackStarted,
    /// Playback paused.
    PlaybackPaused,
    /// The decoder is waiting for media.
    BufferingStarted,
    /// The decoder has media again.
    BufferingEnded,
    /// The clock moved.
    PositionChanged,
    /// The container duration became known or changed.
    DurationChanged,
    /// The first decoded frame was presented.
    FirstFrameReady,
    /// The decoded picture size changed.
    VideoSizeChanged,
    /// A seek began.
    SeekStarted,
    /// A seek finished.
    SeekCompleted,
    /// The media ended.
    Ended,
    /// The decoder backend changed.
    DecoderChanged,
    /// An error was reported.
    Error,
    /// A resource limit was reached.
    ResourcePressure,
}

/// One ordered event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackEventRecord {
    /// Monotonic sequence number.
    pub sequence: u64,
    /// What changed.
    pub kind: PlaybackEventKind,
    /// Clock position when it changed.
    pub position: Duration,
    /// Wall time of publication.
    pub at: Instant,
}

#[derive(Debug)]
struct Inner {
    events: VecDeque<PlaybackEventRecord>,
    capacity: usize,
    next: u64,
    coalesce_position: bool,
}

/// A bounded event log shared by clones.
#[derive(Debug, Clone)]
pub struct EventBus {
    inner: Mutex<Inner>,
}

impl EventBus {
    /// A log holding at most `capacity` events.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                events: VecDeque::with_capacity(capacity.max(1)),
                capacity: capacity.max(1),
                next: 1,
                coalesce_position: true,
            }),
        }
    }

    /// Whether consecutive position events are replaced instead of appended.
    pub fn coalesces_positions(&self) -> bool {
        self.lock().coalesce_position
    }

    /// Enables or disables position coalescing.
    pub fn set_coalesce_positions(&self, enabled: bool) {
        self.lock().coalesce_position = enabled;
    }

    /// Publishes an event and returns its sequence number.
    pub fn publish(&self, kind: PlaybackEventKind, position: Duration) -> u64 {
        let mut inner = self.lock();
        let sequence = inner.next;
        inner.next = inner.next.wrapping_add(1);
        let record = PlaybackEventRecord {
            sequence,
            kind,
            position,
            at: Instant::now(),
        };
        if inner.coalesce_position
            && kind == PlaybackEventKind::PositionChanged
            && inner.events.back().is_some_and(|last| last.kind == kind)
        {
            if let Some(last) = inner.events.back_mut() {
                *last = record;
            }
            return sequence;
        }
        if inner.events.len() == inner.capacity {
            inner.events.pop_front();
        }
        inner.events.push_back(record);
        sequence
    }

    /// Returns every recorded event in order.
    pub fn events(&self) -> Vec<PlaybackEventRecord> {
        self.lock().events.iter().copied().collect()
    }

    /// Returns events newer than `sequence`.
    pub fn since(&self, sequence: u64) -> Vec<PlaybackEventRecord> {
        self.lock()
            .events
            .iter()
            .copied()
            .filter(|event| event.sequence > sequence)
            .collect()
    }

    /// The newest event, if any.
    pub fn last(&self) -> Option<PlaybackEventRecord> {
        self.lock().events.back().copied()
    }

    /// Removes every event.
    pub fn clear(&self) {
        self.lock().events.clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_ordered_and_sequence_starts_at_one() {
        let bus = EventBus::new(8);
        let first = bus.publish(PlaybackEventKind::PlaybackStarted, Duration::ZERO);
        let second = bus.publish(PlaybackEventKind::FirstFrameReady, Duration::from_millis(20));
        assert_eq!((first, second), (1, 2));
        let events = bus.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, PlaybackEventKind::PlaybackStarted);
    }

    #[test]
    fn position_events_are_coalesced_by_default() {
        let bus = EventBus::new(8);
        bus.publish(PlaybackEventKind::PositionChanged, Duration::from_millis(10));
        bus.publish(PlaybackEventKind::PositionChanged, Duration::from_millis(20));
        assert_eq!(bus.events().len(), 1);
        assert_eq!(bus.last().unwrap().position, Duration::from_millis(20));
    }

    #[test]
    fn coalescing_can_be_disabled() {
        let bus = EventBus::new(8);
        bus.set_coalesce_positions(false);
        bus.publish(PlaybackEventKind::PositionChanged, Duration::from_millis(10));
        bus.publish(PlaybackEventKind::PositionChanged, Duration::from_millis(20));
        assert_eq!(bus.events().len(), 2);
    }

    #[test]
    fn the_log_is_bounded() {
        let bus = EventBus::new(2);
        bus.publish(PlaybackEventKind::BufferingStarted, Duration::ZERO);
        bus.publish(PlaybackEventKind::BufferingEnded, Duration::ZERO);
        bus.publish(PlaybackEventKind::Ended, Duration::from_secs(1));
        let events = bus.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, PlaybackEventKind::BufferingEnded);
    }

    #[test]
    fn since_returns_only_newer_events() {
        let bus = EventBus::new(8);
        let marker = bus.publish(PlaybackEventKind::SeekStarted, Duration::ZERO);
        bus.publish(PlaybackEventKind::SeekCompleted, Duration::from_secs(1));
        let newer = bus.since(marker);
        assert_eq!(newer.len(), 1);
        assert_eq!(newer[0].kind, PlaybackEventKind::SeekCompleted);
        bus.clear();
        assert!(bus.events().is_empty());
    }
}
