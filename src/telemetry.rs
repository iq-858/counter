//! Additive observability for playback.
//!
//! A player records a bounded history of what happened to it and a few
//! aggregate counters, so a screen or a diagnostic build can answer "why did
//! playback stutter?" after the fact without the crate calling back into the
//! application. Everything here is pull-based: nothing is logged to a global
//! sink, nothing spawns a thread, and reading a snapshot never changes state.
//!
//! The layer is deliberately separate from [`VideoState`](crate::VideoState).
//! That enum describes what a screen shows right now and cannot grow a new
//! variant without breaking callers that match on it. Events describe what
//! happened along the way — including transient things like a clock re-anchor
//! that leave no trace in the current state — and live in their own type.

use std::{
    collections::VecDeque,
    fmt,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use parking_lot::Mutex;
use web_time::Instant;

use crate::{PixelFormat, VideoError, VideoInfo, VideoState};

/// How many events one session keeps. The buffer is a ring: once full, the
/// oldest record is dropped so a long-lived stream cannot grow memory without
/// bound.
const EVENT_CAP: usize = 256;

/// How many authoritative publication stamps one session keeps. Each published
/// picture contributes one `(wall time, media time)` pair; a 15-second 60 fps
/// probe needs 900, so this leaves headroom for longer runs without growing
/// memory without bound.
const PUBLICATION_CAP: usize = 4096;

/// How much of the newest sample each rolling cost average keeps.
///
/// A tenth drops a stall that has just ended within a dozen pictures, and still
/// holds steady against the run-to-run noise of one decode.
const COST_WEIGHT: f64 = 0.1;

/// Identifies one [`VideoPlayer`](crate::VideoPlayer) session.
///
/// Ids are unique for the life of a process and increase across players, so a
/// screen that recreates a player after a source change can tell the new
/// session's events from the old one's. The value is opaque: it is only useful
/// for correlation in logs and snapshots.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(u64);

impl SessionId {
    fn next() -> Self {
        // A process-wide counter. `usize` keeps this available on 32-bit
        // targets such as wasm32, where a 64-bit atomic may not exist. Wrapping
        // after billions of sessions still leaves an astronomically small
        // chance of a collision with a still-live player.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed).wrapping_add(1) as u64)
    }

    /// The numeric value, for logging and comparison across snapshots.
    pub fn get(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session-{}", self.0)
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({})", self.0)
    }
}

/// Something that happened during a playback session.
///
/// Events are recorded in the order they happen and can be read with
/// [`VideoPlayer::events`](crate::VideoPlayer::events). They supplement, and
/// never replace, [`VideoState`](crate::VideoState): a state says where a player
/// is now, an event says how it got there.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEvent {
    /// The session opened a source and knows its first size and duration.
    Opened,
    /// Frames started moving.
    Playing,
    /// The clock stopped with the picture left on screen.
    Paused,
    /// The play head moved to `position`.
    Seeking {
        /// The position the player asked for, after clamping to the duration.
        position: Duration,
    },
    /// The clock was pulled back to a picture that arrived late at `position`.
    ///
    /// A occasional re-anchor is normal for a network source; a rapid run of
    /// them means the decoder cannot keep up with the clock.
    Reanchored {
        /// The presentation time the clock was moved to.
        position: Duration,
    },
    /// The last picture was shown and playback stopped.
    Ended,
    /// Reading stopped on an error.
    Failed {
        /// The failure also held by [`VideoState::Failed`].
        error: VideoError,
    },
    /// A decoder that did not know the picture size at open time reported it.
    InfoReady {
        /// The picture width in pixels.
        width: u32,
        /// The picture height in pixels.
        height: u32,
    },
    /// The playback rate changed.
    SpeedChanged {
        /// The new rate, where 1.0 is the recorded speed.
        speed: f32,
    },
    /// Audio was muted or unmuted.
    Muted {
        /// Whether sound is now silenced.
        muted: bool,
    },
    /// The audio volume changed.
    VolumeChanged {
        /// The new level, from 0.0 to 1.0.
        volume: f32,
    },
    /// The player was closed and its decoder released.
    Closed,
}

/// One entry in a session's event history.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRecord {
    pub(crate) sequence: u64,
    pub(crate) at: Duration,
    pub(crate) position: Duration,
    pub(crate) event: PlaybackEvent,
}

