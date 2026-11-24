//! Frame-construction hardening on the public player.
//!
//! A decoder may describe a picture that its pixel buffer cannot hold. The
//! player used to drop such a picture in silence: no frame reached the screen,
//! no failure was reported, and a control that waited for a picture waited for
//! ever. These tests use an application-supplied decoder so they run on every
//! target, including those without a platform decoder.

use std::time::{Duration, Instant};

use cranpose_video::{
    DecodedFrame, VideoDecoder, VideoError, VideoFrame, VideoInfo, VideoPlayer, VideoState,
};

/// How long a test waits for the decode thread to settle.
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

/// A decoder whose first picture has no dimensions. The empty byte buffer
/// satisfies the length check, so only frame construction can reject it.
struct ZeroSizeDecoder {
    reads: usize,
}

impl VideoDecoder for ZeroSizeDecoder {
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
        self.reads += 1;
        if self.reads == 1 {
            rgba.clear();
            Ok(Some(DecodedFrame {
                pts: Duration::ZERO,
                width: 0,
                height: 0,
            }))
        } else {
            Ok(None)
        }
    }
}

#[test]
fn a_zero_sized_rgba_picture_is_rejected_before_it_reaches_the_player() {
    // An empty buffer is the right length for 0x0, so the byte-count check on
    // its own cannot catch this picture. Frame construction must reject it;
    // the player then maps the `None` to a decode failure.
    assert!(VideoFrame::from_rgba8(0, 0, Duration::ZERO, Vec::new()).is_none());
    assert!(VideoFrame::from_rgba8(0, 4, Duration::ZERO, Vec::new()).is_none());
    assert!(VideoFrame::from_rgba8(4, 0, Duration::ZERO, Vec::new()).is_none());

    // A positive size with a matching buffer still produces a usable frame.
    let frame = VideoFrame::from_rgba8(2, 3, Duration::ZERO, vec![0; 2 * 3 * 4])
        .expect("a normal RGBA frame is accepted");
    assert_eq!(frame.width(), 2);
    assert_eq!(frame.height(), 3);
}

#[test]
fn an_invalid_decoded_picture_fails_the_player_instead_of_being_dropped() {
    let player = VideoPlayer::with_decoder(ZeroSizeDecoder { reads: 0 }).expect("the player opens");

    assert!(
        wait_until(|| player.state().failure().is_some()),
        "the invalid picture did not fail the player: {:?}",
        player.state()
    );
    assert!(
        matches!(player.state(), VideoState::Failed(VideoError::Decode(_))),
        "an invalid picture reports a decode failure: {:?}",
        player.state()
    );
    // The picture was never published, so the reported size is not a zero pair.
    assert_ne!(player.decoded_size(), Some((0, 0)));
}

/// A decoder that states no size until it reads a picture, as a browser does.
/// The player adopts the size from the first valid frame.
struct LateSizeDecoder;

impl VideoDecoder for LateSizeDecoder {
    fn info(&self) -> VideoInfo {
        VideoInfo::default()
    }

    fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        rgba.resize(8 * 6 * 4, 0);
        Ok(Some(DecodedFrame {
            pts: Duration::ZERO,
            width: 8,
            height: 6,
        }))
    }
}

#[test]
fn the_reported_decoded_size_always_has_non_zero_dimensions() {
    let player = VideoPlayer::with_decoder(LateSizeDecoder).expect("the player opens");
    assert!(
        wait_until(|| player.decoded_size().is_some()),
        "the first picture was never published"
    );
    let size = player.decoded_size().expect("a size is reported");
    assert_eq!(size, (8, 6));
    assert!(size.0 > 0 && size.1 > 0);
}
