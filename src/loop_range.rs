//! A repeating section of a source, for A/B study and clip review.
//!
//! [`set_looping`](crate::VideoPlayer::set_looping) repeats the whole file. A
//! language learner, a dance coach or a reviewer wants one passage to repeat
//! instead, and rewinding to the top of the file each time makes that useless.
//!
//! The range is a plain pair of positions with the rules that keep it sane: the
//! end is after the start, and a range shorter than one frame is widened so the
//! player cannot spin on a single instant.

use std::time::Duration;

/// The shortest range a player will honour, so a zero-length request still
/// advances.
pub const MIN_RANGE: Duration = Duration::from_millis(50);

/// A section of a source that repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopRange {
    start: Duration,
    end: Duration,
}

impl LoopRange {
    /// A range from `start` to `end`.
    ///
    /// The pair is ordered, so passing them the other way round still works. A
    /// span shorter than [`MIN_RANGE`] is widened to it.
    pub fn new(start: Duration, end: Duration) -> Self {
        let (start, mut end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        if end.saturating_sub(start) < MIN_RANGE {
            end = start.saturating_add(MIN_RANGE);
        }
        Self { start, end }
    }

    /// A range of `length` from `start`.
    pub fn from_length(start: Duration, length: Duration) -> Self {
        Self::new(start, start.saturating_add(length))
    }

    /// Where the range begins.
    pub fn start(&self) -> Duration {
        self.start
    }

    /// Where the range ends.
    pub fn end(&self) -> Duration {
        self.end
    }

    /// How long the range runs.
    pub fn length(&self) -> Duration {
        self.end.saturating_sub(self.start)
    }

    /// Whether `position` is inside the range, start inclusive and end
    /// exclusive.
    pub fn contains(&self, position: Duration) -> bool {
        position >= self.start && position < self.end
    }

    /// Whether `position` has reached or passed the end, so the player should
    /// jump back to the start.
    pub fn is_past_end(&self, position: Duration) -> bool {
        position >= self.end
    }

    /// Where a position inside the range sits, `0.0..=1.0`.
    pub fn fraction_at(&self, position: Duration) -> f32 {
        let length = self.length().as_secs_f64();
        if length <= 0.0 {
            return 0.0;
        }
        let offset = position.saturating_sub(self.start).as_secs_f64();
        (offset / length).clamp(0.0, 1.0) as f32
    }

    /// The range clamped to a known duration, or unchanged when the duration
    /// is unknown.
    ///
    /// A start past the end of the media collapses to the last [`MIN_RANGE`]
    /// before the end, so a stale range on a shorter file still plays.
    pub fn clamped_to(&self, duration: Option<Duration>) -> Self {
        let Some(duration) = duration else {
            return *self;
        };
        if duration <= MIN_RANGE {
            return Self::new(Duration::ZERO, duration.max(MIN_RANGE));
        }
        let start = self.start.min(duration.saturating_sub(MIN_RANGE));
        let end = self.end.min(duration);
        Self::new(start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pair_is_ordered() {
        let range = LoopRange::new(Duration::from_secs(9), Duration::from_secs(3));
        assert_eq!(range.start(), Duration::from_secs(3));
        assert_eq!(range.end(), Duration::from_secs(9));
    }

    #[test]
    fn a_short_span_is_widened() {
        let range = LoopRange::new(Duration::from_secs(2), Duration::from_secs(2));
        assert_eq!(range.length(), MIN_RANGE);
    }

    #[test]
    fn membership_is_start_inclusive_end_exclusive() {
        let range = LoopRange::new(Duration::from_secs(1), Duration::from_secs(2));
        assert!(range.contains(Duration::from_secs(1)));
        assert!(range.contains(Duration::from_millis(1999)));
        assert!(!range.contains(Duration::from_secs(2)));
        assert!(range.is_past_end(Duration::from_secs(2)));
    }

    #[test]
    fn the_fraction_runs_across_the_range() {
        let range = LoopRange::new(Duration::from_secs(10), Duration::from_secs(20));
        assert_eq!(range.fraction_at(Duration::from_secs(10)), 0.0);
        assert_eq!(range.fraction_at(Duration::from_secs(15)), 0.5);
        assert_eq!(range.fraction_at(Duration::from_secs(30)), 1.0);
    }

    #[test]
    fn clamping_pulls_a_stale_range_inside_the_media() {
        let range = LoopRange::new(Duration::from_secs(50), Duration::from_secs(60));
        let clamped = range.clamped_to(Some(Duration::from_secs(30)));
        assert!(clamped.end() <= Duration::from_secs(30));
        assert!(clamped.start() < clamped.end());
    }

    #[test]
    fn an_unknown_duration_leaves_the_range_alone() {
        let range = LoopRange::new(Duration::from_secs(5), Duration::from_secs(6));
        assert_eq!(range.clamped_to(None), range);
    }

    #[test]
    fn from_lengh_builds_forward() {
        let range = LoopRange::from_length(Duration::from_secs(4), Duration::from_secs(2));
        assert_eq!(range.start(), Duration::from_secs(4));
        assert_eq!(range.end(), Duration::from_secs(6));
    }
}
