//! Playback rate with pitch correction, following Media3 `PlaybackParameters`.
//!
//! A speed multiplier alone is enough for video: the picture is frames, so
//! playing at `2.0` simply shows them faster. A source that carries audio is
//! different, because speeding up an audio stream raises its pitch and the
//! result sounds like a tape played too fast. A viewer expects speed without
//! chipmunk voices, so rate and pitch are two separate numbers.
//!
//! This module holds that pair and the range a player accepts. It is pure data,
//! so a caller can validate a speed before touching a decoder, and a UI can
//! label the pitch state without knowing which backend runs underneath.

use std::fmt;

/// The lowest rate a player accepts.
pub const MIN_SPEED: f32 = 0.25;

/// The highest rate a player accepts.
pub const MAX_SPEED: f32 = 4.0;

/// Rate and pitch, the pair a player needs to play a clip faster or slower
/// without changing how voices sound.
///
/// `speed` is how fast the media advances. `pitch` is the frequency multiplier
/// applied to audio: `1.0` keeps the original pitch, which is what a viewer
/// expects from a speed control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackParameters {
    speed: f32,
    pitch: f32,
}

impl Default for PlaybackParameters {
    fn default() -> Self {
        Self {
            speed: 1.0,
            pitch: 1.0,
        }
    }
}

impl PlaybackParameters {
    /// Normal speed at the original pitch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds the pair, clamping both into the accepted range.
    ///
    /// Clamping rather than refusing keeps a slider safe: a caller that reads a
    /// value slightly outside the range still gets a usable player instead of
    /// an error at the wrong moment.
    pub fn with_speed(speed: f32) -> Self {
        Self {
            speed: speed.clamp(MIN_SPEED, MAX_SPEED),
            pitch: 1.0,
        }
    }

    /// Builds the pair with an independent pitch multiplier.
    pub fn from_parts(speed: f32, pitch: f32) -> Self {
        Self {
            speed: speed.clamp(MIN_SPEED, MAX_SPEED),
            pitch: pitch.clamp(MIN_SPEED, MAX_SPEED),
        }
    }

    /// The rate the media advances at.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// The audio frequency multiplier.
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// Whether this is normal speed at the original pitch.
    pub fn is_default(&self) -> bool {
        (self.speed - 1.0).abs() < f32::EPSILON && (self.pitch - 1.0).abs() < f32::EPSILON
    }

    /// Whether the sound is a normal voice, which is what a player reports to
    /// a backend that can preserve pitch.
    pub fn preserves_pitch(&self) -> bool {
        (self.pitch - 1.0).abs() < 0.001
    }

    /// The same speed with pitch preservation turned on.
    pub fn at_speed(speed: f32) -> Self {
        Self::with_speed(speed)
    }

    /// Whether a raw value is inside the accepted range.
    pub fn is_speed_supported(speed: f32) -> bool {
        speed.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(&speed)
    }
}

impl fmt::Display for PlaybackParameters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.preserves_pitch() {
            write!(f, "{:.2}x", self.speed)
        } else {
            write!(f, "{:.2}x (pitch {:.2})", self.speed, self.pitch)
        }
    }
}

/// Buffer thresholds, following the Media3 `LoadControl` idea.
///
/// A player that starts as soon as it has one frame stalls constantly on a slow
/// connection, and a player that waits for the whole file feels broken. The
/// three numbers here are the trade-off: how much to hold before starting, how
/// much to keep ahead while playing, and how little is left when playback
/// pauses to refill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadControl {
    /// Media to hold before playback may start.
    pub min_buffer: std::time::Duration,
    /// Media to keep ahead of the play head while playing.
    pub max_buffer: std::time::Duration,
    /// The level that triggers a refill pause.
    pub buffer_for_playback: std::time::Duration,
}

impl Default for LoadControl {
    fn default() -> Self {
        use std::time::Duration;
        Self {
            min_buffer: Duration::from_millis(2500),
            max_buffer: Duration::from_secs(30),
            buffer_for_playback: Duration::from_millis(500),
        }
    }
}

impl LoadControl {
    /// Thresholds for a short-form feed: start almost immediately, keep little.
    pub fn for_feed() -> Self {
        use std::time::Duration;
        Self {
            min_buffer: Duration::from_millis(500),
            max_buffer: Duration::from_secs(5),
            buffer_for_playback: Duration::from_millis(250),
        }
    }

    /// Thresholds for long-form streaming: wait longer, hold more.
    pub fn for_streaming() -> Self {
        use std::time::Duration;
        Self {
            min_buffer: Duration::from_secs(5),
            max_buffer: Duration::from_secs(120),
            buffer_for_playback: Duration::from_secs(2),
        }
    }

