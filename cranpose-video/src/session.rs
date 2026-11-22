//! Playback session lifecycle and recovery, shared by every native target.
//!
//! This module is additive: it introduces a new public type and new defaulted
//! decoder hooks. Nothing in the existing player API changes shape, and a
//! decoder that ignores the hooks behaves exactly as it did before.
//!
//! A session is deliberately not an operating-system media session. It is the
//! library's own record of what one player is doing right now — playing,
//! interrupted, recovering — and the bounded retry policy that brings sound
//! back after a failure. Operating systems give us no portable interruption
//! callback, so every failure detector in the crate reports into this object
//! instead: a failed audio prepare, a lost output device, a route change, a
//! dead stream, a broken network source. Recovery is a cooldown, never a
//! shutdown: a device that was busy a moment ago can be available now, so an
//! open player keeps trying for as long as it lives.
//!
//! Linux is intentionally outside this module. Nothing here is compiled or
//! wired on that target.

use crate::diagnostics::redact_for_report;

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// How long a failed recovery is left alone before it may be tried again.
///
/// A retry throttle, not a shutdown deadline. It stops a missing output device
/// from being polled on every decoded frame while still letting a device that
/// appears a moment later be picked up.
pub const RECOVERY_COOLDOWN: Duration = Duration::from_millis(400);

/// The longest delay between two recovery attempts.
///
/// Repeated failures back off geometrically up to this bound and stay there.
/// There is no terminal "give up" state: an open player always tries again.
pub const RECOVERY_COOLDOWN_MAX: Duration = Duration::from_secs(4);

/// How many events one session keeps. The buffer is a ring, so a long-lived
/// player that loses its device repeatedly cannot grow without bound.
const EVENT_CAPACITY: usize = 64;

/// The lifecycle phase of one playback session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    /// The source is being opened; no sound has been committed yet.
    Opening,
    /// Sound and picture are running normally.
    Active,
    /// The system took the audio away (focus loss, device removal, route loss).
    Interrupted,
    /// The last recovery attempt failed; another is scheduled.
    Recovering,
    /// The player was closed. No further transitions are accepted.
    Closed,
}

/// Why a session stopped hearing sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptionReason {
    /// Another app took audio focus (Android).
    AudioFocusLoss,
    /// The output device disappeared or its stream died.
    DeviceLost,
    /// The system default output changed.
    RouteChanged,
    /// The media source failed to load or stalled on the network.
    Network,
    /// A failure with no more specific classification.
    Unknown,
}

/// Something that happened to one playback session.
///
/// Events are diagnostic: they never drive playback directly. The player and
/// the audio backends own the actual pause/reopen actions and report their
/// outcome here so the application can see the recovery as it happens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// The player opened its source.
    Opened,
    /// Sound was taken away; the picture keeps running.
    Interrupted(InterruptionReason),
    /// Sound came back after an interruption.
    Resumed,
    /// The default output changed. Recovery follows only if it is needed.
    RouteChanged,
    /// A component failed. `stage` names it, `reason` is its human-readable cause.
    Failed {
        /// The component that failed, such as `audio.prepare` or `hls.open`.
        stage: &'static str,
        /// The human-readable cause of the failure.
        reason: String,
    },
    /// A recovery attempt is being made.
    Recovering {
        /// The one-based number of this attempt.
        attempt: u32,
    },
    /// A recovery attempt succeeded.
    Recovered {
        /// The one-based number of the attempt that succeeded.
        attempt: u32,
    },
    /// The player closed. This is always the last event.
    Closed,
}

#[derive(Debug)]
struct Phase {
    state: PlaybackState,
    reason: Option<InterruptionReason>,
    /// Consecutive failures, used only to grow the backoff.
    failures: u32,
    /// Recovery attempts started, used for event numbering.
    attempts: u32,
    retry_at: Option<Instant>,
}

/// A cloneable handle to one session's lifecycle state.
///
/// All clones share the same state. Cloning never starts a new session; it is
/// how the player hands the same session to the decoder on its worker thread.
#[derive(Clone)]
pub struct PlaybackSession {
    inner: Arc<SessionInner>,
}

