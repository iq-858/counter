//! `close()` must never wait on a decoder stuck inside `read()`.
//!
//! The decode thread is detached on shutdown rather than joined: the worker
//! owns the decoder and closes it when a blocking read finally returns, but the
//! thread that draws (or closes) must not be held by a backend that ignores its
//! shutdown signal. These tests drive that boundary with a decoder that parks
//! inside `read()`: one honours the shutdown flag, one ignores it entirely. In
//! both cases `close()` returns promptly and `Closed` stays the last event.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    time::{Duration, Instant},
};

use cranpose_video::{
    DecodedFrame, PlaybackEvent, VideoDecoder, VideoError, VideoInfo, VideoPlayer,
};

/// A read that parks until released, either by the shutdown flag or by nothing
/// at all. It tells the test when it has entered the blocking section.
struct BlockingDecoder {
    entered: Sender<()>,
    shutdown: Option<Arc<AtomicBool>>,
    /// When false the decoder ignores its shutdown signal, simulating a backend
    /// whose read cannot be interrupted.
    honour_shutdown: bool,
    /// Kept alive by the test; releasing it lets the parked read finish so the
    /// detached worker can actually exit and drop the decoder.
    release: Arc<AtomicBool>,
}

impl VideoDecoder for BlockingDecoder {
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
        let _ = self.entered.send(());
        loop {
            let signalled = self
                .shutdown
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed));
            if (self.honour_shutdown && signalled) || self.release.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        rgba.resize(16 * 16 * 4, 0);
        Ok(Some(DecodedFrame {
            pts: Duration::ZERO,
            width: 16,
            height: 16,
        }))
    }

    fn observe_shutdown(&mut self, flag: Arc<AtomicBool>) {
        self.shutdown = Some(flag);
    }
}

/// Opens a player whose decoder is parked in `read()`, returns the pieces the
/// test needs, and waits until the worker has actually entered the read.
fn parked_player(honour_shutdown: bool) -> (VideoPlayer, Arc<AtomicBool>) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let release = Arc::new(AtomicBool::new(false));
    let decoder = BlockingDecoder {
        entered: entered_tx,
        shutdown: None,
        honour_shutdown,
        release: Arc::clone(&release),
    };
    let player = VideoPlayer::with_decoder(decoder).expect("the player opens");
    player.play().expect("playback starts");
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the decoder entered read()");
    (player, release)
}

/// `close()` must come back well under the ten-second test budget even though
/// the decoder is stuck. A join here would hang until the test timed out.
const CLOSE_BUDGET: Duration = Duration::from_secs(2);

#[test]
fn close_returns_while_a_decoder_honours_shutdown() {
    let (mut player, _release) = parked_player(true);
    let began = Instant::now();
    player.close();
    assert!(
        began.elapsed() < CLOSE_BUDGET,
        "close blocked for {:?}",
        began.elapsed()
    );
    assert!(player.is_closed());
    assert_eq!(
        player.events().last().map(|record| record.event()),
        Some(&PlaybackEvent::Closed)
    );
}

#[test]
fn close_returns_even_if_a_decoder_ignores_shutdown() {
    // The decoder never observes its shutdown flag and stays in `read()`.
    // `close()` still returns: the worker is detached, not joined. Release the
    // read afterwards so the detached worker can finish and drop the decoder.
    let (mut player, release) = parked_player(false);
    let began = Instant::now();
    player.close();
    assert!(
        began.elapsed() < CLOSE_BUDGET,
        "close blocked for {:?}",
        began.elapsed()
    );
    assert!(player.is_closed());
    release.store(true, Ordering::Relaxed);
    // A short grace period for the detached worker to observe the release and
    // exit on its own; the test does not join it, matching `Engine::shutdown`.
    std::thread::sleep(Duration::from_millis(50));
}
