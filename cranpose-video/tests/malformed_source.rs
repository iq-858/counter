//! Arbitrary received videos must fail closed, not panic, hang, or claim health.
//!
//! A caller can hand the library any bytes — a truncated download, an error
//! page saved with a video extension, a half-written cache entry. The platform
//! decoder is the last line of defence: it has to reject those without an
//! unexpected crash, and the player has to settle into a truthful state so a
//! screen never reports a healthy feed that is frozen. These tests run against
//! the real decoder where one exists; builds without a decoder skip by design.

use std::time::{Duration, Instant};

use cranpose_video::{VideoError, VideoPlayer, VideoSource, VideoState};

/// A plausible MP4 file header followed by noise: enough to get past a shallow
/// `starts with ftyp` sniff, with no valid tracks or boxes behind it.
fn plausible_garbage() -> Vec<u8> {
    let mut bytes = b"\x00\x00\x00\x18ftypisom\x00\x00\x02\x00isomiso2mp41".to_vec();
    bytes.extend(std::iter::repeat(0xA5).take(4096));
    bytes
}

/// Pure noise with no container header at all.
fn noise() -> Vec<u8> {
    (0u32..2048)
        .map(|i| (i.wrapping_mul(31).wrapping_add(7)) as u8)
        .collect()
}

/// How long a malformed source gets to settle before the test calls it stuck.
const SETTLE: Duration = Duration::from_secs(6);
/// How long `close()` may take before a decode thread counts as wedged.
const CLOSE_BUDGET: Duration = Duration::from_secs(3);

#[expect(
    clippy::print_stderr,
    reason = "a test that skips says why in its own output"
)]
fn supported() -> bool {
    let ok = cranpose_video::is_supported();
    if !ok {
        eprintln!(
            "skipped: no decoder on {}, so there is no backend to reject bytes",
            cranpose_video::platform().name()
        );
    }
    ok
}

fn settling_state(state: &VideoState) -> bool {
    matches!(
        state,
        VideoState::Failed(_) | VideoState::Ended | VideoState::Paused
    )
}

fn one_source(bytes: Vec<u8>, label: &str) {
    if !supported() {
        return;
    }

    // Opening may reject the bytes outright. That is a valid, healthy outcome.
    let mut player = match VideoPlayer::open(VideoSource::bytes(bytes)) {
        Ok(player) => player,
        Err(
            VideoError::Source(_)
            | VideoError::Decode(_)
            | VideoError::NoVideoTrack
            | VideoError::Unsupported { .. },
        ) => return,
        Err(other) => panic!("{label}: unexpected open error {other:?}"),
    };

    // If it opened, pump reads for a bounded time. A malformed source may fail
    // or run to an abrupt end, but it must never report an endless healthy feed.
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline && !settling_state(&player.state()) {
        let _frame = player.take_frame();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        settling_state(&player.state()),
        "{label}: a malformed source never settled, state is {:?}",
        player.state()
    );

    // The decode thread must still take the shutdown signal; a corrupt sample
    // must not wedge it so `close()` has to be abandoned.
    let began = Instant::now();
    player.close();
    assert!(
        began.elapsed() < CLOSE_BUDGET,
        "{label}: close() wedged for {:?} on a malformed source",
        began.elapsed()
    );
    assert!(player.is_closed(), "{label}: the player did not close");
}

#[test]
fn a_plausible_but_empty_mp4_fails_closed() {
    one_source(plausible_garbage(), "empty mp4");
}

#[test]
fn pure_noise_fails_closed() {
    one_source(noise(), "noise");
}
