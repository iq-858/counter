//! Explicit decoder read outcomes.
//!
//! Historically `VideoDecoder::read` returned `Ok(None)` for "end of media".
//! Real backends need to say more: a live edge that is buffering, a codec that
//! needs more compressed input, or a picture that is simply not ready yet.
//! Collapsing all of those into `None` made the player treat a temporary stall
//! as a terminal end — the freeze/restart cycle seen on live HLS.
//!
//! This module adds a typed outcome that a backend MAY return through a new
//! defaulted trait method (`read_outcome`). Existing decoders keep working: the
//! default derives the outcome from the legacy `read()` result, so nothing
//! breaks and no platform code has to change on day one.

use std::time::Duration;

use crate::DecodedFrame;

/// Why a decoder call did not produce a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    /// A picture is available now (`Some(frame)`).
    Frame(DecodedFrame),
    /// The media ended for good. A seek or restart is required to continue.
    EndOfStream,
    /// The decoder has consumed everything and needs more compressed input.
    /// The worker should keep pulling (a network backend fetches the next
    /// fragment) rather than declaring the stream finished.
    NeedMoreInput,
    /// A picture will arrive shortly (live buffering / a not-yet-ready backend).
    /// The worker should wait and retry, not stop.
    NoPictureYet,
}

impl ReadOutcome {
    /// The frame, if this outcome carries one.
    pub fn frame(self) -> Option<DecodedFrame> {
        match self {
            Self::Frame(frame) => Some(frame),
            _ => None,
        }
    }

    /// Whether the worker should keep trying instead of ending playback.
    pub fn should_continue(self) -> bool {
        matches!(self, Self::NeedMoreInput | Self::NoPictureYet)
    }

    /// Whether this is the terminal end of media.
    pub fn is_end(self) -> bool {
        matches!(self, Self::EndOfStream)
    }

    /// Converts the legacy `Option<DecodedFrame>` result.
    ///
    /// `Some(frame)` → `Frame`, `None` → `EndOfStream` — exactly the old
    /// semantics, so a decoder that only implements `read()` behaves as before.
    pub fn from_legacy(result: Option<DecodedFrame>) -> Self {
        match result {
            Some(frame) => Self::Frame(frame),
            None => Self::EndOfStream,
        }
    }
}

/// A frame's identity: presentation time and size, for telemetry and dedup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameStamp {
    /// Presentation timestamp of the frame.
    pub pts: Duration,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
}

impl From<DecodedFrame> for FrameStamp {
    fn from(frame: DecodedFrame) -> Self {
        Self {
            pts: frame.pts,
            width: frame.width,
            height: frame.height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(pts: Duration) -> DecodedFrame {
        DecodedFrame {
            pts,
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn legacy_some_becomes_frame() {
        let out = ReadOutcome::from_legacy(Some(frame(Duration::from_secs(1))));
        assert_eq!(out.frame().map(|f| f.pts), Some(Duration::from_secs(1)));
        assert!(!out.is_end());
        assert!(!out.should_continue());
    }

    #[test]
    fn legacy_none_becomes_end() {
        let out = ReadOutcome::from_legacy(None);
        assert!(out.is_end());
        assert!(!out.should_continue());
    }

    #[test]
    fn temporary_states_continue() {
        assert!(ReadOutcome::NeedMoreInput.should_continue());
        assert!(ReadOutcome::NoPictureYet.should_continue());
        assert!(!ReadOutcome::EndOfStream.should_continue());
    }

    #[test]
    fn stamp_keeps_identity() {
        let stamp = FrameStamp::from(frame(Duration::from_millis(40)));
        assert_eq!(stamp.pts, Duration::from_millis(40));
        assert_eq!((stamp.width, stamp.height), (1920, 1080));
    }
}