impl EventRecord {
    /// The record's one-based place in the session, increasing with every
    /// event even after older records leave the bounded buffer.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// How long after the session opened this happened.
    pub fn at(&self) -> Duration {
        self.at
    }

    /// Where the play head was when this happened.
    pub fn position(&self) -> Duration {
        self.position
    }

    /// What happened.
    pub fn event(&self) -> &PlaybackEvent {
        &self.event
    }
}

/// The bounded, oldest-first event history of one session.
#[derive(Debug, Clone)]
pub(crate) struct EventLog {
    records: VecDeque<EventRecord>,
    sequence: u64,
}

impl EventLog {
    fn new() -> Self {
        Self {
            records: VecDeque::new(),
            sequence: 0,
        }
    }

    fn push(&mut self, event: PlaybackEvent, at: Duration, position: Duration) {
        self.sequence = self.sequence.wrapping_add(1);
        if self.records.len() == EVENT_CAP {
            self.records.pop_front();
        }
        self.records.push_back(EventRecord {
            sequence: self.sequence,
            at,
            position,
            event,
        });
    }

    pub(crate) fn snapshot(&self) -> Vec<EventRecord> {
        self.records.iter().cloned().collect()
    }

    pub(crate) fn len(&self) -> usize {
        self.records.len()
    }
}

/// Aggregate counts, updated as events are emitted.
#[derive(Debug, Clone, Copy, Default)]
struct Counters {
    frames: u64,
    reanchors: u64,
    errors: u64,
    seeks: u64,
    plays: u64,
    /// Average cost of one decode, in microseconds.
    decode_micros: f64,
    /// Average cost of one NV12 to RGBA conversion, in microseconds.
    convert_micros: f64,
}

/// Folds one measured cost into a rolling average.
fn average(previous: f64, sample: Duration) -> f64 {
    let micros = sample.as_secs_f64() * 1e6;
    if previous == 0.0 {
        micros
    } else {
        previous + (micros - previous) * COST_WEIGHT
    }
}

struct TelemetryInner {
    session: SessionId,
    started: Instant,
    events: EventLog,
    counters: Counters,
    /// Authoritative publication timeline: the wall time each picture was
    /// published and its presentation time. Unlike a polled single-slot slot,
    /// this cannot miss a picture between two samples, so pacing probes can
    /// measure the player rather than their own polling cadence.
    publications: VecDeque<(Instant, Duration)>,
}

/// The counters and identity a diagnostics snapshot needs from the telemetry
/// side, collected under one lock.
pub(crate) struct TelemetryView {
    pub(crate) session: SessionId,
    pub(crate) uptime: Duration,
    pub(crate) frames: u64,
    pub(crate) reanchors: u64,
    pub(crate) errors: u64,
    pub(crate) seeks: u64,
    pub(crate) plays: u64,
    pub(crate) decode_time: Duration,
    pub(crate) convert_time: Duration,
    pub(crate) event_count: usize,
}

/// Poll-able telemetry shared by the screen thread and the decode thread.
///
/// Every method takes the inner lock on its own and holds it only for a
/// constant-time update or copy, so emitting an event from a hot decode path
/// never holds a lock across a read of another subsystem.
pub(crate) struct Telemetry {
    inner: Mutex<TelemetryInner>,
}

