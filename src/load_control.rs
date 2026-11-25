//! Buffering policy as pure data, following the Media3 `LoadControl` model.
//!
//! A player backend decides how much media to hold; this type gives an
//! application one place to state its preferences and to ask the same
//! questions a feed asks: should playback start, should a decoder keep filling,
//! and has the buffer stalled long enough to offer a retry. No backend is
//! touched here, so the type is usable in tests and on every target.

use std::time::Duration;

/// Bounds and thresholds for how far a player reads ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadControl {
    min_buffer: Duration,
    max_buffer: Duration,
    buffer_for_playback: Duration,
    rebuffer_threshold: Duration,
}

impl Default for LoadControl {
    fn default() -> Self {
        Self {
            min_buffer: Duration::from_millis(500),
            max_buffer: Duration::from_secs(5),
            buffer_for_playback: Duration::from_millis(250),
            rebuffer_threshold: Duration::from_secs(3),
        }
    }
}

impl LoadControl {
    /// The default policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// How much media must be ready before playback starts.
    pub fn with_min_buffer(mut self, value: Duration) -> Self {
        self.min_buffer = value;
        self
    }

    /// The most media a decoder should hold ahead of the clock.
    pub fn with_max_buffer(mut self, value: Duration) -> Self {
        self.max_buffer = value;
        self
    }

    /// How much media must remain to avoid a rebuffering pause.
    pub fn with_buffer_for_playback(mut self, value: Duration) -> Self {
        self.buffer_for_playback = value;
        self
    }

    /// How long a stall must last before an application offers a retry.
    pub fn with_rebuffer_threshold(mut self, value: Duration) -> Self {
        self.rebuffer_threshold = value;
        self
    }

    /// The configured bounds.
    pub fn min_buffer(&self) -> Duration {
        self.min_buffer
    }

    /// The most media a decoder should hold ahead of the clock.
    pub fn max_buffer(&self) -> Duration {
        self.max_buffer
    }

    /// How much media must remain to avoid a rebuffering pause.
    pub fn buffer_for_playback(&self) -> Duration {
        self.buffer_for_playback
    }

    /// How long a stall must last before an application offers a retry.
    pub fn rebuffer_threshold(&self) -> Duration {
        self.rebuffer_threshold
    }

    /// Whether enough media is ready to start playback.
    pub fn should_start(&self, buffered: Duration) -> bool {
        buffered >= self.min_buffer
    }

    /// Whether a decoder should keep reading ahead.
    pub fn should_continue_loading(&self, buffered: Duration) -> bool {
        buffered < self.max_buffer
    }

    /// Whether playback should pause to avoid running out of media.
    pub fn should_rebuffer(&self, buffered: Duration) -> bool {
        buffered < self.buffer_for_playback
    }

    /// Whether a waiting period is long enough to count as a stall.
    pub fn is_stalled(&self, waiting: Duration) -> bool {
        waiting >= self.rebuffer_threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn defaults_are_ordered() {
        let control = LoadControl::default();
        assert!(control.buffer_for_playback() <= control.min_buffer());
        assert!(control.min_buffer() <= control.max_buffer());
    }

    #[test]
    fn start_and_continue_are_bounded() {
        let control = LoadControl::default();
        assert!(!control.should_start(ms(100)));
        assert!(control.should_start(ms(500)));
        assert!(control.should_continue_loading(ms(500)));
        assert!(!control.should_continue_loading(Duration::from_secs(5)));
    }

    #[test]
    fn rebuffer_and_stall_thresholds_are_explicit() {
        let control = LoadControl::new()
            .with_buffer_for_playback(ms(200))
            .with_rebuffer_threshold(ms(900));
        assert!(control.should_rebuffer(ms(100)));
        assert!(!control.should_rebuffer(ms(200)));
        assert!(!control.is_stalled(ms(899)));
        assert!(control.is_stalled(ms(900)));
    }
}