struct SessionInner {
    phase: std::sync::Mutex<Phase>,
    events: std::sync::Mutex<VecDeque<SessionEvent>>,
    /// Bumped on every deliberate discontinuity so a stale device callback can
    /// be ignored by whoever is listening.
    generation: AtomicU64,
    /// Set by `close()` before `Closed` is appended. A detached worker that
    /// passed its phase check before the close can still reach `push()` later;
    /// this seal drops that late event so `Closed` stays last.
    closed: AtomicBool,
}

impl Default for PlaybackSession {
    fn default() -> Self {
        Self::new()
    }
}

/// Weak handles to every live session, so a platform route observer can arm a
/// verification on all of them without the app having to track its player.
/// Weak references mean a closed player is simply skipped.
#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
static LIVE_SESSIONS: std::sync::OnceLock<std::sync::Mutex<Vec<std::sync::Weak<SessionInner>>>> =
    std::sync::OnceLock::new();

#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
fn live_sessions() -> &'static std::sync::Mutex<Vec<std::sync::Weak<SessionInner>>> {
    LIVE_SESSIONS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Bumped on every system audio-route change (headset unplug, Bluetooth).
///
/// A route change must NOT force the player into `Recovering`: that stops frame
/// publication, which froze both picture and sound when headphones were
/// unplugged. Instead the generic audio backend polls this counter and re-binds
/// only its own output stream. A decoder that owns its sound (AVPlayer) ignores
/// it — AVAudioSession re-routes AVPlayer automatically.
#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
static ROUTE_CHANGE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The current route-change generation, for a backend to notice a new route.
#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
pub(crate) fn route_change_generation() -> u64 {
    ROUTE_CHANGE_GENERATION.load(Ordering::Acquire)
}

