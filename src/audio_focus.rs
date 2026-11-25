//! Audio focus as pure state, following the Android `AudioFocus` model.
//!
//! A platform owns the actual focus request (Android `AudioManager`, iOS
//! `AVAudioSession`, desktop system mixers). This module gives an application a
//! small state machine it can drive from those callbacks: whether to keep
//! playing, duck the volume, or pause while another app briefly owns sound.
//! It touches no platform API and is therefore testable on every target.

/// What another participant in the system is doing with audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioFocus {
    /// No other participant holds focus.
    #[default]
    None,
    /// Another app plays briefly, so this player should duck but may continue.
    Transient,
    /// Like `Transient`, with an explicit permission to keep playing quietly.
    TransientMayDuck,
    /// Another app owns sound permanently; this player should pause.
    Permanent,
}

impl AudioFocus {
    /// Whether playback should be paused while this focus is held.
    pub fn should_pause(&self) -> bool {
        matches!(self, Self::Permanent)
    }

    /// Whether the volume should be reduced while this focus is held.
    pub fn should_duck(&self) -> bool {
        matches!(self, Self::Transient | Self::TransientMayDuck)
    }

    /// The volume multiplier an application should apply.
    pub fn volume_multiplier(&self, duck_factor: f32) -> f32 {
        if self.should_duck() {
            duck_factor.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// Whether returning to `None` should resume a player that ducked.
    pub fn resumes_after(&self) -> bool {
        !matches!(self, Self::Permanent)
    }
}

/// A small state holder for the focus an application has currently accepted.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AudioFocusState {
    current: AudioFocus,
    duck_factor: f32,
    paused: bool,
}

impl AudioFocusState {
    /// A state holder that ducks to `duck_factor` on transient focus.
    pub fn new(duck_factor: f32) -> Self {
        Self {
            duck_factor: duck_factor.clamp(0.0, 1.0),
            ..Self::default()
        }
    }

    /// The last focus change this holder accepted.
    pub fn current(&self) -> AudioFocus {
        self.current
    }

    /// Whether the holder asks the player to pause now.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// The volume multiplier the player should use now.
    pub fn volume_multiplier(&self) -> f32 {
        self.current.volume_multiplier(self.duck_factor)
    }

    /// Applies a focus change and returns whether playback should pause.
    pub fn apply(&mut self, focus: AudioFocus) -> bool {
        self.current = focus;
        self.paused = focus.should_pause();
        self.paused
    }

    /// Applies a focus loss, resuming only a transiently ducked player.
    pub fn abandon(&mut self) -> bool {
        let resume = self.current.resumes_after();
        self.current = AudioFocus::None;
        self.paused = false;
        resume
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_focus_ducks_without_pausing() {
        let focus = AudioFocus::Transient;
        assert!(!focus.should_pause());
        assert!(focus.should_duck());
        assert_eq!(focus.volume_multiplier(0.25), 0.25);
    }

    #[test]
    fn permanent_focus_pauses() {
        assert!(AudioFocus::Permanent.should_pause());
        assert!(!AudioFocus::Permanent.should_duck());
        assert!(!AudioFocus::Permanent.resumes_after());
    }

    #[test]
    fn state_holder_tracks_pause_and_resume() {
        let mut state = AudioFocusState::new(0.2);
        assert!(!state.apply(AudioFocus::Transient));
        assert_eq!(state.volume_multiplier(), 0.2);
        assert!(state.abandon());
        assert_eq!(state.current(), AudioFocus::None);
        assert_eq!(state.volume_multiplier(), 1.0);
        assert!(state.apply(AudioFocus::Permanent));
        assert!(!state.abandon());
    }
}
