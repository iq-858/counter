//! Intensive, multi-scenario checks for audio recovery and route changes.
//!
//! A device unplug and a Reels swipe hit the same machinery: a component
//! reports an audio failure, the worker retries on a cooldown, and the picture
//! must keep running throughout. These tests drive that machinery through the
//! public API with an application-supplied decoder, so they run on every target
//! and do not need a real WASAPI endpoint. The Windows-only device selection
//! (default handle plus concrete-endpoint fallback) is unit tested in
//! `mf_audio.rs` and exercised by `av_probe` on a real device.
//!
//! Each scenario asserts a different failure mode:
//!   1. recovery restores sound after several failed attempts;
//!   2. repeated failures never give up and never fail the picture;
//!   3. a route change arms exactly one verification attempt;
//!   4. video keeps publishing frames while audio is recovering;
//!   5. rapid clip switches do not inherit a stuck session.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use std::time::{Duration, Instant};

use cranpose_video::{
    DecodedFrame, PlaybackSession, SessionEvent, VideoDecoder, VideoError, VideoInfo, VideoPlayer,
    VideoState,
};

const TIMEOUT: Duration = Duration::from_secs(16);

/// A decoder that publishes frames and can script audio failures.
struct RouteDecoder {
    pts: Duration,
    session: Option<PlaybackSession>,
    /// Set by the test to make the component report one audio failure.
    fail_once: Arc<AtomicBool>,
    /// How many `recover()` calls must fail before one succeeds.
    fail_remaining: Arc<AtomicUsize>,
    recover_calls: Arc<AtomicUsize>,
    report_route: Arc<AtomicBool>,
    reported: bool,
}

impl RouteDecoder {
    fn new(fail_remaining: usize) -> (Self, Arc<AtomicBool>, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let fail_once = Arc::new(AtomicBool::new(false));
        let fail_remaining = Arc::new(AtomicUsize::new(fail_remaining));
        let recover_calls = Arc::new(AtomicUsize::new(0));
        let report_route = Arc::new(AtomicBool::new(false));
        (
            Self {
                pts: Duration::ZERO,
                session: None,
                fail_once: Arc::clone(&fail_once),
                fail_remaining: Arc::clone(&fail_remaining),
                recover_calls: Arc::clone(&recover_calls),
                report_route: Arc::clone(&report_route),
                reported: false,
            },
            fail_once,
            recover_calls,
            report_route,
        )
    }
}

impl VideoDecoder for RouteDecoder {
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
        // Report exactly once, once the test has armed the failure. This mirrors
        // the fixed backend: one component failure per cycle, not two.
        if !self.reported && self.fail_once.swap(false, Ordering::AcqRel) {
            if let Some(session) = &self.session {
                session.failed("audio.setup", "device switching");
            }
            self.reported = true;
        }
        if let Some(session) = &self.session {
            if self.report_route.swap(false, Ordering::AcqRel) {
                session.route_changed();
            }
        }
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

    fn attach_session(&mut self, session: PlaybackSession) {
        self.session = Some(session);
    }