impl PlaybackSession {
    /// Starts a session in [`PlaybackState::Opening`].
    pub fn new() -> Self {
        let session = Self {
            inner: Arc::new(SessionInner {
                phase: std::sync::Mutex::new(Phase {
                    state: PlaybackState::Opening,
                    reason: None,
                    failures: 0,
                    attempts: 0,
                    retry_at: None,
                }),
                events: std::sync::Mutex::new(VecDeque::new()),
                generation: AtomicU64::new(0),
                closed: AtomicBool::new(false),
            }),
        };
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        live_sessions()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::downgrade(&session.inner));
        session
    }

    /// Records a system audio-route change on every live session. A platform
    /// route observer (headset unplug, Bluetooth connect) calls this. The generic
    /// audio backend re-binds its own stream by polling the generation counter;
    /// the picture and the session phase are deliberately left untouched.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub fn route_changed_all() {
        // A route change must never stop the frame pump. Forcing `Recovering`
        // here made the worker pause publication and wait, which froze picture
        // AND sound when headphones were unplugged; only toggling play restored
        // it. Instead, bump a generation counter the generic audio backend
        // polls, and record the event on each session for observability. The
        // backend re-binds its output without disturbing the picture. Decoders
        // that own their sound (AVPlayer) re-route through AVAudioSession on
        // their own and ignore the counter.
        ROUTE_CHANGE_GENERATION.fetch_add(1, Ordering::AcqRel);
        let mut live = live_sessions().lock().unwrap_or_else(|e| e.into_inner());
        live.retain(|weak| weak.strong_count() > 0);
        let sessions: Vec<_> = live.iter().filter_map(|weak| weak.upgrade()).collect();
        drop(live);
        for inner in sessions {
            let session = PlaybackSession { inner };
            // Record only; do not transition to Recovering or arm a retry.
            session.push(SessionEvent::RouteChanged);
        }
    }

    /// The current lifecycle phase.
    pub fn state(&self) -> PlaybackState {
        self.with_phase(|phase| phase.state)
    }

    /// The reason sound was taken away, when that is known.
    pub fn interruption_reason(&self) -> Option<InterruptionReason> {
        self.with_phase(|phase| phase.reason)
    }

    /// A monotonically increasing discontinuity counter.
    ///
    /// Device callbacks that outlive their player can be discarded by comparing
    /// the generation they were created with against this one.
    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::Acquire)
    }

    /// A snapshot of the most recent events, oldest first.
    pub fn events(&self) -> Vec<SessionEvent> {
        self.inner
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// When the next recovery attempt is due, if one is scheduled.
    ///
    /// The decode thread uses this to wait for a deadline instead of polling:
    /// a parked player costs nothing until a failure actually arms a retry.
    pub fn next_retry(&self) -> Option<Instant> {
        self.with_phase(|phase| {
            if matches!(
                phase.state,
                PlaybackState::Recovering | PlaybackState::Interrupted
            ) {
                phase.retry_at
            } else {
                None
            }
        })
    }

    /// Records that the source opened successfully.
    pub fn opened(&self) {
        self.transition(PlaybackState::Active, None, Some(SessionEvent::Opened));
    }

    /// Records that sound was taken away while the picture continues.
    ///
    /// A retry is armed as well: if the system never sends an explicit resume,
    /// the player still checks whether sound can be brought back.
    pub fn interrupted(&self, reason: InterruptionReason) {
        {
            let mut phase = self.lock_phase();
            if phase.state == PlaybackState::Closed {
                return;
            }
            phase.state = PlaybackState::Interrupted;
            phase.reason = Some(reason);
            phase.retry_at = Some(Instant::now() + RECOVERY_COOLDOWN);
        }
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
        self.push(SessionEvent::Interrupted(reason));
    }

    /// Records that sound came back.
    pub fn resumed(&self) {
        self.transition(PlaybackState::Active, None, Some(SessionEvent::Resumed));
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Records a system default-output change and arms one verification attempt.
    pub fn route_changed(&self) {
        {
            let mut phase = self.lock_phase();
            if phase.state == PlaybackState::Closed {
                return;
            }
            if phase.state == PlaybackState::Active {
                phase.state = PlaybackState::Recovering;
            }
            if phase.retry_at.is_none() {
                phase.retry_at = Some(Instant::now() + RECOVERY_COOLDOWN);
            }
        }
        self.push(SessionEvent::RouteChanged);
    }

    /// Records a component failure and schedules the next recovery attempt.
    ///
    /// The cooldown grows with consecutive failures but never becomes a
    /// terminal state: an open player always has another attempt waiting.
    /// Failures before the first open are recorded too, so a native audio setup
    /// that failed inside `open` is still recovered once the decode thread runs.
    pub fn failed(&self, stage: &'static str, reason: impl Into<String>) {
        // Session history is readable by the application and may be written to
        // a field log. A reason that names a live URL must not carry its token.
        let reason = redact_for_report(&reason.into());
        {
            let mut phase = self.lock_phase();
            if phase.state == PlaybackState::Closed {
                // `Closed` is the last event of a session. A late failure from a
                // detached worker is dropped, never appended after it, so an
                // observer that stops at `Closed` cannot miss a later event.
                return;
            }
            phase.failures = phase.failures.saturating_add(1);
            let shift = phase.failures.saturating_sub(1).min(4);
            let cooldown = RECOVERY_COOLDOWN
                .saturating_mul(1u32 << shift)
                .min(RECOVERY_COOLDOWN_MAX);
            phase.retry_at = Some(Instant::now() + cooldown);
            if phase.state != PlaybackState::Interrupted {
                phase.state = PlaybackState::Recovering;
            }
        }
        self.push(SessionEvent::Failed { stage, reason });
    }

    /// Whether a recovery attempt is due now.
    pub fn recovery_due(&self) -> bool {
        self.with_phase(|phase| {
            matches!(
                phase.state,
                PlaybackState::Recovering | PlaybackState::Interrupted
            ) && phase.retry_at.is_some_and(|at| Instant::now() >= at)
        })
    }

    /// Starts a recovery attempt, returning its one-based number.
    ///
    /// Returns `None` when the session is closed or no attempt is due.
    pub fn begin_recovery(&self) -> Option<u32> {
        {
            let mut phase = self.lock_phase();
            if phase.state == PlaybackState::Closed {
                return None;
            }
            // Only a failure, an interruption or a route change arms an attempt.
            // An active session carries no `retry_at`, and without this test
            // every pass of the worker's loop looked like a due recovery: the
            // reader and the output device were rebuilt on every frame, so a
            // fresh audio stream was torn down before it could be heard. That
            // is exactly what a video "with no sound" was.
            if !matches!(
                phase.state,
                PlaybackState::Recovering | PlaybackState::Interrupted
            ) {
                return None;
            }
            if !phase.retry_at.is_none_or(|at| Instant::now() >= at) {
                return None;
            }
            phase.attempts = phase.attempts.saturating_add(1);
            let attempt = phase.attempts;
            phase.retry_at = None;
            phase.state = PlaybackState::Recovering;
            phase.reason = None;
            drop(phase);
            self.inner.generation.fetch_add(1, Ordering::AcqRel);
            self.push(SessionEvent::Recovering { attempt });
            Some(attempt)
        }
    }

    /// Records that a recovery attempt succeeded.
    pub fn recovered(&self) {
        {
            let mut phase = self.lock_phase();
            if phase.state == PlaybackState::Closed {
                return;
            }
            let attempt = phase.attempts;
            phase.state = PlaybackState::Active;
            phase.reason = None;
            phase.failures = 0;
            phase.attempts = 0;
            phase.retry_at = None;
            drop(phase);
            self.inner.generation.fetch_add(1, Ordering::AcqRel);
            self.push(SessionEvent::Recovered { attempt });
        }
    }

    /// Closes the session. Every later transition is ignored.
    pub fn close(&self) {
        {
            let mut phase = self.lock_phase();
            if phase.state == PlaybackState::Closed {
                return;
            }
            phase.state = PlaybackState::Closed;
            phase.reason = None;
            phase.retry_at = None;
        }
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
        // Seal before publishing `Closed`. A worker whose earlier phase check
        // saw an open session now reaches `push()` after the seal and is dropped
        // there, so no event can follow `Closed`.
        self.inner.closed.store(true, Ordering::Release);
        self.push(SessionEvent::Closed);
    }

    fn transition(
        &self,
        state: PlaybackState,
        reason: Option<InterruptionReason>,
        event: Option<SessionEvent>,
    ) {
        {
            let mut phase = self.lock_phase();
            if phase.state == PlaybackState::Closed {
                return;
            }
            phase.state = state;
            phase.reason = reason;
            if state == PlaybackState::Active {
                phase.failures = 0;
                phase.attempts = 0;
                phase.retry_at = None;
            }
        }
        if let Some(event) = event {
            self.push(event);
        }
    }

    fn push(&self, event: SessionEvent) {
        let mut events = self
            .inner
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // The seal is read while holding the events lock. That serialises with
        // `close()`'s own `Closed` append: a late worker either appends before
        // the seal and is followed by `Closed`, or appends after it and drops.
        if self.inner.closed.load(Ordering::Acquire) && !matches!(event, SessionEvent::Closed) {
            return;
        }
        if events.len() == EVENT_CAPACITY {
            events.pop_front();
        }
        events.push_back(event);
    }

    fn lock_phase(&self) -> std::sync::MutexGuard<'_, Phase> {
        self.inner
            .phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn with_phase<T>(&self, read: impl FnOnce(&Phase) -> T) -> T {
        let phase = self.lock_phase();
        read(&phase)
    }

    /// Moves a scheduled retry into the past without sleeping.
    ///
    /// Device-free tests use this to exercise the recovery path deterministically.
    #[cfg(test)]
    pub(crate) fn expire_retry_for_test(&self) {
        let mut phase = self.lock_phase();
        phase.retry_at = Some(Instant::now() - Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_is_ordered_and_closed_is_last() {
        let session = PlaybackSession::new();
        assert_eq!(session.state(), PlaybackState::Opening);
        session.opened();
        assert_eq!(session.state(), PlaybackState::Active);
        session.interrupted(InterruptionReason::DeviceLost);
        assert_eq!(session.state(), PlaybackState::Interrupted);
        assert_eq!(
            session.interruption_reason(),
            Some(InterruptionReason::DeviceLost)
        );
        session.resumed();
        assert_eq!(session.state(), PlaybackState::Active);
        session.close();
        assert_eq!(session.state(), PlaybackState::Closed);
        session.failed("late", "ignored");
        assert_eq!(session.state(), PlaybackState::Closed);
        let events = session.events();
        assert_eq!(events.last(), Some(&SessionEvent::Closed));
        assert_eq!(events.first(), Some(&SessionEvent::Opened));
    }

    #[test]
    fn a_late_push_after_close_is_sealed() {
        // A detached worker can pass its phase check before `close()` and only
        // reach `push()` afterwards. Bypassing the early check proves the seal
        // at the single append point keeps `Closed` last regardless of order.
        let session = PlaybackSession::new();
        session.opened();
        session.close();
        session.push(SessionEvent::Failed {
            stage: "late.worker",
            reason: "ignored".into(),
        });
        session.push(SessionEvent::Recovering { attempt: 9 });
        let events = session.events();
        assert_eq!(events.last(), Some(&SessionEvent::Closed));
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn an_active_session_has_no_recovery_to_start() {
        let session = PlaybackSession::new();
        session.opened();
        assert_eq!(session.state(), PlaybackState::Active);
        assert!(!session.recovery_due());
        assert_eq!(
            session.begin_recovery(),
            None,
            "an active session must not rebuild the reader or the device"
        );
        // Recovering returns to Active, and still nothing is due afterwards.
        session.failed("audio.prepare", "busy");
        {
            let mut phase = session.lock_phase();
            phase.retry_at = Some(Instant::now() - Duration::from_millis(1));
        }
        assert_eq!(session.begin_recovery(), Some(1));
        session.recovered();
        assert_eq!(session.begin_recovery(), None);
    }

    #[test]
    fn failure_retries_forever_with_a_cooldown() {
        let session = PlaybackSession::new();
        session.opened();
        session.failed("audio.prepare", "busy");
        assert_eq!(session.state(), PlaybackState::Recovering);
        assert!(session.next_retry().is_some());
        assert!(!session.recovery_due());
        assert_eq!(session.begin_recovery(), None);
        // Simulate the cooldown expiring without sleeping in the test.
        {
            let mut phase = session.lock_phase();
            phase.retry_at = Some(Instant::now() - Duration::from_millis(1));
        }
        assert!(session.recovery_due());
        assert_eq!(session.begin_recovery(), Some(1));
        session.recovered();
        assert_eq!(session.state(), PlaybackState::Active);
        assert!(session.next_retry().is_none());
    }

    #[test]
    fn opening_failure_is_still_recovered() {
        let session = PlaybackSession::new();
        session.failed("audio.setup", "no device");
        assert_eq!(session.state(), PlaybackState::Recovering);
        assert!(session.next_retry().is_some());
    }

    #[test]
    fn cooldown_grows_one_step_per_failure_and_caps() {
        // [ROOT CAUSE GUARD] A failed recovery must advance the backoff by one
        // step. Reporting the same failure twice (once in the backend and once
        // in the worker) used to double the shift each cycle and reach the 4 s
        // ceiling after two attempts, so sound stayed away for minutes.
        let session = PlaybackSession::new();
        session.opened();
        let expected = [
            RECOVERY_COOLDOWN,
            RECOVERY_COOLDOWN.saturating_mul(2),
            RECOVERY_COOLDOWN.saturating_mul(4),
            RECOVERY_COOLDOWN.saturating_mul(8),
            RECOVERY_COOLDOWN_MAX,
            RECOVERY_COOLDOWN_MAX,
        ];
        for (index, want) in expected.iter().enumerate() {
            let before = Instant::now();
            session.failed("audio.setup", "busy");
            let at = session.next_retry().expect("a retry is always armed");
            let got = at.duration_since(before);
            assert!(
                got >= *want && got <= want.saturating_add(Duration::from_millis(25)),
                "failure {}: expected about {want:?}, got {got:?}",
                index + 1
            );
        }
    }

    #[test]
    fn failure_history_redacts_live_tokens() {
        let session = PlaybackSession::new();
        session.failed(
            "hls.open",
            "cannot read https://host.example/live.m3u8?token=secret",
        );
        assert!(session.events().contains(&SessionEvent::Failed {
            stage: "hls.open",
            reason: "cannot read https://host.example/live.m3u8?<redacted>".to_owned(),
        }));
    }

    #[test]
    fn event_buffer_is_bounded() {
        let session = PlaybackSession::new();
        for _ in 0..EVENT_CAPACITY + 10 {
            session.route_changed();
        }
        assert_eq!(session.events().len(), EVENT_CAPACITY);
    }
}
