//! How a source repeats, following the Media3 `RepeatMode` model.
//!
//! A boolean `looping` flag cannot state the three things a player UI offers:
//! stop at the end, repeat this item, or advance to the next item and repeat
//! the list. A feed that loops a single clip and a playlist that repeats all of
//! its items both "loop", so one flag cannot describe both.
//!
//! This module is that three-way state plus the decision a player makes when
//! the play head reaches the end. It is pure data: the player asks
//! [`decide`] what to do and carries it out.

use std::time::Duration;

/// What happens when the play head reaches the end of the current item.
///
/// The default is [`RepeatMode::Off`], which stops at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    /// Stop when the item ends and stay on the last frame.
    #[default]
    Off,
    /// Rewind and play the same item again.
    One,
    /// Rewind and play every item in the list again from the first.
    All,
}

impl RepeatMode {
    /// Whether this mode repeats anything at all.
    pub fn is_repeating(&self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Whether this mode repeats the same single item.
    pub fn is_single(&self) -> bool {
        matches!(self, Self::One)
    }

    /// The name to show on a repeat button.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::One => "one",
            Self::All => "all",
        }
    }

    /// The mode a repeat button moves to when pressed, in the order a viewer
    /// expects: off, then this item, then the whole list, then off again.
    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::One,
            Self::One => Self::All,
            Self::All => Self::Off,
        }
    }

    /// Whether reaching the end of an item should rewind and continue.
    ///
    /// `position` and `duration` come from the player; `has_next` says whether
    /// a playlist has another item after this one. A source with no known
    /// duration never ends, so a live stream is never rewound.
    pub fn repeats_at(
        &self,
        position: Duration,
        duration: Option<Duration>,
        has_next: bool,
    ) -> bool {
        let Some(duration) = duration else {
            return false;
        };
        if duration.is_zero() || position < duration {
            return false;
        }
        match self {
            Self::Off => false,
            // Repeating this item is exactly the same regardless of a playlist.
            Self::One => true,
            // The list repeats only once it has run out of further items.
            Self::All => !has_next,
        }
    }
}

/// What the player should do now that the play head reached the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndAction {
    /// Stop and stay on the last frame.
    Stop,
    /// Seek back to the start of this item and keep playing.
    Restart,
    /// Move on to the next item in the list.
    Advance,
}

impl EndAction {
    /// Whether playback continues after this action.
    pub fn continues(&self) -> bool {
        !matches!(self, Self::Stop)
    }
}

/// Decides what happens at the end of an item, for a player or a test.
///
/// This is the single place the repeat rule lives, so the player and the
/// public API cannot drift apart.
pub fn decide(mode: RepeatMode, has_next: bool) -> EndAction {
    match mode {
        RepeatMode::Off => {
            if has_next {
                EndAction::Advance
            } else {
                EndAction::Stop
            }
        }
        RepeatMode::One => EndAction::Restart,
        RepeatMode::All => {
            if has_next {
                EndAction::Advance
            } else {
                EndAction::Restart
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_off_and_stops() {
        let mode = RepeatMode::default();
        assert_eq!(mode, RepeatMode::Off);
        assert!(!mode.is_repeating());
        assert_eq!(decide(mode, false), EndAction::Stop);
    }

    #[test]
    fn button_cycles_through_all_three_modes() {
        assert_eq!(RepeatMode::Off.next(), RepeatMode::One);
        assert_eq!(RepeatMode::One.next(), RepeatMode::All);
        assert_eq!(RepeatMode::All.next(), RepeatMode::Off);
    }

    #[test]
    fn single_item_repeat_ignores_the_rest_of_the_list() {
        assert_eq!(decide(RepeatMode::One, true), EndAction::Restart);
        assert_eq!(decide(RepeatMode::One, false), EndAction::Restart);
    }

    #[test]
    fn repeat_all_advances_first_then_restarts() {
        assert_eq!(decide(RepeatMode::All, true), EndAction::Advance);
        assert_eq!(decide(RepeatMode::All, false), EndAction::Restart);
    }

    #[test]
    fn off_advances_in_a_playlist_and_stops_at_the_last_item() {
        assert_eq!(decide(RepeatMode::Off, true), EndAction::Advance);
        assert_eq!(decide(RepeatMode::Off, false), EndAction::Stop);
    }

    #[test]
    fn a_live_source_is_never_rewound() {
        let ten = Duration::from_secs(10);
        for mode in [RepeatMode::Off, RepeatMode::One, RepeatMode::All] {
            assert!(
                !mode.repeats_at(ten, None, false),
                "{mode:?} must not rewind a live source"
            );
        }
    }

    #[test]
    fn position_before_the_end_does_not_repeat() {
        let mode = RepeatMode::One;
        assert!(!mode.repeats_at(
            Duration::from_secs(9),
            Some(Duration::from_secs(10)),
            false
        ));
    }

    #[test]
    fn reaching_the_end_repeats_according_to_the_mode() {
        let end = Duration::from_secs(10);
        assert!(RepeatMode::One.repeats_at(end, Some(end), false));
        assert!(RepeatMode::All.repeats_at(end, Some(end), false));
        assert!(!RepeatMode::Off.repeats_at(end, Some(end), false));
        // With a further item, All advances instead of rewinding.
        assert!(!RepeatMode::All.repeats_at(end, Some(end), true));
    }

    #[test]
    fn a_zero_length_source_never_repeats() {
        let zero = Duration::ZERO;
        assert!(!RepeatMode::One.repeats_at(zero, Some(zero), false));
    }

    #[test]
    fn only_stop_ends_playback() {
        assert!(!EndAction::Stop.continues());
        assert!(EndAction::Restart.continues());
        assert!(EndAction::Advance.continues());
    }
}
