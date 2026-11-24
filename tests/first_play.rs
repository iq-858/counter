//! The `Playing` event belongs to a picture the screen can show.
//!
//! `play()` starts the clock at once so the decode thread can produce pictures,
//! but the event that announces playback is held until the first picture is
//! actually published. A screen that waits for `Playing` before showing video
//! therefore never announces a blank frame.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use cranpose_video::{
    DecodedFrame, PlaybackEvent, VideoDecoder, VideoError, VideoInfo, VideoPlayer,
};

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

fn has_event(player: &VideoPlayer, want: PlaybackEvent) -> bool {
    player.events().iter().any(|record| record.event() == &want)
}

/// A decoder that blocks its first read until the test releases it. That gives
/// the test a window in which playback has been requested but no picture exists.
struct GatedFrames {
    release: Arc<AtomicBool>,
}

impl VideoDecoder for GatedFrames {
    fn info(&self) -> VideoInfo {
        VideoInfo {
            width: 16,
            height: 16,
            ..VideoInfo::default()
        }
    }

    fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        while !self.release.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(1));
        }
        rgba.resize(16 * 16 * 4, 0);
        Ok(Some(DecodedFrame {
            pts: Duration::ZERO,
            width: 16,
            height: 16,
        }))
    }
}

#[test]
fn playing_is_announced_with_the_first_picture_not_before_it() {
    let release = Arc::new(AtomicBool::new(false));
    let player = VideoPlayer::with_decoder(GatedFrames {
        release: Arc::clone(&release),
    })
    .expect("the player opens");

    // Give the decode thread time to enter the blocked read.
    std::thread::sleep(Duration::from_millis(30));
    player.play().expect("playback starts");

    assert!(
        player.state().is_playing(),
        "the clock starts as soon as play is called"
    );
    assert!(
        !has_event(&player, PlaybackEvent::Playing),
        "Playing must not be announced before a picture exists"
    );

    release.store(true, Ordering::Relaxed);
    assert!(
        wait_until(|| player.diagnostics().frames_published() > 0),
        "the first picture was never published"
    );
    assert!(
        has_event(&player, PlaybackEvent::Playing),
        "Playing is announced once the first picture arrives"
    );
}