impl Telemetry {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(TelemetryInner {
                session: SessionId::next(),
                started: Instant::now(),
                events: EventLog::new(),
                counters: Counters::default(),
                publications: VecDeque::with_capacity(PUBLICATION_CAP),
            }),
        }
    }

    pub(crate) fn session(&self) -> SessionId {
        self.inner.lock().session
    }

    /// Records an event and folds it into the aggregate counters.
    pub(crate) fn emit(&self, event: PlaybackEvent, position: Duration) {
        let mut inner = self.inner.lock();
        match &event {
            PlaybackEvent::Playing => inner.counters.plays += 1,
            PlaybackEvent::Seeking { .. } => inner.counters.seeks += 1,
            PlaybackEvent::Reanchored { .. } => inner.counters.reanchors += 1,
            PlaybackEvent::Failed { .. } => inner.counters.errors += 1,
            _ => {}
        }
        let at = inner.started.elapsed();
        inner.events.push(event, at, position);
    }

    /// Counts a picture published to the screen. Frames are counted rather
    /// than logged: logging every frame would evict the events that explain a
    /// stall from the bounded buffer within a second.
    pub(crate) fn note_frame(&self) {
        self.inner.lock().counters.frames += 1;
    }

    /// Records one authoritative publication stamp. Called on the decode thread
    /// next to `note_frame`, so the two always describe the same picture.
    pub(crate) fn note_publication(&self, at: Instant, pts: Duration) {
        let mut inner = self.inner.lock();
        if inner.publications.len() == PUBLICATION_CAP {
            inner.publications.pop_front();
        }
        inner.publications.push_back((at, pts));
    }

    /// A snapshot of the publication timeline, oldest first.
    pub(crate) fn publication_timeline(&self) -> Vec<(Instant, Duration)> {
        self.inner.lock().publications.iter().copied().collect()
    }

    pub(crate) fn events(&self) -> Vec<EventRecord> {
        self.inner.lock().events.snapshot()
    }

    /// Folds one decode's cost into the rolling average.
    ///
    /// Called on the decode thread for every picture, so it takes the lock once
    /// and does constant work.
    pub(crate) fn note_decode(&self, took: Duration) {
        let mut inner = self.inner.lock();
        inner.counters.decode_micros = average(inner.counters.decode_micros, took);
    }

    /// Folds one NV12 to RGBA conversion into the rolling average.
    pub(crate) fn note_convert(&self, took: Duration) {
        let mut inner = self.inner.lock();
        inner.counters.convert_micros = average(inner.counters.convert_micros, took);
    }

    pub(crate) fn view(&self) -> TelemetryView {
        let inner = self.inner.lock();
        TelemetryView {
            session: inner.session,
            uptime: inner.started.elapsed(),
            frames: inner.counters.frames,
            reanchors: inner.counters.reanchors,
            errors: inner.counters.errors,
            seeks: inner.counters.seeks,
            plays: inner.counters.plays,
            decode_time: Duration::from_secs_f64(inner.counters.decode_micros / 1e6),
            convert_time: Duration::from_secs_f64(inner.counters.convert_micros / 1e6),
            event_count: inner.events.len(),
        }
    }
}

/// A point-in-time picture of a playback session.
///
/// Take one with [`VideoPlayer::diagnostics`](crate::VideoPlayer::diagnostics)
/// whenever a screen redraws or a test checks playback. A snapshot is a plain
/// value: it neither borrows the player nor changes as the session goes on.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackDiagnostics {
    pub(crate) session: SessionId,
    pub(crate) state: VideoState,
    pub(crate) position: Duration,
    pub(crate) duration: Option<Duration>,
    pub(crate) speed: f32,
    pub(crate) is_live: bool,
    pub(crate) is_seekable: bool,
    pub(crate) muted: bool,
    pub(crate) volume: f32,
    pub(crate) info: VideoInfo,
    pub(crate) uptime: Duration,
    pub(crate) decode_time: Duration,
    pub(crate) convert_time: Duration,
    pub(crate) pixel_format: PixelFormat,
    pub(crate) frames_published: u64,
    pub(crate) reanchors: u64,
    pub(crate) errors: u64,
    pub(crate) seeks: u64,
    pub(crate) plays: u64,
    pub(crate) event_count: usize,
}

impl PlaybackDiagnostics {
    /// The session this snapshot describes.
    pub fn session(&self) -> SessionId {
        self.session
    }

    /// The state at the moment the snapshot was taken.
    pub fn state(&self) -> &VideoState {
        &self.state
    }

    /// The play head, between zero and `duration`.
    pub fn position(&self) -> Duration {
        self.position
    }

    /// The media length, when the container states one.
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// The playback rate, where 1.0 is the recorded speed.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Whether the source keeps adding media as it plays.
    pub fn is_live(&self) -> bool {
        self.is_live
    }

    /// Whether the source can be seeked.
    pub fn is_seekable(&self) -> bool {
        self.is_seekable
    }

    /// Whether audio a backend carries is silenced.
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// The level of audio a backend carries, from 0.0 to 1.0.
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// The size, duration and rotation the decoder knows.
    pub fn info(&self) -> VideoInfo {
        self.info
    }

    /// How long this session has been alive.
    pub fn uptime(&self) -> Duration {
        self.uptime
    }

    /// How many pictures have been published to the screen.
    pub fn frames_published(&self) -> u64 {
        self.frames_published
    }

