//! Observability through the public player API.
//!
//! These tests use an application-supplied decoder so they run on every target
//! the crate builds for, including those without a platform decoder. They check
//! the contract a screen or a diagnostic build relies on: a stable session id,
//! an ordered and bounded event history, a snapshot that folds state and
//! counters together, and an event for each transition that matters.

use std::time::{Duration, Instant};

use cranpose_video::{
    DecodedFrame, PlaybackEvent, VideoDecoder, VideoError, VideoInfo, VideoPlayer, VideoState,
};

/// How long a test waits for the decode thread to settle.
const TIMEOUT: Duration = Duration::from_secs(10);

/// What the mock decoder does when asked for a picture.
#[derive(Clone, Copy)]
enum Mode {
    /// No media: the first read ends playback.
    Silent,
    /// Pictures for as long as the player wants them.
    Frames,
    /// Reading fails at once.
    Fail,
}

struct TestDecoder {
    mode: Mode,
    pts: Duration,
}

impl TestDecoder {
    fn silent() -> Self {
        Self {
            mode: Mode::Silent,
            pts: Duration::ZERO,
        }
    }

    fn frames() -> Self {
        Self {
            mode: Mode::Frames,
            pts: Duration::ZERO,
        }
    }

    fn fail() -> Self {
        Self {
            mode: Mode::Fail,
            pts: Duration::ZERO,
        }
    }
}

impl VideoDecoder for TestDecoder {
    fn info(&self) -> VideoInfo {
        VideoInfo {
            width: 16,
            height: 16,
            ..VideoInfo::default()
        }
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        self.pts = position;
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        match self.mode {
            Mode::Silent => Ok(None),
            Mode::Fail => Err(VideoError::Decode("test failure".to_string())),
            Mode::Frames => {
                // A short pause keeps the decoder from spinning; the player
                // paces published pictures against its own clock.
                std::thread::sleep(Duration::from_millis(2));
                rgba.resize(16 * 16 * 4, 0);
                let pts = self.pts;
                self.pts += Duration::from_millis(33);
                Ok(Some(DecodedFrame {
                    pts,
                    width: 16,
                    height: 16,
                }))
            }
        }
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    ready()
}

/// A lightweight name for an event, used to assert their order without
/// comparing payloads.
fn kind(event: &PlaybackEvent) -> &'static str {
    match event {
        PlaybackEvent::Opened => "opened",
        PlaybackEvent::Playing => "playing",
        PlaybackEvent::Paused => "paused",
        PlaybackEvent::Seeking { .. } => "seeking",
        PlaybackEvent::Reanchored { .. } => "reanchored",
        PlaybackEvent::Ended => "ended",
        PlaybackEvent::Failed { .. } => "failed",
        PlaybackEvent::InfoReady { .. } => "info",
        PlaybackEvent::SpeedChanged { .. } => "speed",
        PlaybackEvent::Muted { .. } => "muted",
        PlaybackEvent::VolumeChanged { .. } => "volume",
        PlaybackEvent::Closed => "closed",
    }
}

#[test]
fn a_session_has_a_stable_unique_id() {
    let first = VideoPlayer::with_decoder(TestDecoder::silent()).expect("the player opens");
    let second = VideoPlayer::with_decoder(TestDecoder::silent()).expect("the player opens");

    assert_ne!(first.session(), second.session(), "sessions are unique");
    assert!(first.session().get() > 0);
    let snapshot = first.diagnostics();
    assert_eq!(snapshot.session(), first.session());
    // Reading does not change the identity.
    assert_eq!(first.session(), first.diagnostics().session());
}