    /// A configuration whose `min_buffer` never exceeds `max_buffer`.
    ///
    /// A caller that passes the pair the other way round still gets a usable
    /// player instead of a player that can never start.
    pub fn sanitized(&self) -> Self {
        let max = self.max_buffer.max(self.min_buffer);
        let for_playback = self.buffer_for_playback.min(max);
        Self {
            min_buffer: self.min_buffer.min(max),
            max_buffer: max,
            buffer_for_playback: for_playback,
        }
    }

    /// Whether enough media is held to start.
    pub fn can_start(&self, buffered: std::time::Duration) -> bool {
        buffered >= self.min_buffer
    }

    /// Whether enough media is held to keep playing without a stall.
    pub fn can_continue(&self, buffered: std::time::Duration) -> bool {
        buffered >= self.buffer_for_playback
    }

    /// Whether the player should keep filling.
    pub fn should_fill(&self, buffered: std::time::Duration) -> bool {
        buffered < self.max_buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn default_is_normal_speed_and_pitch() {
        let params = PlaybackParameters::default();
        assert_eq!(params.speed(), 1.0);
        assert_eq!(params.pitch(), 1.0);
        assert!(params.is_default());
        assert!(params.preserves_pitch());
    }

    #[test]
    fn speed_is_clamped_into_the_supported_range() {
        assert_eq!(PlaybackParameters::with_speed(0.01).speed(), MIN_SPEED);
        assert_eq!(PlaybackParameters::with_speed(99.0).speed(), MAX_SPEED);
        assert_eq!(PlaybackParameters::with_speed(2.0).speed(), 2.0);
    }

    #[test]
    fn changing_speed_keeps_the_original_pitch() {
        let params = PlaybackParameters::at_speed(2.0);
        assert_eq!(params.pitch(), 1.0);
        assert!(
            params.preserves_pitch(),
            "a speed control must not raise the pitch"
        );
        assert!(!params.is_default());
    }

    #[test]
    fn an_independent_pitch_is_reported_as_such() {
        let params = PlaybackParameters::from_parts(1.0, 1.25);
        assert!(!params.preserves_pitch());
        assert_eq!(params.to_string(), "1.00x (pitch 1.25)");
    }

    #[test]
    fn display_shows_a_plain_factor_when_pitch_is_normal() {
        assert_eq!(PlaybackParameters::at_speed(1.5).to_string(), "1.50x");
    }

    #[test]
    fn supported_speed_rejects_nan_and_out_of_range() {
        assert!(PlaybackParameters::is_speed_supported(1.0));
        assert!(PlaybackParameters::is_speed_supported(MIN_SPEED));
        assert!(PlaybackParameters::is_speed_supported(MAX_SPEED));
        assert!(!PlaybackParameters::is_speed_supported(0.1));
        assert!(!PlaybackParameters::is_speed_supported(10.0));
        assert!(!PlaybackParameters::is_speed_supported(f32::NAN));
    }

    #[test]
    fn default_load_control_can_start_after_min_buffer() {
        let control = LoadControl::default();
        assert!(!control.can_start(Duration::from_millis(100)));
        assert!(control.can_start(control.min_buffer));
    }

    #[test]
    fn feed_profile_starts_sooner_than_the_streaming_profile() {
        let feed = LoadControl::for_feed();
        let streaming = LoadControl::for_streaming();
        assert!(feed.min_buffer < streaming.min_buffer);
        assert!(feed.max_buffer < streaming.max_buffer);
    }

    #[test]
    fn inverted_thresholds_are_sanitized() {
        let control = LoadControl {
            min_buffer: Duration::from_secs(10),
            max_buffer: Duration::from_secs(2),
            buffer_for_playback: Duration::from_secs(5),
        }
        .sanitized();
        assert!(control.min_buffer <= control.max_buffer);
        assert!(control.buffer_for_playback <= control.max_buffer);
        assert_eq!(control.max_buffer, Duration::from_secs(10));
    }

    #[test]
    fn filling_stops_at_the_maximum() {
        let control = LoadControl::default();
        assert!(control.should_fill(Duration::from_secs(1)));
        assert!(!control.should_fill(control.max_buffer));
    }

    #[test]
    fn playing_continues_above_the_playback_threshold() {
        let control = LoadControl::default();
        assert!(!control.can_continue(Duration::from_millis(10)));
        assert!(control.can_continue(control.buffer_for_playback));
    }
}
