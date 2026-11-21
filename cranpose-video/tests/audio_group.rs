//! Tests for the audio-group API through the public player.
//!
//! A player assigns its group from the drawing thread and the decode thread
//! delivers it to the decoder's `set_audio_group` hook. These tests use an
//! application-supplied decoder so they run without a platform decoder or a
//! real audio device: the decoder records the group it receives behind
//! atomics, which the test polls with the same `wait_until` helper the other
//! integration tests use.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use cranpose_video::{DecodedFrame, VideoDecoder, VideoError, VideoInfo, VideoPlayer};

/// How long a test waits for the decode thread to deliver a group.
const TIMEOUT: Duration = Duration::from_secs(10);

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

/// A decoder that publishes one picture, then ends, and records every group
/// the player sends it from the decode thread.
struct GroupDecoder {
    pts: Duration,
    emitted: bool,
    last_group: Arc<AtomicU64>,
    calls: Arc<AtomicUsize>,
}

impl GroupDecoder {
    fn new() -> (Self, Arc<AtomicU64>, Arc<AtomicUsize>) {
        // 0 is never a group a test assigns, so it doubles as "not yet called".
        let last_group = Arc::new(AtomicU64::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                pts: Duration::ZERO,
                emitted: false,
                last_group: Arc::clone(&last_group),
                calls: Arc::clone(&calls),
            },
            last_group,
            calls,
        )
    }
}

impl VideoDecoder for GroupDecoder {
    fn info(&self) -> VideoInfo {
        VideoInfo {
            width: 16,
            height: 16,
            ..VideoInfo::default()
        }
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        self.pts = position;
        self.emitted = false;
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        if self.emitted {
            return Ok(None);
        }
        self.emitted = true;
        rgba.resize(16 * 16 * 4, 0);
        let pts = self.pts;
        self.pts += Duration::from_millis(33);
        Ok(Some(DecodedFrame {
            pts,
            width: 16,
            height: 16,
        }))
    }

    fn set_audio_group(&mut self, group: u64) {
        self.last_group.store(group, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn set_audio_group_reaches_the_decoder_after_play() {
    let (decoder, last_group, calls) = GroupDecoder::new();
    let mut player = VideoPlayer::with_decoder(decoder).expect("the player opens");

    player.play().expect("play works");
    assert!(
        wait_until(|| player.diagnostics().frames_published() > 0),
        "the decoder never published a picture"
    );

    player.set_audio_group(42).expect("assigning a group works");
    assert!(
        wait_until(|| last_group.load(Ordering::SeqCst) == 42),
        "the decoder never received the assigned group"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "one assignment reaches the hook once"
    );
    player.close();
}

#[test]
fn two_players_can_share_a_group_and_reassigning_is_idempotent() {
    let (decoder_a, last_a, calls_a) = GroupDecoder::new();
    let (decoder_b, last_b, calls_b) = GroupDecoder::new();
    let mut a = VideoPlayer::with_decoder(decoder_a).expect("the first player opens");
    let mut b = VideoPlayer::with_decoder(decoder_b).expect("the second player opens");
    a.play().expect("the first player plays");
    b.play().expect("the second player plays");

    a.set_audio_group(7).expect("the first player accepts a group");
    b.set_audio_group(7).expect("the second player accepts the same group");
    assert!(
        wait_until(|| last_a.load(Ordering::SeqCst) == 7
            && last_b.load(Ordering::SeqCst) == 7),
        "both decoders must receive the shared group"
    );
    assert_eq!(calls_a.load(Ordering::SeqCst), 1);
    assert_eq!(calls_b.load(Ordering::SeqCst), 1);

    // Assigning the same group again is accepted and simply reaches the hook
    // again with the same value: no error, no change to the assignment.
    a.set_audio_group(7).expect("reassigning a group works");
    assert!(
        wait_until(|| calls_a.load(Ordering::SeqCst) >= 2),
        "the repeated assignment must reach the decoder"
    );
    assert_eq!(last_a.load(Ordering::SeqCst), 7);
    assert_eq!(calls_a.load(Ordering::SeqCst), 2);

    a.close();
    b.close();
}

#[test]
fn different_groups_are_accepted() {
    let (decoder_a, last_a, _) = GroupDecoder::new();
    let (decoder_b, last_b, _) = GroupDecoder::new();
    let mut a = VideoPlayer::with_decoder(decoder_a).expect("the first player opens");
    let mut b = VideoPlayer::with_decoder(decoder_b).expect("the second player opens");
    a.play().expect("the first player plays");
    b.play().expect("the second player plays");

    a.set_audio_group(11).expect("the first group works");
    b.set_audio_group(22).expect("a different group works");
    assert!(
        wait_until(|| last_a.load(Ordering::SeqCst) == 11
            && last_b.load(Ordering::SeqCst) == 22),
        "each decoder must receive its own group"
    );
    assert_ne!(
        last_a.load(Ordering::SeqCst),
        last_b.load(Ordering::SeqCst),
        "distinct groups stay distinct"
    );

    a.close();
    b.close();
}

#[test]
fn set_audio_group_after_close_returns_closed() {
    let (decoder, _, _) = GroupDecoder::new();
    let mut player = VideoPlayer::with_decoder(decoder).expect("the player opens");
    player.close();
    assert!(player.is_closed());

    assert_eq!(player.set_audio_group(9), Err(VideoError::Closed));
    assert_eq!(
        player.set_audio_group(10),
        Err(VideoError::Closed),
        "every call after close reports the closed player"
    );
}