    fn recover(&mut self) -> bool {
        let call = self.recover_calls.fetch_add(1, Ordering::AcqRel) + 1;
        call > self.fail_remaining.load(Ordering::Acquire)
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

fn count_events(events: &[SessionEvent], wanted: &SessionEvent) -> usize {
    events.iter().filter(|event| *event == wanted).count()
}

/// A decoder that fails once through the session (not through `Shared::fail`)
/// and then publishes again — the Apple/Android/HLS `session.failed()` path.
struct SessionReportedDecoder {
    pts: Duration,
    session: Option<PlaybackSession>,
    reported: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

impl VideoDecoder for SessionReportedDecoder {
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
        if !self.reported.swap(true, Ordering::AcqRel) {
            // Report exactly like apple.rs:489/515: arm the session AND return a
            // non-transient Decode error, which the worker turns into the public
            // Failed state (`recovering` stays false — that is the bug).
            if let Some(session) = &self.session {
                session.failed("apple.stream", "stream stopped");
            }
            return Err(VideoError::Decode("stream stopped".to_owned()));
        }
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

    fn attach_session(&mut self, session: PlaybackSession) {
        self.session = Some(session);
    }

    fn recover(&mut self) -> bool {
        // The rebuilt reader can publish again from this point.
        self.done.store(true, Ordering::Release);
        true
    }
}

#[test]
fn a_session_reported_failure_resumes_to_playing_after_recovery() {
    // apple.rs / android.rs / hls arm recovery via `session.failed()` without a
    // transient Decode error, so the public `recovering` flag stays false. After
    // the rebuild the next frame must still clear `Failed` and emit Playing.
    let decoder = SessionReportedDecoder {
        pts: Duration::ZERO,
        session: None,
        reported: Arc::new(AtomicBool::new(false)),
        done: Arc::new(AtomicBool::new(false)),
    };
    let mut player = VideoPlayer::with_decoder(decoder).expect("the player opens");
    player.play().expect("play starts");

    assert!(
        wait_until(|| matches!(player.state(), VideoState::Failed(_))),
        "the session-reported failure must reach the public state"
    );
    assert!(
        wait_until(|| matches!(player.state(), VideoState::Playing)),
        "a frame after recovery must clear Failed and resume Playing"
    );
    player.close();
}

#[test]
fn recovery_restores_sound_after_several_failed_attempts() {
    let (decoder, fail_once, recover_calls, _) = RouteDecoder::new(3);
    let mut player = VideoPlayer::with_decoder(decoder).expect("the player opens");
    player.play().expect("play starts");
    fail_once.store(true, Ordering::Release);

    assert!(
        wait_until(|| recover_calls.load(Ordering::Acquire) >= 4),
        "four recovery attempts should be made"
    );
    assert!(
        wait_until(|| player
            .recovery_events()
            .iter()
            .any(|event| matches!(event, SessionEvent::Recovered { .. }))),
        "sound must eventually come back"
    );
    assert_eq!(player.state(), VideoState::Playing);
    assert!(player.diagnostics().frames_published() > 0);
    player.close();
}

#[test]
fn repeated_failures_never_give_up_and_never_fail_the_picture() {
    let (decoder, fail_once, recover_calls, _) = RouteDecoder::new(usize::MAX);
    let mut player = VideoPlayer::with_decoder(decoder).expect("the player opens");
    player.play().expect("play starts");
    fail_once.store(true, Ordering::Release);

    assert!(
        wait_until(|| recover_calls.load(Ordering::Acquire) >= 4),
        "retries must continue past four attempts"
    );
    assert!(
        matches!(player.state(), VideoState::Playing | VideoState::Loading),
        "an audio failure must not fail the picture, state is {:?}",
        player.state()
    );
    assert!(
        player.recovery_events().iter().any(|event| matches!(
            event,
            SessionEvent::Failed { stage, .. } if *stage == "audio.setup"
        )),
        "the component failure is visible in the history"
    );
    let before = player.diagnostics().frames_published();
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        player.diagnostics().frames_published() > before,
        "frames keep coming while audio is recovering"
    );
    player.close();
}

#[test]
fn a_route_change_arms_exactly_one_verification() {
    let (decoder, _, recover_calls, report_route) = RouteDecoder::new(0);
    let mut player = VideoPlayer::with_decoder(decoder).expect("the player opens");
    player.play().expect("play starts");
    report_route.store(true, Ordering::Release);

    assert!(
        wait_until(|| recover_calls.load(Ordering::Acquire) >= 1),
        "a route change must be verified"
    );
    assert!(
        wait_until(|| player
            .recovery_events()
            .iter()
            .any(|event| matches!(event, SessionEvent::Recovered { .. }))),
        "the verification must settle"
    );
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        recover_calls.load(Ordering::Acquire),
        1,
        "one route change must not become a rebind loop"
    );
    assert_eq!(
        count_events(&player.recovery_events(), &SessionEvent::RouteChanged),
        1
    );
    player.close();
}

#[test]
fn rapid_clip_switches_do_not_inherit_a_stuck_session() {
    for round in 0..8 {
        let (decoder, fail_once, _, _) = RouteDecoder::new(usize::MAX);
        let mut player = VideoPlayer::with_decoder(decoder).expect("the player opens");
        player.play().expect("play starts");
        fail_once.store(true, Ordering::Release);
        // Give the failure a chance to be reported, then swipe away before the
        // cooldown elapses — exactly what a fast Reels swipe does.
        std::thread::sleep(Duration::from_millis(30));
        let began = Instant::now();
        player.close();
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "round {round}: close must not wait for a recovery in progress"
        );
        assert!(
            player
                .recovery_events()
                .last()
                .is_some_and(|event| *event == SessionEvent::Closed),
            "round {round}: Closed must be the last event"
        );
    }
}
