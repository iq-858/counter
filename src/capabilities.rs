//! Build-time decoder capabilities, reported honestly.
//!
//! This is **not** a device probe. It reports what the selected platform
//! backend can be asked to decode; whether a particular device actually exposes
//! a codec depends on its hardware, drivers and installed plugins. Use it to
//! choose a sensible starting point, then let a failed open fall back.

use crate::{Platform, is_supported, platform};

/// Codecs this crate's API can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    H264,
    Hevc,
    Vp9,
    Av1,
    Mpeg2,
}

/// How a backend can obtain decoded pictures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecoderKind {
    /// The platform's own hardware-assisted media stack.
    Platform,
    /// A software decoder supplied by the platform's media framework.
    Software,
    /// An application-supplied decoder through `VideoPlayer::with_decoder`.
    Application,
    /// Nothing is available in this build.
    None,
}

/// What the selected backend reports about decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoCapabilities {
    platform: Platform,
    supported: bool,
    codecs: &'static [VideoCodec],
}

const WINDOWS: &[VideoCodec] = &[
    VideoCodec::H264,
    VideoCodec::Hevc,
    VideoCodec::Vp9,
    VideoCodec::Av1,
    VideoCodec::Mpeg2,
];
const APPLE: &[VideoCodec] = &[VideoCodec::H264, VideoCodec::Hevc];
const ANDROID: &[VideoCodec] = &[
    VideoCodec::H264,
    VideoCodec::Hevc,
    VideoCodec::Vp9,
    VideoCodec::Av1,
];
const WEB: &[VideoCodec] = &[
    VideoCodec::H264,
    VideoCodec::Vp9,
    VideoCodec::Av1,
];
const GSTREAMER: &[VideoCodec] = &[
    VideoCodec::H264,
    VideoCodec::Hevc,
    VideoCodec::Vp9,
    VideoCodec::Av1,
    VideoCodec::Mpeg2,
];
const NONE: &[VideoCodec] = &[];

impl VideoCapabilities {
    /// Capabilities of the backend selected in this build.
    pub fn detect() -> Self {
        let platform = platform();
        let supported = is_supported();
        let codecs = match platform {
            Platform::Windows if supported => WINDOWS,
            Platform::MacOs | Platform::Ios if supported => APPLE,
            Platform::Android if supported => ANDROID,
            Platform::Web if supported => WEB,
            Platform::Linux if supported => GSTREAMER,
            _ => NONE,
        };
        Self {
            platform,
            supported,
            codecs,
        }
    }

    /// The platform whose backend this build selected.
    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// Whether any decoder backend is available at all.
    pub fn is_supported(&self) -> bool {
        self.supported
    }

    /// Codecs the selected backend can be asked to decode.
    pub fn supported_codecs(&self) -> &'static [VideoCodec] {
        self.codecs
    }

    /// Whether the backend can be asked to decode `codec`.
    pub fn supports_codec(&self, codec: VideoCodec) -> bool {
        self.codecs.contains(&codec)
    }

    /// The backend families this build can use.
    pub fn decoders(&self) -> &'static [DecoderKind] {
        if !self.supported {
            return &[DecoderKind::None];
        }
        &[DecoderKind::Platform, DecoderKind::Application]
    }

    /// Whether this build can use an application-supplied decoder.
    pub fn accepts_custom_decoder(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_matches_the_selected_platform() {
        let caps = VideoCapabilities::detect();
        assert_eq!(caps.platform(), platform());
        assert_eq!(caps.is_supported(), is_supported());
        assert_eq!(caps.accepts_custom_decoder(), true);
    }

    #[test]
    fn an_unsupported_build_reports_no_codecs() {
        let caps = VideoCapabilities::detect();
        if !caps.is_supported() {
            assert!(caps.supported_codecs().is_empty());
            assert_eq!(caps.decoders(), &[DecoderKind::None]);
        }
    }

    #[test]
    fn codec_names_are_distinct() {
        let all = [
            VideoCodec::H264,
            VideoCodec::Hevc,
            VideoCodec::Vp9,
            VideoCodec::Av1,
            VideoCodec::Mpeg2,
        ];
        for (index, codec) in all.iter().enumerate() {
            assert!(!all[index + 1..].contains(codec));
        }
    }
}
