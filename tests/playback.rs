//! Playback through the public player, on the clip the tests ship.

use std::time::{Duration, Instant};

use cranpose_video::{VideoError, VideoFrame, VideoPlayer, VideoState};

const CLIP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/clip.mp4");

/// The width and height the fixture records.
const SIZE: (u32, u32) = (64, 48);

/// How long a test waits for a decoder before it fails.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Whether this build carries a decoder, which a target without a media
/// framework does not.
#[expect(
    clippy::print_stderr,
    reason = "a test that skips says why in its own output"
)]
fn decodes() -> bool {
    let supported = cranpose_video::is_supported();
    if !supported {
        eprintln!(
            "skipped: this build carries no decoder, so {} decodes nothing",
            cranpose_video::platform().name()
        );
    }
    supported
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

/// The next picture the player publishes.
fn take_frame(player: &VideoPlayer) -> Option<VideoFrame> {
    let mut taken = None;
    wait_until(|| {
        taken = player.take_frame();
        taken.is_some()
    });
    taken
}

#[test]
fn opening_states_the_track() {
    if !decodes() {
        return;
    }
    let player = VideoPlayer::open(CLIP).expect("the clip opens");
    let info = player.info();
    assert_eq!((info.width, info.height), SIZE);
    let duration = info.duration.expect("the container states a duration");
    assert!(
        (duration.as_secs_f64() - 1.0).abs() < 0.2,
        "the clip runs a second, and the player states {duration:?}"
    );
    let rate = info
        .frame_rate
        .expect("the container states a frame rate")
        .frames_per_second();
    assert!(
        (rate - 15.0).abs() < 1.0,
        "the clip runs at 15 fps, stated as {rate}"
    );
    assert_eq!(player.state(), VideoState::Paused);
    assert_eq!(player.position(), Duration::ZERO);
}

#[test]
fn a_paused_player_shows_the_first_picture() {
    if !decodes() {
        return;
    }
    let player = VideoPlayer::open(CLIP).expect("the clip opens");
    let frame = take_frame(&player).expect("a paused player shows its first picture");
    assert_eq!((frame.width(), frame.height()), SIZE);
    assert_eq!(frame.byte_len(), SIZE.0 as usize * SIZE.1 as usize * 4);
    assert_eq!(frame.rgba().len(), frame.byte_len());
    assert!(
        frame
            .rgba()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] != 0),
        "the decoded picture holds no colour"
    );
    assert_eq!(player.state(), VideoState::Paused);
}

#[test]
fn play_runs_to_the_end() {
    if !decodes() {
        return;
    }
    let player = VideoPlayer::open(CLIP).expect("the clip opens");
    player.play().expect("playback starts");
    let mut pictures = 0;
    let ended = wait_until(|| {
        if player.take_frame().is_some() {
            pictures += 1;
        }
        player.state().is_ended()
    });
    assert!(ended, "playback stopped at {:?}", player.state());
    assert!(
        pictures > 5,
        "the second of video produced {pictures} pictures"
    );
    let duration = player.duration().expect("the clip states its length");
    assert!(
        player.position() + Duration::from_millis(50) >= duration,
        "playback ended at {:?} of {duration:?}",
        player.position()
    );
}

#[test]
fn playing_again_after_the_end_starts_over() {
    if !decodes() {
        return;
    }
    let player = VideoPlayer::open(CLIP).expect("the clip opens");
    player.play().expect("playback starts");
    assert!(
        wait_until(|| player.state().is_ended()),
        "playback never ended"
    );
    player.play().expect("playback starts again");
    assert!(wait_until(|| player.state().is_playing()));
    let frame = take_frame(&player).expect("a picture arrives after the restart");
    assert!(
        frame.pts() < Duration::from_millis(500),
        "a restart shows the start of the clip, and the first picture is at {:?}",
        frame.pts()
    );
}

#[test]
fn seeking_a_paused_player_moves_the_picture() {
    if !decodes() {
        return;
    }
    let player = VideoPlayer::open(CLIP).expect("the clip opens");
    assert!(take_frame(&player).is_some(), "the first picture arrives");
    player
        .seek(Duration::from_millis(700))
        .expect("the player seeks");
    assert!(
        wait_until(|| player.position() >= Duration::from_millis(600)),
        "the clock stands at {:?} after a seek to 700ms",
        player.position()
    );
    let frame = take_frame(&player).expect("the seeked picture arrives");
    assert!(
        frame.pts() >= Duration::from_millis(500),
        "the picture after a seek to 700ms is at {:?}",
        frame.pts()
    );
    assert_eq!(player.state(), VideoState::Paused);
}

#[test]
fn a_seek_by_fraction_lands_in_the_middle() {
    if !decodes() {
        return;
    }
    let player = VideoPlayer::open(CLIP).expect("the clip opens");
    player.seek_fraction(0.5).expect("the player seeks");
    let duration = player.duration().expect("the clip states its length");
    let middle = duration / 2;
    assert!(
        wait_until(|| player.position() + Duration::from_millis(120) >= middle),
        "the clock stands at {:?} after a seek to the middle of {duration:?}",
        player.position()
    );
}

#[test]
fn the_clip_opens_from_its_bytes() {
    if !decodes() {
        return;
    }
    let bytes = std::fs::read(CLIP).expect("the fixture reads");
    let player = VideoPlayer::open(bytes).expect("the clip opens from its bytes");
    assert_eq!((player.info().width, player.info().height), SIZE);
    assert!(take_frame(&player).is_some(), "a picture arrives");
}

#[test]
fn a_missing_file_reports_its_source() {
    if !decodes() {
        return;
    }
    let missing = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/absent.mp4");
    // Backends name the failure differently: a file-backed demuxer reports
    // `Source`, while AVFoundation may report its own decoder error. What
    // matters is that opening an absent file fails and does not open a player.
    match VideoPlayer::open(missing) {
        Err(_) => {}
        other => panic!("a missing file must fail to open, and the player returned {other:?}"),
    }
}

#[test]
fn two_players_show_their_pictures_at_once() {
    if !decodes() {
        return;
    }
    let first = VideoPlayer::open(CLIP).expect("the first clip opens");
    let second = VideoPlayer::open(CLIP).expect("the second clip opens");
    first.play().expect("the first player plays");
    second.play().expect("the second player plays");
    let mut first_pictures = 0_u32;
    let mut second_pictures = 0_u32;
    let both_moved = wait_until(|| {
        first_pictures += u32::from(first.take_frame().is_some());
        second_pictures += u32::from(second.take_frame().is_some());
        first_pictures > 0
            && second_pictures > 0
            && first.position() > Duration::ZERO
            && second.position() > Duration::ZERO
    });
    assert!(
        both_moved,
        "the two players produced {first_pictures} and {second_pictures} pictures"
    );
}

#[test]
fn closing_stops_the_player() {
    if !decodes() {
        return;
    }
    let mut player = VideoPlayer::open(CLIP).expect("the clip opens");
    player.close();
    assert!(player.is_closed());
    assert!(player.take_frame().is_none());
    assert_eq!(player.play(), Err(VideoError::Closed));
    assert_eq!(player.pause(), Err(VideoError::Closed));
    assert_eq!(player.seek(Duration::from_secs(1)), Err(VideoError::Closed));
}
