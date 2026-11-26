#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(all(feature = "native-audio", not(target_arch = "wasm32")))]
mod audio;
// Audio-output arbitration is used by both the AVPlayer path and the generic
// SoftwareMediaPlayer path; keep it available wherever either audio owner can
// exist (not wasm, which plays in the browser).
#[cfg(not(target_arch = "wasm32"))]
mod arbitration;
mod abr;
mod audio_focus;
mod buffer;
mod cache;
mod capabilities;
mod color;
mod controller;
mod dash;
mod dash_scheduler;
mod decode_result;
mod decoder;
mod demux;
mod diagnostics;
mod drm;
mod disk_cache;
mod error;
mod events;
mod frame;
mod feed;
#[cfg(any(
    target_os = "windows",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    all(target_os = "linux", feature = "linux-gstreamer")
))]
mod hls;
mod info;
mod load_control;
mod logfile;
mod loop_range;
mod media;
mod pixels;
mod playback_params;
mod preload;
mod preload_coordinator;
mod probe;
mod platform;
mod pool;
mod player;
mod range;
mod repeat_mode;
mod resource;
mod surface;
mod segment;
#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
mod session;
mod source;
mod state;
mod stream_policy;
mod subtitle;
mod telemetry;
#[cfg(feature = "upstream-video")]
mod upstream;
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    all(target_os = "linux", feature = "linux-gstreamer")
))]
mod temp_file;
mod time_wait;
mod tracks;
#[cfg(feature = "widget")]
pub mod widget;

pub use buffer::{Buffering, BufferingState};
pub use abr::{AbrConfig, AbrController, AbrMode, Representation};
pub use audio_focus::{AudioFocus, AudioFocusState};
pub use cache::MediaCache;
pub use capabilities::{DecoderKind, VideoCapabilities, VideoCodec};
pub use color::{ChromaLocation, ColorInfo, ColorPrimaries, HdrMetadata, TransferFunction};
pub use controller::{CpuSurface, PlaybackController, VideoSurface};
pub use dash::{DashManifest, DashRepresentation, parse_mpd};
pub use dash_scheduler::{DashSegment, DashStream, choose_stream, schedule as schedule_dash};
pub use decode_result::{FrameStamp, ReadOutcome};
pub use demux::{AccessUnit as ContainerAccessUnit, DemuxError, Mp4Codec, demux_mp4};
pub use decoder::{DecodedFrame, VideoDecoder};
pub use disk_cache::{CacheMetrics, DiskCache};
pub(crate) use diagnostics::diag;
pub use diagnostics::set_diagnostic_sink;
#[cfg(feature = "upstream-video")]
pub use cranpose::PlayerSurface as VideoHardwareSurface;
#[cfg(feature = "upstream-video")]
pub use cranpose_services::{
    AccessUnit, DecoderCapabilities, DecoderConfig, DecoderError, DecoderOutput, DecodedFrames,
    DecoderStats, VideoCodec as UpstreamVideoCodec,
};
pub use drm::{
    FAIRPLAY_SYSTEM_ID, PLAYREADY_SYSTEM_ID, ProtectionInfo, ProtectionSystem,
    WIDEVINE_SYSTEM_ID, parse_hls_key, parse_protection, parse_pssh_boxes,
};
pub use error::VideoError;
pub use events::{EventBus, PlaybackEventKind, PlaybackEventRecord};
pub use feed::{
    FeedItem, FeedStage, PreloadStrategy, VideoFeedConfig, VideoFeedController,
};
pub use frame::{ColorMatrix, ColorRange, PixelFormat, VideoFrame, YuvColor};
pub use info::{FrameRate, Rotation, VideoInfo};
pub use load_control::LoadControl;
pub use logfile::{is_logging, log_note, log_path, start_log_file, stop_log_file};
pub use loop_range::{LoopRange, MIN_RANGE};
pub use media::{Container, MediaInfo};
pub use playback_params::{MAX_SPEED, MIN_SPEED, PlaybackParameters};
pub use preload::{PreloadManager, PreloadStage};
pub use preload_coordinator::{PreloadConfig, PreloadCoordinator, PreloadLevel, PreloadTask};
pub use probe::{ProbeResult, RuntimeProbe, shared as shared_probe};
pub use range::{RangeFetcher, RangeOutcome, RangeRequest};
pub use platform::Platform;
pub use player::VideoPlayer;
pub use pool::{PlayerPool, PooledPlayer};
pub use repeat_mode::RepeatMode;
pub use resource::{PrefetchGuard, Priority, ResourceLimits, VideoResourceManager};
pub use segment::{SegmentKey, SegmentScheduler, SegmentState};
#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
pub use session::{
    InterruptionReason, PlaybackSession, PlaybackState, RECOVERY_COOLDOWN, RECOVERY_COOLDOWN_MAX,
    SessionEvent,
};
pub use source::VideoSource;
pub use state::VideoState;
pub use stream_policy::{BufferSnapshot, StreamChoice, StreamPolicy};
pub use subtitle::{SubtitleCue, active_at, parse as parse_subtitles};
pub use surface::{SurfaceEvidence, SurfaceKind, SurfacePlan};
pub use telemetry::{EventRecord, PlaybackDiagnostics, PlaybackEvent, SessionId};
#[cfg(feature = "upstream-video")]
pub use upstream::HardwareVideo;
pub use tracks::{TrackInfo, TrackKind, TrackList};
#[cfg(feature = "widget")]
pub use widget::{
    Video, VideoControls, VideoFrame, VideoFrameWithPoster, VideoPlayerHandle,
    VideoProgressSlider, VideoProgressSliderWith, VideoSubtitles, VideoWithControls,
    progress_fraction_for, rememberVideoPlayer,
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
/// have no media framework. [`VieoPlayer::with_ecoder`] still works there
/// with an application-supplie [`VideoDecoder`].
pub fn is_supported() -> bool {
    platform::SUPPORTED
}
