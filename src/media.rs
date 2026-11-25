//! What a source is, beyond its picture size.
//!
//! [`VideoInfo`](crate::VideoInfo) answers what a screen needs to lay a picture
//! out. An about screen, a quality picker and a bug report need more: the
//! container, the codecs, the bitrate and the tracks. This module carries that
//! as one value a backend fills once at open time.
//!
//! Pure Rust: no decoder, no platform call.

use std::time::Duration;

use crate::tracks::{TrackInfo, TrackKind};

/// The container format a source uses, as far as the backend can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Container {
    /// MP4 and its fragmented form.
    Mp4,
    /// Matroska / WebM.
    Matroska,
    /// An MPEG transport stream, which is what HLS fragments are.
    MpegTs,
    /// A raw elementary stream.
    Elementary,
    /// A format the backend did not name.
    #[default]
    Unknown,
}

impl Container {
    /// The container as a person reads it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Matroska => "Matroska",
            Self::MpegTs => "MPEG-TS",
            Self::Elementary => "elementary stream",
            Self::Unknown => "unknown container",
        }
    }
}

/// What a backend learned about a source when it opened it.
///
/// Every field is best-effort: a backend that cannot read one leaves it empty
/// rather than failing the open. An empty `MediaInfo` is valid and means the
/// backend states nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaInfo {
    /// The container, when the backend recognises it.
    pub container: Container,
    /// The video codec the container names, such as `"h264"` or `"hevc"`.
    pub video_codec: Option<String>,
    /// The audio codec of the selected track, such as `"aac"`.
    pub audio_codec: Option<String>,
    /// The overall bitrate in bits per second, when the container states one.
    pub bitrate: Option<u64>,
    /// How long the media runs, when it is finite.
    pub duration: Option<Duration>,
    /// Every track the source carries, audio and subtitle, in container order.
    pub tracks: Vec<TrackInfo>,
}

impl MediaInfo {
    /// Whether the backend stated nothing at all about the source.
    ///
    /// True for the default value, which a decoder that reads one elementary
    /// stream returns. A player uses it to leave the track lists empty instead
    /// of replacing them with nothing.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
            && self.container == Container::Unknown
            && self.video_codec.is_none()
            && self.audio_codec.is_none()
            && self.bitrate.is_none()
    }

    /// The tracks of one kind.
    pub fn tracks_of(&self, kind: TrackKind) -> impl Iterator<Item = &TrackInfo> {
        self.tracks.iter().filter(move |track| track.kind == kind)
    }

    /// How many tracks of one kind the source carries.
    pub fn count_of(&self, kind: TrackKind) -> usize {
        self.tracks_of(kind).count()
    }

    /// The bitrate in kilobits per second, for a display that wants it.
    pub fn kilobit_rate(&self) -> Option<f64> {
        self.bitrate
            .map(|rate| rate as f64 / 1000.0)
            .filter(|rate| rate.is_finite())
    }

    /// One line that names the source's shape, for a log or an about screen.
    pub fn summary(&self) -> String {
        let mut parts = vec![self.container.name().to_owned()];
        if let Some(codec) = &self.video_codec {
            parts.push(format!("video {codec}"));
        }
        if let Some(codec) = &self.audio_codec {
            parts.push(format!("audio {codec}"));
        }
        if let Some(rate) = self.kilobit_rate() {
            parts.push(format!("{rate:.0} kbit/s"));
        }
        let audio = self.count_of(TrackKind::Audio);
        let subtitle = self.count_of(TrackKind::Subtitle);
        if audio > 0 || subtitle > 0 {
            parts.push(format!("{audio} audio / {subtitle} subtitle track(s)"));
        }
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_are_filtered_by_kind() {
        let info = MediaInfo {
            tracks: vec![
                TrackInfo::new(0, TrackKind::Audio),
                TrackInfo::new(1, TrackKind::Subtitle),
                TrackInfo::new(2, TrackKind::Audio),
            ],
            ..MediaInfo::default()
        };
        assert_eq!(info.count_of(TrackKind::Audio), 2);
        assert_eq!(info.count_of(TrackKind::Subtitle), 1);
        assert_eq!(info.tracks_of(TrackKind::Subtitle).count(), 1);
    }

    #[test]
    fn the_summary_names_what_is_known() {
        let info = MediaInfo {
            container: Container::Mp4,
            video_codec: Some("h264".into()),
            bitrate: Some(2_500_000),
            tracks: vec![TrackInfo::new(0, TrackKind::Audio)],
            ..MediaInfo::default()
        };
        let summary = info.summary();
        assert!(summary.contains("MP4"));
        assert!(summary.contains("video h264"));
        assert!(summary.contains("2500 kbit/s"));
        assert!(summary.contains("1 audio"));
    }

    #[test]
    fn an_empty_info_is_valid() {
        let info = MediaInfo::default();
        assert_eq!(info.container, Container::Unknown);
        assert_eq!(info.kilobit_rate(), None);
        assert_eq!(info.summary(), "unknown container");
    }
}
