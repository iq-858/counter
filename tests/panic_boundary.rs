//! A backend panic must not silently kill the decode thread.
//!
//! Before the worker had a `catch_unwind` boundary, a panic inside a decoder
//! call unwound out of the thread: the last picture stayed on screen, no failure
//! reached the state, and the player claimed to be healthy while the feed was
//! frozen. These tests use an application-supplied decoder so they run on every
//! target, including those without a platform decoder. They check that a panic
//! from `read`, `seek`, and `close` is contained and reported through the normal
//! failure path.

use std::time::{Duration, Instant};

use cranpose_video::{
    DecodedFrame, PlaybackEvent, VideoDecoder, VideoError, VideoInfo, VideoPlayer,
};

/// How long a test waits for the decode thread to settle.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Which decoder call panics.
#[derive(Clone, Copy)]
enum PanicIn {
    Read,
    Seek,
    Close,
}

struct PanickingDecoder {
    where_: PanicIn,
    /// Avoid re-panicking on `close` after a `read` panic; the boundary closes
    /// the decoder on the way out and a second panic must also be contained.
    close_panics: bool,
}

impl PanickingDecoder {
    fn new(where_: PanicIn) -> Self {
        Self {
            where_,
            close_panics: matches!(where_, PanicIn::Close),
        }
    }
}

impl VideoDecoder for PanickingDecoder {
    fn info(&self) -> VideoInfo {
        VideoInfo {
            width: 16,
            height: 16,
            ..VideoInfo::default()
        }
    }

    fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
        if matches!(self.where_, PanicIn::Seek) {
            panic!("seek exploded");
        }
        Ok(())
    }

    fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        if matches!(self.where_, PanicIn::Read) {
            panic!("read exploded");
        }
        // Give the worker something to do so it keeps calling `read` until the
        // panic fires, but stay silent when the panic is elsewhere.
        Ok(None)
    }

    fn close(&mut self) {
        if self.close_panics {
            panic!("close exploded");
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
    false
}

fn panic_failure(player: &VideoPlayer) -> Option<String> {
    player.state().failure().and_then(|error| match error {
        VideoError::Decode(message) if message.contains("panicked") => Some(message.clone()),
        _ => None,
    })
}

#[test]
fn a_panic_in_read_is_reported_as_a_decode_failure() {
    let player =
        VideoPlayer::with_decoder(PanickingDecoder::new(PanicIn::Read)).expect("the player opens");
    assert!(
        wait_until(|| panic_failure(&player).is_some()),
        "the read panic never reached the failure state"
    );

    let message = panic_failure(&player).expect("a panic failure");
    assert!(
        message.contains("read exploded"),
        "unexpected message: {message}"
    );
    assert!(
        player
            .events()
            .iter()
            .any(|record| matches!(record.event(), PlaybackEvent::Failed { .. })),
        "the panic is in the event history"
    );
    assert_eq!(player.diagnostics().errors(), 1);
}

#[test]
fn a_panic_in_seek_is_reported_as_a_decode_failure() {
    let player =
        VideoPlayer::with_decoder(PanickingDecoder::new(PanicIn::Seek)).expect("the player opens");
    player
        .seek(Duration::from_millis(100))
        .expect("seek dispatches");
    assert!(
        wait_until(|| panic_failure(&player).is_some()),
        "the seek panic never reached the failure state"
    );
    let message = panic_failure(&player).expect("a panic failure");
    assert!(
        message.contains("seek exploded"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_panic_in_close_does_not_escape_the_worker() {
    // The worker closes the decoder on its way out. A panic there must be
    // contained: the player can still close without hanging or unwinding the
    // caller, and no success claim is made about a decoder that misbehaved.
    let mut player =
        VideoPlayer::with_decoder(PanickingDecoder::new(PanicIn::Close)).expect("the player opens");
    // Let the worker start and reach its first read, then close. The close-panic
    // fires as the worker drops its decoder after shutdown; it must not escape.
    std::thread::sleep(Duration::from_millis(50));
    player.close();
}