    /// How many times the clock was pulled back to a late picture.
    pub fn reanchors(&self) -> u64 {
        self.reanchors
    }

    /// How many times decoding stopped on an error.
    pub fn errors(&self) -> u64 {
        self.errors
    }

    /// How many repositions were requested, including stop and restart.
    pub fn seeks(&self) -> u64 {
        self.seeks
    }

    /// How many times playback started or resumed.
    pub fn plays(&self) -> u64 {
        self.plays
    }

    /// The average time one picture took to decode, over the newest pictures.
    ///
    /// A value past the frame interval is the signature of a decoder that
    /// cannot keep up with the source: a 60 Hz picture has 16.6 ms for the
    /// decode and the conversion together.
    pub fn decode_time(&self) -> Duration {
        self.decode_time
    }

    /// The average time one NV12 picture took to convert to RGBA.
    ///
    /// Zero when the decoder produces RGBA itself, or when the screen samples
    /// the planes so no conversion runs at all.
    pub fn convert_time(&self) -> Duration {
        self.convert_time
    }

    /// The layout this session's pictures arrive in.
    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    /// How many events are currently retained in the bounded history.
    pub fn event_count(&self) -> usize {
        self.event_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn telemetry() -> Telemetry {
        Telemetry::new()
    }

    #[test]
    fn session_ids_are_unique() {
        let a = telemetry();
        let b = telemetry();
        assert_ne!(a.session(), b.session());
        assert!(a.session().get() > 0);
    }

    #[test]
    fn events_are_ordered_and_numbered_from_one() {
        let log = telemetry();
        log.emit(PlaybackEvent::Opened, Duration::ZERO);
        log.emit(
            PlaybackEvent::Seeking {
                position: Duration::from_secs(2),
            },
            Duration::from_secs(2),
        );
        let records = log.events();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].sequence(), 1);
        assert_eq!(records[1].sequence(), 2);
        assert_eq!(records[0].event(), &PlaybackEvent::Opened);
        assert_eq!(records[1].position(), Duration::from_secs(2));
        assert!(
            records[0].at() <= records[1].at(),
            "records keep the order they were emitted"
        );
    }

    #[test]
    fn a_log_keeps_only_the_newest_events() {
        let log = telemetry();
        for second in 0..(EVENT_CAP + 10) {
            log.emit(
                PlaybackEvent::Reanchored {
                    position: Duration::from_secs(second as u64),
                },
                Duration::ZERO,
            );
        }
        let records = log.events();
        assert_eq!(records.len(), EVENT_CAP);
        assert_eq!(
            records[0].sequence(),
            11,
            "the oldest ten records were dropped"
        );
        assert_eq!(records[EVENT_CAP - 1].sequence(), (EVENT_CAP + 10) as u64);
    }

    #[test]
    fn measured_costs_average_towards_the_newest_pictures() {
        let log = telemetry();
        log.note_decode(Duration::from_millis(4));
        assert_eq!(log.view().decode_time, Duration::from_millis(4));
        log.note_decode(Duration::from_millis(14));
        let average = log.view().decode_time;
        assert!(
            average > Duration::from_millis(4) && average < Duration::from_millis(14),
            "a new sample must move the average: {average:?}"
        );
        assert_eq!(
            log.view().convert_time,
            Duration::ZERO,
            "a cost that never ran stays zero"
        );
    }

    #[test]
    fn counters_fold_in_events_and_frames() {
        let log = telemetry();
        log.emit(PlaybackEvent::Playing, Duration::ZERO);
        log.emit(
            PlaybackEvent::Seeking {
                position: Duration::ZERO,
            },
            Duration::ZERO,
        );
        log.emit(
            PlaybackEvent::Reanchored {
                position: Duration::ZERO,
            },
            Duration::ZERO,
        );
        log.emit(
            PlaybackEvent::Failed {
                error: VideoError::NoVideoTrack,
            },
            Duration::ZERO,
        );
        log.note_frame();
        log.note_frame();
        let view = log.view();
        assert_eq!(view.plays, 1);
        assert_eq!(view.seeks, 1);
        assert_eq!(view.reanchors, 1);
        assert_eq!(view.errors, 1);
        assert_eq!(view.frames, 2);
        assert_eq!(view.event_count, 4);
    }
}