#[test]
fn controls_emit_events_in_order() {
    let mut player = VideoPlayer::with_decoder(TestDecoder::frames()).expect("the player opens");

    // Let the decoder publish its first picture so no start-up event can land
    // between the control calls below.
    assert!(
        wait_until(|| player.diagnostics().frames_published() > 0),
        "the first picture was never published"
    );

    player.play().expect("play works");
    player.pause().expect("pause works");
    player.seek(Duration::from_millis(250)).expect("seek works");
    player.set_speed(2.0).expect("speed works");
    player.set_muted(true).expect("mute works");
    player.set_volume(0.5).expect("volume works");
    player.close();

    let records = player.events();
    let order: Vec<&str> = records.iter().map(|record| kind(record.event())).collect();
    // The events issued by the screen are synchronous and keep their order; a
    // decoder may interleave a re-anchor between them, which is why the check
    // looks for a subsequence rather than adjacent records.
    let wanted = [
        "opened", "playing", "paused", "seeking", "speed", "muted", "volume", "closed",
    ];
    let mut at = 0;
    for expected in wanted {
        let found = order[at..]
            .iter()
            .position(|name| *name == expected)
            .unwrap_or_else(|| panic!("event {expected} missing after {at} in {order:?}"));
        at += found + 1;
    }
}

#[test]
fn the_event_history_is_ordered_and_bounded() {
    let player = VideoPlayer::with_decoder(TestDecoder::silent()).expect("the player opens");
    // One control event per call, well past the bounded history's capacity.
    for level in 0..300 {
        player
            .set_volume(level as f32 / 300.0)
            .expect("volume works");
    }

    let records = player.events();
    assert!(
        records.len() <= 256,
        "the history is bounded, but kept {}",
        records.len()
    );
    assert_eq!(records.len(), 256, "the ring filled up");
    assert!(
        records[0].sequence() > 1,
        "older records left the ring but kept counting"
    );
    for pair in records.windows(2) {
        assert!(
            pair[0].sequence() < pair[1].sequence(),
            "sequences increase in emission order"
        );
        assert!(
            pair[0].at() <= pair[1].at(),
            "timestamps do not run backwards"
        );
    }
    assert_eq!(
        player.diagnostics().event_count(),
        records.len(),
        "the snapshot counts what the history holds"
    );
}

#[test]
fn a_snapshot_folds_state_and_counters_together() {
    let player = VideoPlayer::with_decoder(TestDecoder::frames()).expect("the player opens");
    assert!(
        wait_until(|| player.diagnostics().frames_published() > 0),
        "the first picture was never published"
    );

    player.play().expect("play works");
    assert!(wait_until(|| player.state().is_playing()));
    player.pause().expect("pause works");
    player.seek(Duration::from_millis(500)).expect("seek works");
    player.set_speed(1.5).expect("speed works");

    let snapshot = player.diagnostics();
    assert_eq!(snapshot.state(), &VideoState::Paused);
    assert_eq!(snapshot.speed(), 1.5);
    assert_eq!(snapshot.plays(), 1);
    assert_eq!(snapshot.seeks(), 1);
    assert_eq!(snapshot.errors(), 0);
    assert!(
        snapshot.frames_published() > 0,
        "published pictures are counted without filling the event log"
    );
    assert!(
        snapshot.uptime() > Duration::ZERO,
        "the session knows how long it has been alive"
    );
    assert_eq!(snapshot.info().width, 16);
    assert_eq!(snapshot.info().height, 16);
}

#[test]
fn a_failing_decode_records_the_failure() {
    let player = VideoPlayer::with_decoder(TestDecoder::fail()).expect("the player opens");
    assert!(
        wait_until(|| player.state().failure().is_some()),
        "the decoder failure never reached the state"
    );

    let error = VideoError::Decode("test failure".to_string());
    assert_eq!(player.state().failure(), Some(&error));
    assert!(
        player.events().iter().any(|record| record.event()
            == &PlaybackEvent::Failed {
                error: error.clone()
            }),
        "the failure is in the event history"
    );
    assert_eq!(player.diagnostics().errors(), 1);
}

#[test]
fn end_of_media_records_ended() {
    let player = VideoPlayer::with_decoder(TestDecoder::silent()).expect("the player opens");
    assert!(
        wait_until(|| player.state().is_ended()),
        "the silent decoder never ended"
    );
    assert!(
        player
            .events()
            .iter()
            .any(|record| record.event() == &PlaybackEvent::Ended),
        "the end is in the event history"
    );
}

#[test]
fn closing_records_closed_as_the_last_event() {
    let mut player = VideoPlayer::with_decoder(TestDecoder::frames()).expect("the player opens");
    player.close();
    assert!(player.is_closed());
    let records = player.events();
    assert_eq!(
        records.last().expect("close emits an event").event(),
        &PlaybackEvent::Closed
    );
}
