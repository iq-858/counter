/// What can stop a video player.
///
/// A failure reaches the caller twice: the operation that fails at once
/// returns it, and [`VideoState::Failed`](crate::VideoState::Failed) holds a
/// copy for every later reader. A decode thread that hits one stops reading and
/// keeps the last frame it published on screen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VideoError {
    /// This build has no decoder for the target platform.
    #[error("no video decoder for {platform}")]
    Unsupported {
        /// The platform, as [`Platform`](crate::Platform) names it.
        platform: &'static str,
    },
    /// The source could not be opened or read.
    #[error("cannot read the source: {0}")]
    Source(String),
    /// The media opened, and holds no video track.
    #[error("the media has no video track")]
    NoVideoTrack,
    /// The platform decoder refused a sample.
    #[error("the decoder failed: {0}")]
    Decode(String),
    /// The backend cannot do what was asked, such as a backward seek in a
    /// stream that only plays forward.
    #[error("{operation} is not available on this backend")]
    UnsupportedOperation {
        /// The operation, as the caller named it.
        operation: &'static str,
    },
    /// The player was closed and its thread has ended.
    #[error("the player is closed")]
    Closed,
}

impl VideoError {
    /// The platform decoder's own message, when the failure came from one.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Source(reason) | Self::Decode(reason) => Some(reason),
            Self::Unsupported { .. }
            | Self::NoVideoTrack
            | Self::UnsupportedOperation { .. }
            | Self::Closed => None,
        }
    }

    /// Whether the same call could succeed later, because the failure came
    /// from a source that may recover.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Source(_))
    }

    /// A copy safe to keep in a public history or field log.
    ///
    /// A source/decoder message can quote a live URL whose query carries a
    /// token. The variant is preserved so callers can still match on it; only
    /// the free-form text is sanitized.
    pub(crate) fn redacted(&self) -> Self {
        match self {
            Self::Source(reason) => Self::Source(crate::diagnostics::redact_for_report(reason)),
            Self::Decode(reason) => Self::Decode(crate::diagnostics::redact_for_report(reason)),
            other => other.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_redacted_error_keeps_its_variant_and_drops_the_token() {
        let error = VideoError::Source(
            "cannot read https://user:secret@host.example/live.m3u8?token=abc".into(),
        );
        let redacted = error.redacted();
        assert_eq!(
            redacted,
            VideoError::Source("cannot read https://***@host.example/live.m3u8?<redacted>".into())
        );
        // Non-text variants pass through unchanged so callers can still match.
        assert_eq!(
            VideoError::NoVideoTrack.redacted(),
            VideoError::NoVideoTrack
        );
    }
}
