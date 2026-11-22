#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(all(feature = "native-audio", not(target_arch = "wasm32")))]
mod audio;
// Audio-output arbitration is used by both the AVPlayer path and the generic
// SoftwareMediaPlayer path; keep it available wherever either audio owner can
// exist (not wasm, which plays in the browser).
#[cfg(not(target_arch = "wasm32"))]
mod arbitration;
mod decoder;
mod diagnostics;
mod error;
mod frame;
#[cfg(any(
    target_os = "windows",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    all(target_os = "linux", feature = "linux-gstreamer")
))]
mod hls;
mod info;
mod logfile;
mod pixels;
mod platform;
mod player;
#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
mod session;
mod source;
mod state;
mod telemetry;
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    all(target_os = "linux", feature = "linux-gstreamer")
))]
mod temp_file;
mod time_wait;
#[cfg(feature = "widget")]
pub mod widget;

pub use decoder::{DecodedFrame, VideoDecoder};
pub(crate) use diagnostics::diag;
pub use diagnostics::set_diagnostic_sink;
pub use error::VideoError;
pub use frame::{ColorMatrix, ColorRange, PixelFormat, VideoFrame, YuvColor};
pub use info::{FrameRate, Rotation, VideoInfo};
pub use logfile::{is_logging, log_note, log_path, start_log_file, stop_log_file};
pub use platform::Platform;
pub use player::VideoPlayer;
#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
pub use session::{
    InterruptionReason, PlaybackSession, PlaybackState, RECOVERY_COOLDOWN, RECOVERY_COOLDOWN_MAX,
    SessionEvent,
};
pub use source::VideoSource;
pub use state::VideoState;
pub use telemetry::{EventRecord, PlaybackDiagnostics, PlaybackEvent, SessionId};
#[cfg(feature = "widget")]
pub use widget::{
    GlassStyle, ProControl, Video, VideoControls, VideoControlsLiquid, VideoControlsOptions,
    VideoControlsPro, VideoLiquidAction, VideoLiquidIconButton, VideoLoopButton, VideoMuteButton,
    VideoPlayerHandle, VideoProgressSlider, VideoProgressSliderWith, VideoSpeedButton,
    VideoStopButton, VideoVolumeSlider, VideoWithControls, progress_fraction_for,
    rememberVideoPlayer,
};

/// The platform whose decoder this build selected.
///
/// The choice is made at compile time from the target's operating system, so a
/// single source tree carries one backend per platform and no runtime probe.
pub fn platform() -> Platform {
    platform::current()
}

/// The name of the decoder this build selected, for logs and about screens.
pub fn decoder_name() -> &'static str {
    platform::NAME
}

/// Whether this build can decode video on the target it was compiled for.
///
/// False on Linux without the `linux-gstreamer` feature, and on targets that
/// have no media framework. [`VideoPlayer::with_decoder`] still works there
/// with an application-supplied [`VideoDecoder`].
pub fn is_supported() -> bool {
    platform::SUPPORTED
}
