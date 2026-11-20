/// The platform this build selected a decoder for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// Windows, through Media Foundation.
    Windows,
    /// Linux, through GStreamer when the `linux-gstreamer` feature is on.
    Linux,
    /// macOS, through AVFoundation.
    MacOs,
    /// iOS, through AVFoundation.
    Ios,
    /// watchOS, which has no video decoder to select: `AVAssetReader` is absent
    /// from the platform.
    WatchOs,
    /// Android, through MediaExtractor and MediaCodec.
    Android,
    /// The browser, through a `<video>` element.
    Web,
    /// Emscripten, which has no decoder of its own.
    Emscripten,
    /// A target this crate knows nothing about.
    Unknown,
}

impl Platform {
    /// The platform as a person reads it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::MacOs => "macOS",
            Self::Ios => "iOS",
            Self::WatchOs => "watchOS",
            Self::Android => "Android",
            Self::Web => "web",
            Self::Emscripten => "Emscripten",
            Self::Unknown => "unknown",
        }
    }
}

/// The platform this build selected, decided when the crate was compiled.
///
/// Phone targets are recognized by compile-time `cfg` only: iOS through
/// `target_os = "ios"` and Android through `target_os = "android"`. There is
/// no runtime device probe.
pub(crate) const fn current() -> Platform {
    if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else if cfg!(target_os = "ios") {
        Platform::Ios
    } else if cfg!(target_os = "watchos") {
        Platform::WatchOs
    } else if cfg!(target_os = "android") {
        Platform::Android
    } else if cfg!(all(target_arch = "wasm32", target_os = "unknown")) {
        Platform::Web
    } else if cfg!(target_os = "emscripten") {
        Platform::Emscripten
    } else {
        Platform::Unknown
    }
}

#[cfg(target_os = "windows")]
mod windows;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;

#[cfg(target_os = "android")]
mod android;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;

#[cfg(all(target_os = "linux", feature = "linux-gstreamer"))]
mod linux;

#[cfg(target_os = "android")]
pub(crate) use android::{NAME, SUPPORTED, open as open_native};
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) use apple::{NAME, SUPPORTED, open as open_native};
#[cfg(all(target_os = "linux", feature = "linux-gstreamer"))]
pub(crate) use linux::{NAME, SUPPORTED, open as open_native};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) use web::{NAME, SUPPORTED, open as open_native};
#[cfg(target_os = "windows")]
pub(crate) use windows::{NAME, SUPPORTED, open as open_native};

/// The decoder a build without a platform backend reports.
#[cfg(not(any(
    target_os = "windows",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    all(target_arch = "wasm32", target_os = "unknown"),
    all(target_os = "linux", feature = "linux-gstreamer")
)))]
mod unsupported;

#[cfg(not(any(
    target_os = "windows",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    all(target_arch = "wasm32", target_os = "unknown"),
    all(target_os = "linux", feature = "linux-gstreamer")
)))]
pub(crate) use unsupported::{NAME, SUPPORTED, open as open_native};

use crate::{VideoDecoder, VideoError, VideoSource};

/// Opens a source with the decoder this build carries.
///
/// An HLS playlist is a list of fragments rather than one file, and the native
/// decoders take one file: Media Foundation refuses the byte stream and
/// Android's demuxer opens a URL as one file. So this reads such a source here,
/// as [`crate::hls`] does, and hands the platform the file it joined. The
/// browser reads HLS in its own backend, which this router leaves alone.
pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn VideoDecoder>, VideoError> {
    #[cfg(any(
        target_os = "windows",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        all(target_os = "linux", feature = "linux-gstreamer")
    ))]
    if SUPPORTED && source.is_hls() {
        return crate::hls::open(source);
    }
    open_native(source)
}
