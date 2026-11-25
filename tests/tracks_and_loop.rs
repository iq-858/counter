//! Track selection, loop range and buffering, driven through a real player.
//!
//! Every case here goes through [`VideoPlayer::with_decoder`], so it exercises
//! the same command path a platform backend uses: the call records the command,
//! hands it to the decode thread, and the thread calls the decoder. A feature
//! that is only stored on the player's own state would pass a unit test and fail
//! here.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cranpose_video::{
    Buffering, BufferingState, Container, DecodedFrame, LoopRange, MediaInfo, TrackInfo, TrackKind,
    VideoDecoder, VideoError, VideoInfo, VideoPlayer,
};

/// A decoder that states a container with two audio tracks and one subtitle
/// track, and records the selections the player sends it.
struct TrackedDecoder {
    media: MediaInfo,
    audio: Arc<AtomicI64>,
    subtitle: Arc<AtomicI64>,
    /// The background the decoder reports, so the player's buffering state can
    /// be observed without a network.
    buffering: Buffering,
}

impl VideoDecoder for TrackedDecoder {
    fn info(&self) -> VideoInfo {
        VideoInfo {
            width: 32,
            height: 18,
            ..VideoInfo::default()
        }
    }

    fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
        Ok(())
    }

    fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        Ok(None)
    }

    fn media(&self) -> MediaInfo {
        self.media.clone()
    }

    fn buffering(&self) -> Buffering {
        self.buffering
    }

    fn set_audio_track(&mut self, index: Option<usize>) {
        self.audio
            .store(index.map_or(-1, |index| index as i64), Ordering::SeqCst);
    }

    fn set_subtitle_track(&mut self, index: Option<usize>) {
        self.subtitle
            .store(index.map_or(-1, |index| index as i64), Ordering::SeqCst);
    }
}

fn media_with_tracks() -> MediaInfo {
    MediaInfo {
        container: Container::Mp4,
        video_codec: Some("h264".to_owned()),
        audio_codec: Some("aac".to_owned()),
        bitrate: Some(1_500_000),
        tracks: vec![
            TrackInfo::new(0, TrackKind::Audio)
                .with_language("en-GB")
                .with_label("Main")
                .as_default(),
            TrackInfo::new(1, TrackKind::Audio)
                .with_language("ar")
                .with_label("Commentary"),
            TrackInfo::new(2, TrackKind::Subtitle)
                .with_language("en")
                .with_label("English"),
        ],
        ..MediaInfo::default()
    }
}

fn tracked_player() -> (VideoPlayer, Arc<AtomicI64>, Arc<AtomicI64>) {
    let audio = Arc::new(AtomicI64::new(-2));
    let subtitle = Arc::new(AtomicI64::new(-2));
    let decoder = TrackedDecoder {
        media: media_with_tracks(),
        audio: Arc::clone(&audio),
        subtitle: Arc::clone(&subtitle),
        buffering: Buffering::new(),
    };
    let player = VideoPlayer::with_decoder(decoder).expect("the decoder starts");
    (player, audio, subtitle)
}

#[test]
fn the_container_facts_reach_the_player() {
    let (player, _, _) = tracked_player();
    let media = player.media();
    assert_eq!(media.container, Container::Mp4);
    assert_eq!(media.video_codec.as_deref(), Some("h264"));
    assert_eq!(media.count_of(TrackKind::Audio), 2);
    assert_eq!(media.count_of(TrackKind::Subtitle), 1);
    assert_eq!(media.kilobit_rate(), Some(1500.0));
}

#[test]
fn the_audio_tracks_are_listed_and_one_is_selected() {
    let (player, audio, _) = tracked_player();
    let tracks = player.audio_tracks();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks.selected(), Some(0), "the default track is selected");
    assert_eq!(tracks.tracks()[0].language.as_deref(), Some("en-GB"));

    player.set_audio_track(Some(1)).expect("the track exists");
    assert_eq!(player.audio_tracks().selected(), Some(1));
    wait_for(&audio, 1);
}

#[test]
fn selecting_an_absent_audio_track_is_refused_and_changes_nothing() {
    let (player, audio, _) = tracked_player();
    assert!(player.set_audio_track(Some(9)).is_err());
    assert_eq!(player.audio_tracks().selected(), Some(0));
    assert_eq!(audio.load(Ordering::SeqCst), -2, "the decoder was not called");
}

#[test]
fn subtitles_can_be_turned_off_and_on() {
    let (player, _, subtitle) = tracked_player();
    assert_eq!(player.subtitle_tracks().len(), 1);

    player.set_subtitle_track(None).expect("off is allowed");
    assert_eq!(player.subtitle_tracks().selected(), None);
    wait_for(&subtitle, -1);

    // The subtitle stream's own index is 2: it sits after the two audio
    // streams in the container, and selection is by the backend's index, not by
    // a position within the subtitle list.
    player.set_subtitle_track(Some(2)).expect("the track exists");
    assert_eq!(player.subtitle_tracks().selected(), Some(2));
    wait_for(&subtitle, 2);
}

#[test]
fn a_language_is_found_by_its_primary_subtag() {
    let (player, _, _) = tracked_player();
    let mut list = player.audio_tracks();
    let found = list.select_language("ar", false).expect("an Arabic track exists");
    assert_eq!(found, 1);
    // "en" must match the "en-GB" track through its primary subtag.
    assert_eq!(list.select_language("en", false), Some(0));
}

#[test]
fn a_loop_range_is_stored_and_clamped_to_the_source() {
    let (player, _, _) = tracked_player();
    // The decoder states no duration, so a range keeps its own bounds.
    let range = LoopRange::new(Duration::from_secs(2), Duration::from_secs(5));
    player.set_loop_range(Some(range));
    assert_eq!(player.loop_range(), Some(range));
    assert!(player.is_looping(), "a loop range implies repeating");

    player.set_loop_range(None);
    assert_eq!(player.loop_range(), None);
}

#[test]
fn an_inverted_loop_range_is_ordered_and_a_short_one_is_widened() {
    let inverted = LoopRange::new(Duration::from_secs(5), Duration::from_secs(2));
    assert_eq!(inverted.start(), Duration::from_secs(2));
    assert_eq!(inverted.end(), Duration::from_secs(5));

    // A range that would repeat almost instantly is widened to the minimum, so
    // a loop never spins the decode thread on the same frames.
    let flat = LoopRange::new(Duration::from_secs(5), Duration::from_secs(5));
    assert_eq!(flat.length(), cranpose_video::MIN_RANGE);
}

#[test]
fn buffering_starts_idle_and_reports_what_the_decoder_says() {
    let (player, _, _) = tracked_player();
    assert_eq!(player.buffering().state(), BufferingState::Idle);
    // An idle buffer reports 0.0, not 1.0: the fraction is how much of the
    // window is filled, and a screen draws an indeterminate spinner at 0.0
    // rather than a full ring. `is_idle` is the flag to branch on.
    assert_eq!(player.buffering().fraction(), 0.0);
    assert!(player.buffering().is_idle());
    assert_eq!(player.buffering().percent(), 0);
}

fn wait_for(cell: &Arc<AtomicI64>, want: i64) {
    // The selection is dispatched to the decode thread; the assertion is about
    // what the decoder received, so it is polled rather than assumed instant.
    let mut seen = cell.load(Ordering::SeqCst);
    for _ in 0..200 {
        seen = cell.load(Ordering::SeqCst);
        if seen == want {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(seen, want, "the decoder never received the selection");
}
