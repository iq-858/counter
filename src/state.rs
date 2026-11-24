use crate::VideoError;

/// What the player is doing now.
///
/// The set matches a media player's states rather than a stream's, because a
/// screen shows the same thing for a file and for a live stream and the
/// difference lives in [`VideoInfo::duration`](crate::VideoInfo::duration).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VideoState {
    /// Nothing is open.
    #[default]
    Idle,
    /// A decoder is opening the source, or filling its first frames. A state of
    /// its own because opening a remote file takes long enough that a screen
    /// has to say so.
    Loading,
    /// Frames are moving.
    Playing,
    /// A source is open and positioned, and frames are still.
    Paused,
    /// The last frame was shown and playback stopped at the end.
    Ended,
    /// The source could not be opened, or reading stopped on a failure.
    Failed(VideoError),
}

impl VideoState {
    /// Whether frames are moving now.
    pub fn is_playing(&self) -> bool {
        matches!(self, Self::Playing)
    }

    /// Whether a source is open, moving or not.
    pub fn is_open(&self) -> bool {
        matches!(
            self,
            Self::Loading | Self::Playing | Self::Paused | Self::Ended
        )
    }

    /// Whether the player stopped at the end of the media.
    pub fn is_ended(&self) -> bool {
        matches!(self, Self::Ended)
    }

    /// The failure, when playback stopped on one.
    pub fn failure(&self) -> Option<&VideoError> {
        match self {
            Self::Failed(error) => Some(error),
            Self::Idle | Self::Loading | Self::Playing | Self::Paused | Self::Ended => None,
        }
    }
}
