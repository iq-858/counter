//! Buffering state, so a screen can show a spinner and a percentage.
//!
//! A network source stalls: the socket drains, the server throttles, or the
//! decoder has to fill a window before it can hand out a picture. A player that
//! only reports `Loading` cannot say how far along that is, so a screen either
//! shows a bare spinner or nothing at all.
//!
//! This module is the state a backend reports and a screen reads. It is pure
//! data with no clock of its own: the backend stamps the time it last changed,
//! and [`Buffering::is_stalled`] compares that against a caller-supplied
//! instant, so the type stays testable without sleeping.

use std::time::{Duration, Instant};

/// What the decoder is doing about media it does not have yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BufferingState {
    /// Enough media is in hand; playback is not waiting on the network.
    #[default]
    Idle,
    /// The decoder is filling its window before it can continue.
    Buffering,
    /// The decoder has media in hand and is decoding ahead of the clock.
    Filling,
}

impl BufferingState {
    /// Whether playback is waiting on media right now.
    pub fn is_waiting(&self) -> bool {
        matches!(self, Self::Buffering)
    }
}

/// How far a buffer has filled, and since when.
///
/// `fraction` is `0.0..=1.0` of the window the backend wants. A backend that
/// cannot state a fraction leaves it at `0.0` and the screen shows an
/// indeterminate spinner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Buffering {
    state: BufferingState,
    fraction: f32,
    since: Option<Instant>,
}

impl Default for Buffering {
    fn default() -> Self {
        Self {
            state: BufferingState::Idle,
            fraction: 0.0,
            since: None,
        }
    }
}

impl Buffering {
    /// A fresh idle buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// What the decoder is doing.
    pub fn state(&self) -> BufferingState {
        self.state
    }

    /// How much of the window is filled, `0.0..=1.0`.
    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    /// The percentage a progress ring wants, `0..=100`.
    pub fn percent(&self) -> u8 {
        (self.fraction.clamp(0.0, 1.0) * 100.0).round() as u8
    }

    /// When the state last changed.
    pub fn since(&self) -> Option<Instant> {
        self.since
    }

    /// How long the current state has held, or `None` while idle.
    pub fn held_for(&self, now: Instant) -> Option<Duration> {
        self.since
            .map(|since| now.saturating_duration_since(since))
    }

    /// Whether playback has been waiting longer than `limit`, which is when a
    /// screen should offer a retry instead of an endless spinner.
    pub fn is_stalled(&self, now: Instant, limit: Duration) -> bool {
        self.state.is_waiting()
            && self
                .held_for(now)
                .is_some_and(|held| held >= limit)
    }

    /// Records a new state and fraction, stamping the change time.
    ///
    /// A fraction outside `0.0..=1.0` is clamped; a non-finite one reads as
    /// `0.0`. Repeating the same state and fraction keeps the original stamp,
    /// so a backend that reports every frame does not reset the stall timer.
    pub fn set(&mut self, state: BufferingState, fraction: f32) {
        let fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if self.state == state && self.fraction == fraction {
            return;
        }
        self.state = state;
        self.fraction = fraction;
        self.since = Some(Instant::now());
    }

    /// Records a state with no known fraction.
    pub fn set_state(&mut self, state: BufferingState) {
        self.set(state, 0.0);
    }

    /// Returns to idle, which clears the stall timer.
    pub fn clear(&mut self) {
        self.state = BufferingState::Idle;
        self.fraction = 0.0;
        self.since = None;
    }

    /// Whether the buffer is idle.
    pub fn is_idle(&self) -> bool {
        self.state == BufferingState::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_buffer_is_idle() {
        let buffer = Buffering::new();
        assert!(buffer.is_idle());
        assert_eq!(buffer.percent(), 0);
        assert_eq!(buffer.since(), None);
    }

    #[test]
    fn a_fraction_is_clamped_and_percent_rounds() {
        let mut buffer = Buffering::new();
        buffer.set(BufferingState::Buffering, 0.456);
        assert_eq!(buffer.percent(), 46);
        buffer.set(BufferingState::Buffering, 4.0);
        assert_eq!(buffer.fraction(), 1.0);
        buffer.set(BufferingState::Buffering, f32::NAN);
        assert_eq!(buffer.fraction(), 0.0);
    }

    #[test]
    fn repeating_the_same_value_keeps_the_stamp() {
        let mut buffer = Buffering::new();
        buffer.set(BufferingState::Buffering, 0.5);
        let first = buffer.since();
        buffer.set(BufferingState::Buffering, 0.5);
        assert_eq!(buffer.since(), first);
        buffer.set(BufferingState::Buffering, 0.6);
        assert_ne!(buffer.since(), first);
    }

    #[test]
    fn a_long_wait_is_a_stall() {
        let mut buffer = Buffering::new();
        buffer.set_state(BufferingState::Buffering);
        let now = Instant::now();
        assert!(!buffer.is_stalled(now, Duration::from_secs(5)));
        assert!(buffer.is_stalled(now + Duration::from_secs(6), Duration::from_secs(5)));
    }

    #[test]
    fn an_idle_buffer_never_stalls() {
        let buffer = Buffering::new();
        let now = Instant::now();
        assert!(!buffer.is_stalled(now + Duration::from_secs(60), Duration::from_secs(1)));
    }

    #[test]
    fn clearing_returns_to_idle() {
        let mut buffer = Buffering::new();
        buffer.set(BufferingState::Buffering, 0.3);
        buffer.clear();
        assert!(buffer.is_idle());
        assert_eq!(buffer.since(), None);
    }
}
