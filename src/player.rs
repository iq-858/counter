//! Playback: a clock, one slot for the newest frame, and the engine that fills
//! it.
//!
//! The player is pull-based. A decode thread decodes ahead of the clock and
//! publishes the newest picture into a slot; the screen calls
//! [`VideoPlayer::take_frame`] when it draws and hands the picture back with
//! [`VideoPlayer::recycle`], so a 30 or 60 Hz screen never waits on a decoder
//! and a picture is allocated once instead of once per frame.
//!
//! On the web there is no thread to decode on: the browser plays the media
//! itself and the player decodes inside `take_frame`, on the calling thread.

#[cfg(target_arch = "wasm32")]
mod inline;
#[cfg(not(target_arch = "wasm32"))]
mod threaded;

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// The one-way signal a decoder checks to abandon a blocking read on close.
type Shutdown = Arc<AtomicBool>;

#[cfg(target_arch = "wasm32")]
use inline::Engine;
use parking_lot::Mutex;
#[cfg(not(target_arch = "wasm32"))]
use threaded::Engine;
use web_time::Instant;

use crate::{
    PixelFormat, VideoDecoder, VideoError, VideoFrame, VideoInfo, VideoState,
    telemetry::{
        EventRecord, PlaybackDiagnostics, PlaybackEvent, SessionId, Telemetry, TelemetryView,
    },
};
#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
use crate::{PlaybackSession, PlaybackState, SessionEvent};

/// How many pixel buffers the slot keeps to hand back to the decoder.
///
/// Two covers the picture on screen and the one being decoded, and bounds the
/// memory a paused player holds to a few frames.
const SPARES: usize = 2;

/// The slowest rate [`VideoPlayer::set_speed`] accepts.
const MIN_SPEED: f32 = 0.0625;

/// The fastest rate [`VideoPlayer::set_speed`] accepts.
const MAX_SPEED: f32 = 8.0;

/// The frame duration assumed when neither the container nor the decoder
/// states a rate, and the decoder repeats a presentation time.
const DEFAULT_FRAME: Duration = Duration::from_millis(33);

/// What the screen asks the engine to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Command {
    /// Frames are moving. The clock is already started.
    Play,
    /// Frames are still.
    Pause,
    /// Back to the first frame.
    Stop,
    /// Move to a position.
    Seek(Duration),
    /// Play at another rate.
    SetSpeed(f32),
    /// Silence or unmute audio a backend carries itself.
    SetMuted(bool),
    /// Set the volume of audio a backend carries itself, from 0.0 to 1.0.
    SetVolume(f32),
    /// Assign the audio group (same group = mutually exclusive sound).
    SetAudioGroup(u64),
    /// Select an audio track by backend index, or turn sound off with `None`.
    SelectAudioTrack(Option<usize>),
    /// Select a subtitle track by backend index, or turn subtitles off.
    SelectSubtitleTrack(Option<usize>),
    /// Repeat only this section, or the whole source again with `None`.
    SetLoopRange(Option<crate::loop_range::LoopRange>),
    /// The player is going away.
    Shutdown,
}

/// Where playback is, between one published picture and the next.
///
/// A picture anchors the clock at its presentation time, and the clock counts
/// real time from there at the playback speed. Reading the position between
/// pictures therefore moves smoothly rather than stepping once per frame, and a
/// decoder that falls behind holds the clock back with it instead of letting it
/// run to the end of the media.
pub(crate) struct Clock {
    anchor: Instant,
    base: Duration,
    speed: f32,
    running: bool,
}

impl Clock {
    fn new() -> Self {
        Self {
            anchor: Instant::now(),
            base: Duration::ZERO,
            speed: 1.0,
            running: false,
        }
    }

    fn position(&self) -> Duration {
        if !self.running {
            return self.base;
        }
        let seconds = self.anchor.elapsed().as_secs_f64() * f64::from(self.speed);
        self.base
            .saturating_add(Duration::from_secs_f64(seconds.max(0.0)))
    }

    fn set(&mut self, position: Duration) {
        self.base = position;
        self.anchor = Instant::now();
    }

    fn set_running(&mut self, running: bool) {
        if self.running == running {
            return;
        }
        self.base = self.position();
        self.anchor = Instant::now();
        self.running = running;
    }

    fn set_speed(&mut self, speed: f32) {
        self.base = self.position();
        self.anchor = Instant::now();
        self.speed = speed;
    }

    fn speed(&self) -> f32 {
        self.speed
    }
}

struct Inner {
    state: VideoState,
    clock: Clock,
    info: VideoInfo,
    /// Whether the source keeps adding media as it plays.
    live: bool,
    /// Whether the source can be seeked. A live stream cannot.
    seekable: bool,
    /// Whether audio a backend carries is silenced. A browser starts muted so
    /// playback can begin without a gesture; the app may unmute it later.
    muted: bool,
    /// The volume of audio a backend carries, from 0.0 to 1.0.
    volume: f32,
    /// The layout the decoder produces, for the diagnostics snapshot.
    pixel_format: PixelFormat,
    /// Whether frames reach the screen as the decoder's own NV12 planes.
    native_nv12: bool,
    /// Whether the source repeats when it reaches its end.
    looping: bool,
    /// A section that repeats instead of the whole source, when one is set.
    loop_range: Option<crate::loop_range::LoopRange>,
    /// The audio tracks the source carries and which one is selected.
    audio_tracks: crate::tracks::TrackList,
    /// The subtitle tracks the source carries and which one is selected.
    subtitle_tracks: crate::tracks::TrackList,
    /// How far the decoder has filled its window, for a spinner.
    buffering: crate::buffer::Buffering,
    /// What the backend stated about the container and its tracks.
    media: crate::media::MediaInfo,
    /// Whether a picture has been published in this session. The `Playing`
    /// event is held until the first one exists.
    has_frame: bool,
    /// A `play()` that started the clock before the first picture arrived. Its
    /// `Playing` event is emitted by the decode thread with that picture.
    playing_pending: bool,
    /// The current failure is a transient source error, so a successful retry
    /// may put the player back into `Playing` instead of leaving it terminal.
    recovering: bool,
}

/// The play state and the clock, read by the screen and written by the engine.
pub(crate) struct Shared {
    inner: Mutex<Inner>,
    /// The session's event history and counters, read by the screen and written
    /// by both the screen and the decode thread. Its own lock keeps an emission
    /// from a hot decode path independent of the state lock.
    telemetry: Telemetry,
    /// Set by `VideoPlayer::close` so a decoder blocked in a network read can
    /// return and let the decode thread join.
    shutdown: Shutdown,
    /// The lifecycle and recovery record shared with the decoder and the audio
    /// backends. It is additive: telemetry and the public event history are
    /// unchanged, and a decoder that ignores it behaves as before.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    playback: PlaybackSession,
}

impl Shared {
    fn new(info: VideoInfo, live: bool, seekable: bool, muted: bool, volume: f32) -> Self {
        // A decoder that already knows the picture size can present at once.
        // The web learns it from the browser's metadata a moment later, which
        // is what `Loading` is for.
        let state = if info.width == 0 || info.height == 0 {
            VideoState::Loading
        } else {
            VideoState::Paused
        };
        let shared = Self {
            inner: Mutex::new(Inner {
                state,
                clock: Clock::new(),
                info,
                live,
                seekable,
                muted,
                volume,
                pixel_format: PixelFormat::Rgba8,
                native_nv12: false,
                looping: false,
                loop_range: None,
                audio_tracks: crate::tracks::TrackList::new(crate::tracks::TrackKind::Audio),
                subtitle_tracks: crate::tracks::TrackList::new(crate::tracks::TrackKind::Subtitle),
                buffering: crate::buffer::Buffering::new(),
                media: crate::media::MediaInfo::default(),
                has_frame: false,
                playing_pending: false,
                recovering: false,
            }),
            telemetry: Telemetry::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            playback: PlaybackSession::new(),
        };
        // The first event a session records. The clock starts at zero, so the
        // record needs no state read beyond the one the helper already makes.
        {
            let inner = shared.inner.lock();
            shared.emit(&inner, PlaybackEvent::Opened);
        }
        // A native decoder is already open when `Shared` is built, so the
        // recovery session starts in its active phase. A failed audio setup
        // inside `open` is reported later and moves it into recovery.
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        shared.playback.opened();
        shared
    }

    /// Records an event at the position the current clock reads.
    ///
    /// Takes the state guard as an argument so callers that already hold it can
    /// emit without locking it again (parking_lot locks are not reentrant).
    fn emit(&self, inner: &Inner, event: PlaybackEvent) {
        // Once a close has begun, `Closed` is the last event of the session. A
        // detached decode thread can still be finishing a read, so without this
        // seal it could append `Playing`, `Ended`, or `Failed` afterwards and
        // break the observability contract. `Closed` itself is deliberately
        // allowed through: `close()` sets the flag before emitting it.
        if self.shutdown.load(Ordering::Relaxed) && !matches!(&event, PlaybackEvent::Closed) {
            return;
        }
        let position = inner.clock.position();
        self.telemetry.emit(event, position);
    }

    /// Records an event from a caller that does not already hold the state.
    fn emit_event(&self, event: PlaybackEvent) {
        let inner = self.inner.lock();
        self.emit(&inner, event);
    }

    /// Whether the player is closing, so the decode thread leaves promptly.
    /// The web has no decode thread to check this from.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    /// The flag a blocking decoder read watches for a close.
    pub(crate) fn shutdown_flag(&self) -> Shutdown {
        Arc::clone(&self.shutdown)
    }

    /// Signals a close before detaching from the decode thread.
    pub(crate) fn begin_shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.inner.lock().recovering = false;
    }

    /// A clone of the recovery handle, handed to the decoder and its worker.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub(crate) fn playback_session(&self) -> PlaybackSession {
        self.playback.clone()
    }

    /// The current lifecycle phase of the recovery session.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub(crate) fn playback_state(&self) -> PlaybackState {
        self.playback.state()
    }

    /// The recovery events recorded so far, oldest first.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub(crate) fn playback_events(&self) -> Vec<SessionEvent> {
        self.playback.events()
    }

    /// Whether the visible failure is a transient source error awaiting a retry.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub(crate) fn has_recoverable_failure(&self) -> bool {
        self.inner.lock().recovering
    }

    /// Puts a transiently failed player back into playback after a retry.
    ///
    /// The caller already holds the state guard: `note_frame` invokes this with
    /// the guard it took to publish the picture. `recover()` returning true is
    /// not proof that pictures are flowing again, so this is only called once
    /// the worker has actually published a frame after the retry.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn resume_after_recovery(&self, inner: &mut Inner) {
        if !inner.recovering || !matches!(inner.state, VideoState::Failed(_)) {
            return;
        }
        inner.recovering = false;
        inner.clock.set_running(true);
        inner.state = VideoState::Playing;
        inner.playing_pending = false;
        self.emit(inner, PlaybackEvent::Playing);
    }

    /// A backend rebuild succeeded after a failure. Mark the state as awaiting a
    /// confirming picture so `resume_after_recovery` can clear `Failed` on the
    /// next frame — even when the original error was non-transient (`Decode`),
    /// which is the case for Apple and Android stream failures. The failure has
    /// already armed the session, so this only re-arms the *public* transition.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub(crate) fn recovery_succeeded(&self) {
        let mut inner = self.inner.lock();
        if matches!(inner.state, VideoState::Failed(_)) {
            inner.recovering = true;
        }
    }

    /// Closes the recovery session so `Closed` stays its last event.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub(crate) fn close_playback(&self) {
        self.playback.close();
    }

    pub(crate) fn state(&self) -> VideoState {
        self.inner.lock().state.clone()
    }

    pub(crate) fn info(&self) -> VideoInfo {
        self.inner.lock().info
    }

    pub(crate) fn is_live(&self) -> bool {
        self.inner.lock().live
    }

    pub(crate) fn is_seekable(&self) -> bool {
        self.inner.lock().seekable
    }

    pub(crate) fn is_muted(&self) -> bool {
        self.inner.lock().muted
    }

    pub(crate) fn volume(&self) -> f32 {
        self.inner.lock().volume
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        let mut inner = self.inner.lock();
        inner.muted = muted;
        self.emit(&inner, PlaybackEvent::Muted { muted });
    }

    pub(crate) fn set_volume(&self, volume: f32) {
        let mut inner = self.inner.lock();
        inner.volume = volume;
        self.emit(&inner, PlaybackEvent::VolumeChanged { volume });
    }

    pub(crate) fn set_pixel_format(&self, format: PixelFormat) {
        self.inner.lock().pixel_format = format;
    }

    /// Whether pictures reach the screen in the decoder's own planes.
    pub(crate) fn native_nv12(&self) -> bool {
        self.inner.lock().native_nv12
    }

    pub(crate) fn set_native_nv12(&self, native: bool) {
        self.inner.lock().native_nv12 = native;
    }

    /// Whether the source repeats when it reaches its end.
    pub(crate) fn is_looping(&self) -> bool {
        self.inner.lock().looping
    }

    pub(crate) fn set_looping(&self, looping: bool) {
        self.inner.lock().looping = looping;
    }

    /// The section that repeats, when one is set.
    pub(crate) fn loop_range(&self) -> Option<crate::loop_range::LoopRange> {
        self.inner.lock().loop_range
    }

    /// Sets or clears the repeating section.
    pub(crate) fn set_loop_range(&self, range: Option<crate::loop_range::LoopRange>) {
        self.inner.lock().loop_range = range;
    }

    /// The audio tracks and the current selection.
    pub(crate) fn audio_tracks(&self) -> crate::tracks::TrackList {
        self.inner.lock().audio_tracks.clone()
    }

    /// The subtitle tracks and the current selection.
    pub(crate) fn subtitle_tracks(&self) -> crate::tracks::TrackList {
        self.inner.lock().subtitle_tracks.clone()
    }

    /// Selects an audio track and records the change.
    ///
    /// Returns `false` when the index is not in the list.
    pub(crate) fn select_audio_track(&self, index: Option<usize>) -> bool {
        let mut inner = self.inner.lock();
        if !inner.audio_tracks.select(index) {
            return false;
        }
        self.emit(&inner, PlaybackEvent::TrackChanged {
            kind: crate::tracks::TrackKind::Audio,
            index,
        });
        true
    }

    /// Selects a subtitle track and records the change.
    ///
    /// Returns `false` when the index is not in the list.
    pub(crate) fn select_subtitle_track(&self, index: Option<usize>) -> bool {
        let mut inner = self.inner.lock();
        if !inner.subtitle_tracks.select(index) {
            return false;
        }
        self.emit(&inner, PlaybackEvent::TrackChanged {
            kind: crate::tracks::TrackKind::Subtitle,
            index,
        });
        true
    }

    /// The buffering state a screen reads.
    pub(crate) fn buffering(&self) -> crate::buffer::Buffering {
        self.inner.lock().buffering
    }

    /// Records a buffering change and emits it when it differs.
    pub(crate) fn set_buffering(
        &self,
        state: crate::buffer::BufferingState,
        fraction: f32,
    ) {
        let mut inner = self.inner.lock();
        let before = inner.buffering;
        inner.buffering.set(state, fraction);
        let after = inner.buffering;
        if before.state() != after.state() || before.fraction() != after.fraction() {
            self.emit(&inner, PlaybackEvent::BufferingChanged {
                state: after.state(),
                fraction: after.fraction(),
            });
        }
    }

    /// What the backend stated about the container and its tracks.
    pub(crate) fn media(&self) -> crate::media::MediaInfo {
        self.inner.lock().media.clone()
    }

    /// Records what the backend stated about the source.
    pub(crate) fn set_media(&self, media: crate::media::MediaInfo) {
        let mut inner = self.inner.lock();
        inner.audio_tracks = crate::tracks::TrackList::new(crate::tracks::TrackKind::Audio);
        inner.subtitle_tracks = crate::tracks::TrackList::new(crate::tracks::TrackKind::Subtitle);
        for track in &media.tracks {
            match track.kind {
                crate::tracks::TrackKind::Audio => inner.audio_tracks.push(track.clone()),
                crate::tracks::TrackKind::Subtitle => inner.subtitle_tracks.push(track.clone()),
            }
        }
        inner.audio_tracks.select_default();
        inner.media = media;
    }

    /// Folds one decode's cost into the rolling average the diagnostics read.
    pub(crate) fn note_decode(&self, took: Duration) {
        self.telemetry.note_decode(took);
    }

    /// Folds one NV12 to RGBA conversion into the rolling average.
    pub(crate) fn note_convert(&self, took: Duration) {
        self.telemetry.note_convert(took);
    }

    pub(crate) fn duration(&self) -> Option<Duration> {
        self.inner.lock().info.duration
    }

    pub(crate) fn is_playing(&self) -> bool {
        matches!(self.inner.lock().state, VideoState::Playing)
    }

    pub(crate) fn position(&self) -> Duration {
        let inner = self.inner.lock();
        let position = inner.clock.position();
        match inner.info.duration {
            Some(duration) if position > duration => duration,
            _ => position,
        }
    }

    /// Anchors the clock at a picture's presentation time.
    pub(crate) fn set_position(&self, position: Duration) {
        self.inner.lock().clock.set(position);
    }

    /// Anchors the clock at a picture only when the picture is not behind the
    /// play head. A decoder may hand out a stale frame after a seek or a speed
    /// change; letting it reset the clock makes the play head jump backwards
    /// while audio keeps moving forward.
    pub(crate) fn set_position_monotonic(&self, position: Duration) {
        let mut inner = self.inner.lock();
        if inner.clock.position() <= position {
            inner.clock.set(position);
        }
    }

    /// Starts the clock and says frames are moving.
    ///
    /// The clock starts at once so the decode thread can produce pictures, but
    /// the `Playing` event waits for the first picture when play was called
    /// before one existed. A screen that follows the event never announces a
    /// blank frame as playing.
    pub(crate) fn start(&self) {
        let mut inner = self.inner.lock();
        inner.clock.set_running(true);
        inner.state = VideoState::Playing;
        inner.recovering = false;
        if inner.has_frame {
            self.emit(&inner, PlaybackEvent::Playing);
        } else {
            inner.playing_pending = true;
        }
    }

    /// Stops the clock and says frames are still.
    pub(crate) fn pause(&self) {
        let mut inner = self.inner.lock();
        inner.clock.set_running(false);
        inner.state = VideoState::Paused;
        inner.playing_pending = false;
        inner.recovering = false;
        self.emit(&inner, PlaybackEvent::Paused);
    }

    /// Takes the state a seek leaves behind: a source that ended or failed is
    /// open and positioned again.
    pub(crate) fn after_seek(&self) {
        let mut inner = self.inner.lock();
        if matches!(
            inner.state,
            VideoState::Ended | VideoState::Failed(_) | VideoState::Loading | VideoState::Idle
        ) {
            inner.clock.set_running(false);
            inner.state = VideoState::Paused;
        }
        inner.recovering = false;
        // The caller set the clock to the sought position just before this, so
        // the event records the exact place the player moved to. This also
        // covers stop and restart, which reposition through the same path.
        let position = inner.clock.position();
        self.emit(&inner, PlaybackEvent::Seeking { position });
    }

    /// The source ran out of pictures.
    pub(crate) fn finish(&self) {
        let mut inner = self.inner.lock();
        let position = inner
            .info
            .duration
            .unwrap_or_else(|| inner.clock.position());
        inner.clock.set_running(false);
        inner.clock.set(position);
        inner.state = VideoState::Ended;
        inner.playing_pending = false;
        inner.recovering = false;
        self.emit(&inner, PlaybackEvent::Ended);
    }

    /// Reading stopped on a failure.
    pub(crate) fn fail(&self, error: VideoError) {
        let transient = error.is_transient();
        let reason = error.to_string();
        // The public state and event history outlive the request that produced
        // them; keep a redacted copy so a live URL's token cannot survive there.
        let stored = error.redacted();
        let mut inner = self.inner.lock();
        inner.clock.set_running(false);
        inner.state = VideoState::Failed(stored.clone());
        inner.playing_pending = false;
        inner.recovering = transient;
        self.emit(&inner, PlaybackEvent::Failed { error: stored });
        // A source error can be a lost network window or a device that was
        // briefly unavailable. It must still be visible as a failure, but it
        // is not terminal: arm the shared retry so the worker can rebuild the
        // stream on its own thread and put the player back into playback.
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        if transient {
            self.playback.failed("decoder.read", reason);
        }
    }

    pub(crate) fn set_speed(&self, speed: f32) {
        let mut inner = self.inner.lock();
        inner.clock.set_speed(speed);
        self.emit(&inner, PlaybackEvent::SpeedChanged { speed });
    }

    pub(crate) fn speed(&self) -> f32 {
        self.inner.lock().clock.speed()
    }

    /// Replaces the size and duration a decoder only learned after it opened,
    /// which the browser states a moment after the video element is asked for
    /// the file. A decoder that knew the size keeps it.
    pub(crate) fn adopt_info(&self, info: VideoInfo) {
        let mut inner = self.inner.lock();
        if inner.info.width == 0 || inner.info.height == 0 {
            inner.info = info;
            if matches!(inner.state, VideoState::Loading) {
                inner.state = VideoState::Paused;
            }
            self.emit(
                &inner,
                PlaybackEvent::InfoReady {
                    width: info.width,
                    height: info.height,
                },
            );
        }
    }

    /// Refreshes the live/seekable flags once the decoder has loaded its source.
    ///
    /// HLS learns these only after the playlist is fetched on the decode
    /// thread, but `from_decoder` snapshots them at open, when the placeholder
    /// still says `live=false, seekable=true`. Leaving that snapshot frozen made
    /// the live-empty-window retry branch in the worker dead and let a live
    /// stream end instead of recovering. Called from the first post-open frame,
    /// where the real values are available.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub(crate) fn adopt_capabilities(&self, live: bool, seekable: bool) {
        let mut inner = self.inner.lock();
        // Monotonic: a source that opens as live stays live (a decoder that does
        // not report liveness returns the default false and must not turn a
        // known live feed finite), and seekability can only be revoked once the
        // playlist reveals a live stream — never granted to one that refused it.
        inner.live |= live;
        inner.seekable &= seekable;
    }

    /// Records that the source repeated back to `position`.
    pub(crate) fn emit_looped(&self, position: Duration) {
        self.emit_event(PlaybackEvent::Looped { position });
    }

    /// Records that the player was closed.
    pub(crate) fn emit_closed(&self) {
        self.emit_event(PlaybackEvent::Closed);
    }

    /// Counts a picture the decode thread published to the screen, and
    /// releases a `Playing` event that was waiting for it.
    pub(crate) fn note_frame(&self) {
        let mut inner = self.inner.lock();
        if !inner.has_frame {
            // The first picture of a session is the proof that the decoder
            // delivered a video sample, which is what a field log is asked for.
            crate::diag("INFO", "player.video", "the first picture was published");
        }
        inner.has_frame = true;
        // A transient source failure stays visible until a picture from the
        // rebuilt source is on screen. `recover()` only rebuilt the reader;
        // this frame is the proof that playback really resumed.
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        self.resume_after_recovery(&mut inner);
        if inner.playing_pending {
            inner.playing_pending = false;
            self.emit(&inner, PlaybackEvent::Playing);
        }
        drop(inner);
        // Count the picture last: an observer waiting for the count to rise can
        // then rely on the `Playing` event already being in the history.
        self.telemetry.note_frame();
    }

    /// Records the authoritative wall time and media time of a published
    /// picture. Only the threaded engine has a decode thread to sample.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn note_publication(&self, at: Instant, pts: Duration) {
        self.telemetry.note_publication(at, pts);
    }

    /// The authoritative publication timeline, oldest first.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn publication_timeline(&self) -> Vec<(Instant, Duration)> {
        self.telemetry.publication_timeline()
    }

    /// Counts and records a clock re-anchor to a late picture.
    ///
    /// Only the threaded engine re-anchors its own clock; on the web the browser
    /// owns the clock, so there is nothing to record there.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn note_reanchor(&self, position: Duration) {
        self.emit_event(PlaybackEvent::Reanchored { position });
    }

    pub(crate) fn session(&self) -> SessionId {
        self.telemetry.session()
    }

    pub(crate) fn events(&self) -> Vec<EventRecord> {
        self.telemetry.events()
    }

    /// Builds a point-in-time snapshot from the state and telemetry locks.
    ///
    /// The state lock is taken first, then the telemetry lock — the same order
    /// the `emit` helpers use — so a snapshot cannot deadlock against an event
    /// being recorded on the decode thread.
    pub(crate) fn diagnostics(&self) -> PlaybackDiagnostics {
        let inner = self.inner.lock();
        let TelemetryView {
            session,
            uptime,
            frames,
            reanchors,
            errors,
            seeks,
            plays,
            decode_time,
            convert_time,
            event_count,
        } = self.telemetry.view();
        let position = inner.clock.position();
        let position = match inner.info.duration {
            Some(duration) if position > duration => duration,
            _ => position,
        };
        PlaybackDiagnostics {
            session,
            state: inner.state.clone(),
            position,
            duration: inner.info.duration,
            speed: inner.clock.speed(),
            is_live: inner.live,
            is_seekable: inner.seekable,
            muted: inner.muted,
            volume: inner.volume,
            info: inner.info,
            uptime,
            decode_time,
            convert_time,
            pixel_format: inner.pixel_format,
            frames_published: frames,
            reanchors,
            errors,
            seeks,
            plays,
            event_count,
        }
    }
}

/// The newest picture, and pixel buffers waiting to be filled again.
struct Slot {
    newest: Option<VideoFrame>,
    /// The size of the most recent picture. Kept separately from `newest`
    /// because taking a frame consumes it, but a UI still wants to show the
    /// current resolution between frames — including after an adaptive or
    /// rotated stream changes size mid-playback.
    last_size: Option<(u32, u32)>,
    /// RGBA buffers waiting to be filled again.
    spares: Vec<Vec<u8>>,
    /// NV12 plane buffers waiting to be filled again. A player that hands the
    /// planes to the screen keeps none, because the screen holds them.
    plane_spares: Vec<Vec<u8>>,
}

impl Slot {
    fn new() -> Self {
        Self {
            newest: None,
            last_size: None,
            spares: Vec::new(),
            plane_spares: Vec::new(),
        }
    }

    /// Shows a picture, keeping the replaced one's buffers when there is room.
    fn publish(&mut self, frame: VideoFrame) {
        self.last_size = Some((frame.width(), frame.height()));
        if let Some(previous) = self.newest.replace(frame) {
            self.reclaim(previous);
        }
    }

    /// Puts a picture's buffers back in the pool, in whichever layouts it holds.
    fn reclaim(&mut self, frame: VideoFrame) {
        let (rgba, planes) = frame.into_buffers();
        if let Some(rgba) = rgba
            && rgba.len() <= self.spare_len()
            && self.spares.len() < SPARES
        {
            self.spares.push(rgba);
        }
        if let Some(planes) = planes
            && self.plane_spares.len() < SPARES
        {
            self.plane_spares.push(planes);
        }
    }

    /// The largest buffer the pool needs, so a size change drops the old ones
    /// instead of growing the pool.
    fn spare_len(&self) -> usize {
        self.spares.first().map_or(usize::MAX, Vec::len)
    }

    fn take(&mut self) -> Option<VideoFrame> {
        self.newest.take()
    }

    /// Drops the picture waiting to be shown, which belongs to the position
    /// the player leaves behind.
    fn discard(&mut self) {
        if let Some(previous) = self.newest.take() {
            self.reclaim(previous);
        }
    }

    fn recycle(&mut self, frame: VideoFrame) {
        self.reclaim(frame);
    }

    /// A buffer to decode into, or an empty one when the pool ran dry.
    fn spare(&mut self) -> Vec<u8> {
        self.spares.pop().unwrap_or_default()
    }

    /// A plane buffer to decode into, or an empty one when the pool ran dry.
    fn spare_planes(&mut self) -> Vec<u8> {
        self.plane_spares.pop().unwrap_or_default()
    }

    fn clear(&mut self) {
        self.newest = None;
        self.spares.clear();
        self.plane_spares.clear();
    }
}

/// The presentation time of a picture, kept ahead of the one shown before it.
///
/// A backend that cannot state a time repeats the previous one. Showing both at
/// the same moment would freeze the clock, so the second lands one frame later
/// at the rate the container states.
pub(crate) fn presentation_time(
    previous: Option<Duration>,
    pts: Duration,
    rate: Option<crate::FrameRate>,
) -> Duration {
    match previous {
        Some(last) if pts <= last => {
            let duration = rate
                .and_then(|rate| rate.frame_duration())
                .unwrap_or(DEFAULT_FRAME);
            last.saturating_add(duration)
        }
        _ => pts,
    }
}

/// A video that decodes, plays, pauses and seeks.
///
/// The player opens a source on the calling thread, so a file that cannot be
/// read reports it before a frame is drawn, and decodes on a thread of its own
/// from then on. The screen reads [`state`](Self::state) and
/// [`position`](Self::position) while it draws, takes the newest picture with
/// [`take_frame`](Self::take_frame), and hands it back with
/// [`recycle`](Self::recycle).
///
/// ```
/// # use cranpose_video::{VideoError, VideoPlayer};
/// # fn show(_: &[u8]) {}
/// # fn run() -> Result<(), VideoError> {
/// let player = VideoPlayer::open("clip.mp4")?;
/// player.play()?;
/// if let Some(frame) = player.take_frame() {
///     show(frame.rgba());
///     player.recycle(frame);
/// }
/// # Ok(())
/// # }
/// ```
///
/// A failure reaches the caller twice: the call that fails returns it, and
/// [`state`](Self::state) keeps [`VideoState::Failed`] with a copy, so a screen
/// that only polls the state still shows why playback stopped.
pub struct VideoPlayer {
    shared: Arc<Shared>,
    slot: Arc<Mutex<Slot>>,
    engine: Engine,
    closed: AtomicBool,
}

impl VideoPlayer {
    /// Opens a video with the decoder this build chose for the platform.
    ///
    /// Returns [`VideoError::Unsupported`]on a target the crate carries no
    /// decoder for; [`with_decoder`](Self::with_decoder) still works there.
    ///
    /// # Errors
    ///
    /// Fails when the source cannot be opened, when it holds no video track,
    /// and when the platform decoder refuses it.
    pub fn open(source: impl Into<crate::VideoSource>) -> Result<Self, VideoError> {
        let source = source.into();
        crate::diag("INFO", "player.source", &describe_source(&source));
        let decoder = crate::platform::open(&source)?;
        #[cfg(all(feature = "native-audio", not(target_arch = "wasm32")))]
        let decoder = crate::audio::wrap_direct(decoder, &source);
        Self::from_decoder(decoder)
    }

    /// Opens a source the caller knows is an HLS playlist.
    ///
    /// Use this when the address does not end in `.m3u8` or when a query hides
    /// the playlist path. It skips the suffix check and skips the direct-audio
    /// wrapper, because HLS carries its sound inside the media it joins. The
    /// playlist read runs on the decode thread, so this call does not wait on
    /// the network.
    ///
    /// # Errors
    ///
    /// Fails when the source is not a URL, when the build has no HLS reader,
    /// and when the decode thread cannot start.
    pub fn open_hls(source: impl Into<crate::VideoSource>) -> Result<Self, VideoError> {
        let source = source.into();
        crate::diag("INFO", "player.source", &describe_source(&source));
        #[cfg(any(
            target_os = "windows",
            target_os = "android",
            target_os = "macos",
            target_os = "ios",
            all(target_os = "linux", feature = "linux-gstreamer")
        ))]
        {
            let Some(url) = source.as_url() else {
                return Err(VideoError::UnsupportedOperation {
                    operation: "opening an HLS source that is not a URL",
                });
            };
            // On iOS the original playlist goes straight to AVPlayer: the
            // in-crate HLS demuxer produces a local .ts that AVFoundation
            // cannot open as a standalone file.
            #[cfg(target_os = "ios")]
            return Self::from_decoder(crate::platform::open_hls_direct(url)?);
            #[cfg(not(target_os = "ios"))]
            {
                let _ = url;
                return Self::from_decoder(crate::hls::open(&source)?);
            }
        }
        #[cfg(not(any(
            target_os = "windows",
            target_os = "android",
            target_os = "macos",
            target_os = "ios",
            all(target_os = "linux", feature = "linux-gstreamer")
        )))]
        Self::open(source)
    }

    /// Plays a source an application decoded itself.
    ///
    /// The decoder moves to the player's thread, so it is called off the thread
    /// that draws. It is the way to read a format the platform does not, and
    /// the way to decode on Linux without the `linux-gstreamer` feature.
    ///
    /// # Errors
    ///
    /// Fails when the decode thread cannot start, which a process out of
    /// threads or memory reports.
    pub fn with_decoder<D>(decoder: D) -> Result<Self, VideoError>
    where
        D: VideoDecoder + 'static,
    {
        Self::from_decoder(Box::new(decoder))
    }

    fn from_decoder(mut decoder: Box<dyn VideoDecoder>) -> Result<Self, VideoError> {
        let info = decoder.info();
        let pixel_format = decoder.output_format();
        let live = decoder.is_live();
        let seekable = decoder.is_seekable();
        let muted = decoder.is_muted();
        let volume = decoder.volume();
        crate::diag(
            "INFO",
            "player.open",
            &format!(
                "decoder={} {}x{} live={live} seekable={seekable} format={pixel_format:?} muted={muted} volume={volume}",
                crate::decoder_name(),
                info.width,
                info.height
            ),
        );
        // A backend with a demuxer already knows what the container holds, so
        // the track lists and the about-screen facts are filled before the first
        // picture. A decoder that states nothing leaves the lists empty and the
        // API reports no tracks, which is the truth for a raw elementary stream.
        let media = decoder.media();
        let shared = Arc::new(Shared::new(info, live, seekable, muted, volume));
        shared.set_pixel_format(pixel_format);
        if !media.is_empty() {
            shared.set_media(media);
        }
        // Hand the decoder the close signal before it moves to the decode
        // thread. A network decoder checks it inside a blocking read so closing
        // the player does not wait on a dead request.
        decoder.observe_shutdown(shared.shutdown_flag());
        // Hand the decoder the recovery handle before it moves to the worker.
        // A decoder that owns sound reports failures into it and restores sound
        // from `recover()` on the decode thread, never on the composition one.
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        decoder.attach_session(shared.playback_session());
        let slot = Arc::new(Mutex::new(Slot::new()));
        let engine = Engine::start(decoder, Arc::clone(&shared), Arc::clone(&slot))?;
        Ok(Self {
            shared,
            slot,
            engine,
            closed: AtomicBool::new(false),
        })
    }

    /// The size, duration and rotation the decoder knows.
    ///
    /// A browser learns the from the media's metadata, so on the web the size
    /// reads zero until the first picture arrives and the player updates it;
    /// [`state`](Self::state) is [`VideoState::Loading`] until then.
    pub fn info(&self) -> VideoInfo {
        self.shared.info()
    }

    /// The width and height of the most recent decoded picture, in pixels.
    ///
    /// Unlike [`info`](Self::info), which reports what the decoder learned when
    /// it opened the source, this follows the picture actually being shown. It
    /// therefore updates when an adaptive stream switches rendition, a rotation
    /// changes the display size, or a browser learns the size from the first
    /// frame. Returns `None` until the first picture arrives. Reading it is
    /// cheap: it is a cached value, so it does not consume a frame.
    pub fn decoded_size(&self) -> Option<(u32, u32)> {
        match self.slot.lock().last_size {
            // A browser can state its size from metadata after opening. If a
            // backend still publishes a zero-sized picture, fall back to the
            // size the decoder stated instead of reporting a size no screen can
            // show. Both numbers must be usable before they are returned.
            Some((width, height)) if width > 0 && height > 0 => Some((width, height)),
            _ => {
                let info = self.shared.info();
                if info.width > 0 && info.height > 0 {
                    Some((info.width, info.height))
                } else {
                    None
                }
            }
        }
    }

    /// What the player is doing now.
    pub fn state(&self) -> VideoState {
        self.shared.state()
    }

    /// Whether the clock is currently moving.
    ///
    /// The Media3-style `isPlaying` convenience; derived from `state()`, so it
    /// needs no extra shared state and cannot disagree with `state()`.
    pub fn is_playing(&self) -> bool {
        matches!(self.state(), crate::VideoState::Playing)
    }

    /// Where playback is, between zero and [`duration`](Self::duration).
    pub fn position(&self) -> Duration {
        self.shared.position()
    }

    /// How long the media runs, when the container states it.
    ///
    /// A live stream states no length, so this reads `None` for the whole
    /// stream even while it is playing.
    pub fn duration(&self) -> Option<Duration> {
        self.shared.duration()
    }

    /// Whether the source keeps adding media as it plays.
    ///
    /// A live stream plays and pauses like any other, but
    /// [`seek`](Self::seek), [`seek_fraction`](Self::seek_fraction) and
    /// [`stop`](Self::stop) report [`VideoError::UnsupportedOperation`], and
    /// [`play`](Self::play) cannot restart one that ended.
    pub fn is_live(&self) -> bool {
        self.shared.is_live()
    }

    /// Whether the source can be seeked, which a live stream cannot.
    pub fn is_seekable(&self) -> bool {
        self.shared.is_seekable()
    }

    /// The playback rate, where 1.0 plays at the recorded speed.
    pub fn speed(&self) -> f32 {
        self.shared.speed()
    }

    /// Whether [`close`](Self::close) ran, or the decode thread is gone.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// Starts or resumes playback.
    ///
    /// Playing a source that reached its end starts it again from the first
    /// frame.
    ///
    /// # Errors
    ///
    /// Fails with the failure that stopped an earlier read, and with
    /// [`VideoError::Closed`] after [`close`](Self::close).
    pub fn play(&self) -> Result<(), VideoError> {
        self.check()?;
        let state = self.shared.state();
        if let VideoState::Failed(error) = state {
            return Err(error);
        }
        if state.is_ended() {
            if !self.shared.is_seekable() {
                return Err(VideoError::UnsupportedOperation {
                    operation: "restarting a live stream",
                });
            }
            self.shared.set_position(Duration::ZERO);
            self.shared.after_seek();
            self.slot.lock().discard();
            self.engine.dispatch(Command::Seek(Duration::ZERO))?;
        }
        self.shared.start();
        self.engine.dispatch(Command::Play)
    }

    /// Stops the clock where it is, leaving the picture on screen.
    ///
    /// # Errors
    ///
    /// Fails with [`VideoError::Closed`] after [`close`](Self::close).
    pub fn pause(&self) -> Result<(), VideoError> {
        self.check()?;
        self.shared.pause();
        self.engine.dispatch(Command::Pause)
    }

    /// Pauses a playing source and plays a paused one.
    ///
    /// # Errors
    ///
    /// Fails for the reasons [`play`](Self::play) and [`pause`](Self::pause)
    /// do.
    pub fn toggle(&self) -> Result<(), VideoError> {
        if self.shared.is_playing() {
            self.pause()
        } else {
            self.play()
        }
    }

    /// Returns to the first frame and stops there.
    ///
    /// # Errors
    ///
    /// Fails with [`VideoError::Closed`] after [`close`](Self::close).
    pub fn stop(&self) -> Result<(), VideoError> {
        self.check()?;
        if !self.shared.is_seekable() {
            return Err(VideoError::UnsupportedOperation {
                operation: "stopping a live stream",
            });
        }
        self.shared.pause();
        self.shared.set_position(Duration::ZERO);
        self.shared.after_seek();
        self.slot.lock().discard();
        self.engine.dispatch(Command::Stop)
    }

    /// Moves to a position in the media.
    ///
    /// A position past the end is clamped to the duration. The next picture is
    /// the closest one the platform can decode from, so it lands on the frame
    /// after a key frame rather than exactly where asked.
    ///
    /// The decoder seeks on its own thread, so a platform that refuses the seek
    /// reports it through [`state`](Self::state) rather than here.
    ///
    /// # Errors
    ///
    /// Fails with [`VideoError::Closed`] after [`close`](Self::close).
    pub fn seek(&self, position: Duration) -> Result<(), VideoError> {
        self.check()?;
        if !self.shared.is_seekable() {
            return Err(VideoError::UnsupportedOperation {
                operation: "seeking a live stream",
            });
        }
        let position = match self.shared.duration() {
            Some(duration) => position.min(duration),
            None => position,
        };
        self.shared.set_position(position);
        self.shared.after_seek();
        self.slot.lock().discard();
        self.engine.dispatch(Command::Seek(position))
    }

    /// Moves to a fraction of the media, where 0.5 is the middle.
    ///
    /// A source whose duration the container does not state cannot be seeked by
    /// fraction, and reports [`VideoError::UnsupportedOperation`].
    ///
    /// # Errors
    ///
    /// Fails for the reasons [`seek`](Self::seek) does, and when the duration is
    /// unknown.
    pub fn seek_fraction(&self, fraction: f32) -> Result<(), VideoError> {
        let Some(duration) = self.shared.duration() else {
            return Err(VideoError::UnsupportedOperation {
                operation: "seeking by fraction",
            });
        };
        // `clamp` keeps NaN as NaN; `Duration::mul_f32(NaN)` panics, so a bad
        // UI value (slider or parsed text) must be neutralised first, matching
        // the guards in set_speed/set_volume.
        let fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.seek(duration.mul_f32(fraction))
    }

    /// Plays faster or slower, between 0.0625x and 8x.
    ///
    /// A rate outside the range is clamped to it. The picture rate follows the
    /// clock. Audio plays at the same rate without resampling the samples, so
    /// its pitch stays natural at every speed.
    ///
    /// # Errors
    ///
    /// Fails with [`VideoError::Closed`] after [`close`](Self::close).
    pub fn set_speed(&self, speed: f32) -> Result<(), VideoError> {
        self.check()?;
        let speed = if speed.is_finite() {
            speed.clamp(MIN_SPEED, MAX_SPEED)
        } else {
            1.0
        };
        self.shared.set_speed(speed);
        self.engine.dispatch(Command::SetSpeed(speed))
    }

    /// Silences audio a backend carries itself, and starts it again.
    ///
    /// With `native-audio` enabled, or on Windows where Media Foundation
    /// carries both tracks, this mutes or unmutes the player's own sound. A
    /// browser plays the media in its own element, so this mutes or unmutes
    /// that element. Call this after a person interacts with the page: a
    /// browser may refuse to start sound otherwise, though muting itself is
    /// always allowed. Without an audio backend the state is still recorded so
    /// one control works across every platform.
    ///
    /// # Errors
    ///
    /// Fails with [`VideoError::Closed`] after [`close`](Self::close).
    pub fn set_muted(&self, muted: bool) -> Result<(), VideoError> {
        self.check()?;
        self.shared.set_muted(muted);
        self.engine.dispatch(Command::SetMuted(muted))
    }

    /// Whether audio a backend carries is currently silenced.
    ///
    /// A browser starts an element muted so playback can begin without a
    /// gesture, and this reads `true` until [`set_muted`](Self::set_muted)
    /// unmutes it. A decoder with no audio of its own reads `false`.
    pub fn is_muted(&self) -> bool {
        self.shared.is_muted()
    }

    /// Sets the volume of audio a backend carries itself, from 0.0 to 1.0.
    ///
    /// Values outside the range are clamped, and a non-finite value means 1.0.
    /// Volume and mute are separate: setting volume does not unmute, so a call
    /// to [`set_muted`](Self::set_muted) is still needed to hear sound.
    ///
    /// # Errors
    ///
    /// Fails with [`VideoError::Closed`] after [`close`](Self::close).
    pub fn set_volume(&self, volume: f32) -> Result<(), VideoError> {
        self.check()?;
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.shared.set_volume(volume);
        self.engine.dispatch(Command::SetVolume(volume))
    }

    /// The volume of audio a backend carries, from 0.0 to 1.0.
    pub fn volume(&self) -> f32 {
        self.shared.volume()
    }

    /// Assigns this player's audio group.
    ///
    /// Players in the same group are mutually exclusive: opening a newer one
    /// automatically mutes the older ones, so a feed never plays two clips at
    /// once. Players in different groups keep their sound and can play together.
    /// The default group is 0, which is right for a single-active-player feed.
    /// Call this before or after opening; it takes effect on the decode thread.
    ///
    /// # Errors
    ///
    /// Fails with [`VideoError::Closed`] after [`close`](Self::close).
    pub fn set_audio_group(&self, group: u64) -> Result<(), VideoError> {
        self.check()?;
        self.engine.dispatch(Command::SetAudioGroup(group))
    }

    /// Reports a system audio output route change to the recovery session.
    ///
    /// A host that observes `AVAudioSession.routeChangeNotification` (iOS) or
    /// `AudioDeviceCallback` (Android) calls this when a headset is unplugged or
    /// a Bluetooth device connects. It arms one bounded recovery so the decode
    /// thread re-attaches the audio source — the same effect as a manual
    /// stop/play, which is what a user otherwise has to do to get sound back.
    /// No device query or stream work runs on this (the calling) thread.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub fn note_route_changed(&self) {
        self.shared.playback.route_changed();
    }

    /// The identity of this playback session.
    ///
    /// The value stays the same for the life of the player and is unique among
    /// players in the process. Use it to correlate [`events`](Self::events) and
    /// [`diagnostics`](Self::diagnostics) with a particular source after a
    /// screen recreates its player.
    pub fn session(&self) -> SessionId {
        self.shared.session()
    }

    /// The lifecycle phase of the player's recovery session.
    ///
    /// Unlike [`state`](Self::state), which reports the picture, this reports
    /// the sound: whether it is active, interrupted, recovering, or closed.
    /// A player that keeps showing video while its output device is gone reads
    /// [`PlaybackState::Recovering`] here, so a screen can show that sound is
    /// coming back instead of treating the whole source as failed.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub fn playback_state(&self) -> PlaybackState {
        self.shared.playback_state()
    }

    /// The recovery events this player has recorded, oldest first.
    ///
    /// This is a separate, bounded history from [`events`](Self::events): it
    /// records interruptions, failed audio opens, and recovery attempts and
    /// their outcomes. Reading it does not clear it, and `Closed` is always the
    /// last record.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub fn recovery_events(&self) -> Vec<SessionEvent> {
        self.shared.playback_events()
    }

    /// The events this session has recorded, oldest first.
    ///
    /// The history is bounded to the newest few hundred events. Older records
    /// are dropped, but their [`sequence`](EventRecord::sequence) numbers keep
    /// increasing, so a gap tells a reader how much it missed. Recording never
    /// fails or blocks playback, and reading a snapshot does not clear it.
    pub fn events(&self) -> Vec<EventRecord> {
        self.shared.events()
    }

    /// The authoritative wall time and media time of each published picture,
    /// oldest first. Each pair is `(published_at, presentation_time)`.
    ///
    /// This is meant for pacing probes. A screen normally polls
    /// [`take_frame`](Self::take_frame), but a polled single-picture slot can
    /// miss a picture between two samples — at 60 fps that fabricates a long
    /// gap. This timeline is recorded once per publication on the decode
    /// thread, so it cannot lose a picture. Reading a snapshot does not clear
    /// it; the buffer is a ring that keeps only the newest few thousand stamps.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn publication_timeline(&self) -> Vec<(Instant, Duration)> {
        self.shared.publication_timeline()
    }

    /// A point-in-time snapshot of the session.
    ///
    /// Combines the current [`state`](Self::state), clock and configuration
    /// with the counters telemetry keeps alongside it — pictures published,
    /// clock re-anchors, seeks, errors and uptime. Unlike [`state`](Self::state)
    /// alone, a snapshot can show that a network stream has re-anchored many
    /// times even though it is playing now.
    pub fn diagnostics(&self) -> PlaybackDiagnostics {
        self.shared.diagnostics()
    }

    /// The newest picture, or `None` when none is ready.
    ///
    /// The picture is the newest one decoded. A paused player publishes one
    /// after opening and one after each seek, so a screen has a picture to
    /// show at every position it stops at. The picture leaves the slot when it
    /// is taken, so the screen keeps what it uploaded until the next on
    /// arrives. Hand it back with [`recycle`](Self::recycle) once it is on the
    /// screen; dropping it works too, and only costs the buffer.
    pub fn take_frame(&self) -> Option<VideoFrame> {
        if self.closed.load(Ordering::Relaxed) {
            return None;
        }
        self.engine.pump();
        self.slot.lock().take()
    }

    /// Hands a picture back so its buffer decodes the next frame.
    pub fn recycle(&self, frame: VideoFrame) {
        self.slot.lock().recycle(frame);
    }

    /// Hands the screen the decoder's own NV12 planes instead of converting
    /// them to RGBA first.
    ///
    /// A screen that samples two textures and converts in its shader turns this
    /// on and reads [`VideoFrame::luma`] and [`VideoFrame::chroma`]. Everything
    /// else leaves it off: the decode thread converts once across every core, so
    /// the screen only uploads RGBA. It changes nothing for a decoder that
    /// produces RGBA itself, and nothing for a picture already taken from the
    /// slot.
    pub fn set_native_nv12(&self, native: bool) {
        self.shared.set_native_nv12(native);
    }

    /// Whether pictures are left in the decoder's NV12 planes.
    pub fn native_nv12(&self) -> bool {
        self.shared.native_nv12()
    }

    /// Repeats the source when it reaches its end.
    ///
    /// A looping player rewinds through the decoder it already has, so a
    /// background clip or a feed item repeats with no second open, no device
    /// setup and no allocation. A live stream cannot loop, because it cannot
    /// seek back, and the setting is simply ignored there. The native engines
    /// honour it; on the web the browser plays the media itself, so the element
    /// keeps its own behaviour until the loop is applied there.
    pub fn set_looping(&self, looping: bool) {
        self.shared.set_looping(looping);
    }

    /// Whether the source repeats when it reaches its end.
    pub fn is_looping(&self) -> bool {
        self.shared.is_looping()
    }

    /// Repeats only a section of the source instead of the whole file.
    ///
    /// A language drill, a dance loop or a clip review wants one passage to
    /// repeat; rewinding to the top of the file each time makes that useless.
    /// The range is clamped to the duration, and playback jumps back to its
    /// start when the play head reaches its end. Passing `None` returns to
    /// whole-file looping (or no looping, per [`set_looping`](Self::set_looping)).
    ///
    /// A range implies looping: setting one turns looping on, so the section
    /// repeats without a second call. A live stream cannot seek back, so the
    /// range is ignored there.
    pub fn set_loop_range(&self, range: Option<crate::loop_range::LoopRange>) {
        let range = range.map(|range| range.clamped_to(self.duration()));
        self.shared.set_loop_range(range);
        self.shared.set_looping(range.is_some() || self.shared.is_looping());
        let _ = self.engine.dispatch(Command::SetLoopRange(range));
    }

    /// The section that repeats, when one is set.
    pub fn loop_range(&self) -> Option<crate::loop_range::LoopRange> {
        self.shared.loop_range()
    }

    /// What the backend learned about the container, its codecs and its
    /// bitrate, for an about screen or a quality picker.
    ///
    /// A decoder that states nothing returns the empty value, whose
    /// [`is_empty`](crate::MediaInfo::is_empty) is true.
    pub fn media(&self) -> crate::media::MediaInfo {
        self.shared.media()
    }

    /// The audio tracks the source carries, with the current selection.
    ///
    /// Empty until a backend states them, which most do at open time. The list
    /// is a snapshot: read it again after a change.
    pub fn audio_tracks(&self) -> crate::tracks::TrackList {
        self.shared.audio_tracks()
    }

    /// The subtitle tracks the source carries, with the current selection.
    pub fn subtitle_tracks(&self) -> crate::tracks::TrackList {
        self.shared.subtitle_tracks()
    }

    /// Selects an audio track by its backend index, or turns sound off with
    /// `None`.
    ///
    /// # Errors
    ///
    /// Fails with [`VideoError::Closed`] after [`close`](Self::close), and with
    /// [`VideoError::UnsupportedOperation`] when the index is not one the
    /// source carries.
    pub fn set_audio_track(&self, index: Option<usize>) -> Result<(), VideoError> {
        self.check()?;
        if !self.shared.select_audio_track(index) {
            return Err(VideoError::UnsupportedOperation {
                operation: "that audio track",
            });
        }
        self.engine.dispatch(Command::SelectAudioTrack(index))
    }

    /// Selects a subtitle track by its backend index, or turns subtitles off
    /// with `None`.
    ///
    /// # Errors
    ///
    /// Fails with [`VideoError::Closed`] after [`close`](Self::close), and with
    /// [`VideoError::UnsupportedOperation`] when the index is not one the
    /// source carries.
    pub fn set_subtitle_track(&self, index: Option<usize>) -> Result<(), VideoError> {
        self.check()?;
        if !self.shared.select_subtitle_track(index) {
            return Err(VideoError::UnsupportedOperation {
                operation: "that subtitle track",
            });
        }
        self.engine.dispatch(Command::SelectSubtitleTrack(index))
    }

    /// How far the decoder has filled its window, for a spinner or a ring.
    pub fn buffering(&self) -> crate::buffer::Buffering {
        self.shared.buffering()
    }

    /// What the backend stated about the container, its codecs and its tracks.
    pub fn media_info(&self) -> crate::media::MediaInfo {
        self.shared.media()
    }

    /// Signals the decode thread to stop and releases the player's own state.
    ///
    /// Never waits for a backend stuck inside a blocking read: the worker owns
    /// the decoder and closes it when it eventually returns. Every event after
    /// the shutdown signal is sealed by [`Shared::emit`], so `Closed` stays the
    /// last event of the session even while the worker is still unwinding.
    pub fn close(&mut self) {
        if self.closed.swap(true, Ordering::Relaxed) {
            return;
        }
        // Summarize the session before it is torn down, so the log answers
        // what the session did even when the application never reads a
        // snapshot of its own.
        let snapshot = self.diagnostics();
        crate::diag(
            "INFO",
            "player.close",
            &format!(
                "frames={} errors={} seeks={} reanchors={} position={:?} state={:?}",
                snapshot.frames_published(),
                snapshot.errors(),
                snapshot.seeks(),
                snapshot.reanchors(),
                snapshot.position(),
                snapshot.state()
            ),
        );
        // One-way signal: a cooperative decoder sees it inside a blocking read,
        // and the command sent by `engine.shutdown()` covers the thread that is
        // between reads. Neither can make `close` wait on an uncooperative read.
        self.shared.begin_shutdown();
        self.engine.shutdown();
        self.slot.lock().clear();
        // Close the recovery session before the telemetry history so both
        // records end in `Closed`. A late failure from a detached worker is
        // dropped by `PlaybackSession::failed` once the session is closed.
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        self.shared.close_playback();
        self.shared.emit_closed();
    }

    fn check(&self) -> Result<(), VideoError> {
        if self.closed.load(Ordering::Relaxed) {
            Err(VideoError::Closed)
        } else {
            Ok(())
        }
    }
}

/// What a source is, in one line, for the session log.
fn describe_source(source: &crate::VideoSource) -> String {
    match source {
        crate::VideoSource::Path(path) => format!("opening file {}", path.display()),
        crate::VideoSource::Url(url) => format!("opening url {url}"),
        crate::VideoSource::Bytes(bytes) => format!("opening {} bytes", bytes.len()),
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.close();
    }
}

impl fmt::Debug for VideoPlayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoPlayer")
            .field("state", &self.state())
            .field("info", &self.info())
            .field("position", &self.position())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Condvar, Mutex as StdMutex,
            atomic::{AtomicBool, AtomicU32, Ordering},
            mpsc,
        },
        thread,
        time::Instant,
    };

    use super::*;
    use crate::DecodedFrame;

    /// A decoder that only implements what the trait requires, so the audio
    /// methods come from their defaults.
    struct BareDecoder;

    impl VideoDecoder for BareDecoder {
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

        fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            Ok(None)
        }
    }

    /// A decoder that carries audio state and records the calls it receives.
    /// The record lives behind atomics because the player calls it on its own
    /// thread.
    struct AudioDecoder {
        muted: bool,
        volume: f32,
        seen_muted: Arc<AtomicBool>,
        seen_volume: Arc<AtomicU32>,
    }

    impl AudioDecoder {
        fn new(muted: bool, volume: f32) -> (Self, Arc<AtomicBool>, Arc<AtomicU32>) {
            let seen_muted = Arc::new(AtomicBool::new(false));
            let seen_volume = Arc::new(AtomicU32::new(f32::to_bits(1.0)));
            let decoder = Self {
                muted,
                volume,
                seen_muted: Arc::clone(&seen_muted),
                seen_volume: Arc::clone(&seen_volume),
            };
            (decoder, seen_muted, seen_volume)
        }
    }

    impl VideoDecoder for AudioDecoder {
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

        fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            Ok(None)
        }

        fn set_muted(&mut self, muted: bool) {
            self.muted = muted;
            self.seen_muted.store(muted, Ordering::SeqCst);
        }

        fn set_volume(&mut self, volume: f32) {
            self.volume = volume;
            self.seen_volume.store(volume.to_bits(), Ordering::SeqCst);
        }

        fn is_muted(&self) -> bool {
            self.muted
        }

        fn volume(&self) -> f32 {
            self.volume
        }
    }

    /// A decoder held inside a blocking read that never observes shutdown,
    /// modelling a platform reader stalled by a driver.
    struct StuckDecoder {
        gate: Arc<(StdMutex<bool>, Condvar)>,
        entered: Arc<AtomicBool>,
    }

    impl VideoDecoder for StuckDecoder {
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

        fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            self.entered.store(true, Ordering::SeqCst);
            let (lock, cvar) = &*self.gate;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = cvar.wait(released).unwrap();
            }
            Ok(None)
        }
    }

    #[test]
    fn close_does_not_wait_on_a_decoder_stuck_in_read() {
        let gate = Arc::new((StdMutex::new(false), Condvar::new()));
        let entered = Arc::new(AtomicBool::new(false));
        let decoder = StuckDecoder {
            gate: Arc::clone(&gate),
            entered: Arc::clone(&entered),
        };
        let mut player = VideoPlayer::with_decoder(decoder).expect("the player opens");

        let deadline = Instant::now() + Duration::from_secs(5);
        while !entered.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            entered.load(Ordering::SeqCst),
            "the decode thread never reached read"
        );

        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            player.close();
            let _ = done_tx.send(());
        });
        let closed = done_rx.recv_timeout(Duration::from_millis(800)).is_ok();

        // Free the worker so it can tear down and the process stays clean.
        {
            let (lock, cvar) = &*gate;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
        assert!(closed, "close blocked on a decode thread stuck in read");
    }

    #[test]
    fn a_decoder_without_audio_is_unmuted_and_loud() {
        let mut decoder = BareDecoder;
        assert!(!decoder.is_muted());
        assert_eq!(decoder.volume(), 1.0);

        // The default setters are no-ops: a decoder with no audio of its own
        // still reports the same state afterwards.
        decoder.set_muted(true);
        decoder.set_volume(0.25);
        assert!(!decoder.is_muted());
        assert_eq!(decoder.volume(), 1.0);
    }

   #[test]
    fn a_player_takes_its_initial_audio_state_from_the_decoder() {
        let (decoder, _, _) = AudioDecoder::new(true, 0.4);
        let player = VideoPlayer::with_decoder(decoder).expect("the player opens");
        assert!(player.is_muted());
        assert_eq!(player.volume(), 0.4);
    }

    #[test]
    fn a_player_updates_and_clamps_audio_state() {
        let (decoder, _, _) = AudioDecoder::new(false, 1.0);
        let player = VideoPlayer::with_decoder(decoder).expect("the player opens");

        player.set_muted(true).expect("muting works");
        assert!(player.is_muted());
        player.set_muted(false).expect("unmuting works");
        assert!(!player.is_muted());

        player.set_volume(2.0).expect("a loud volume works");
        assert_eq!(player.volume(), 1.0);
        player.set_volume(-1.0).expect("a quiet volume works");
        assert_eq!(player.volume(), 0.0);
        player.set_volume(0.35).expect("a middle volume works");
        assert_eq!(player.volume(), 0.35);
        player
            .set_volume(f32::NAN)
            .expect("a non-finite volume works");
        assert_eq!(player.volume(), 1.0);
        player
            .set_volume(f32::INFINITY)
            .expect("an infinite volume works");
        assert_eq!(player.volume(), 1.0);
    }

    #[test]
    fn audio_commands_reach_the_decoder() {
        let (decoder, seen_muted, seen_volume) = AudioDecoder::new(false, 1.0);
        let player = VideoPlayer::with_decoder(decoder).expect("the player opens");
        player.set_muted(true).expect("muting works");
        player.set_volume(0.25).expect("a volume works");

        // The decoder lives on the player's thread, so the calls arrive a
        // little after the setters return. Even a decoder with no pictures to
        // read must keep handling commands.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !seen_muted.load(Ordering::SeqCst)
            || f32::from_bits(seen_volume.load(Ordering::SeqCst)) != 0.25
        {
            assert!(Instant::now() < deadline, "audio commands never arrived");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_published_frame_never_rewinds_the_clock() {
        let shared = Shared::new(VideoInfo::default(), false, true, false, 1.0);
        shared.set_position(Duration::from_secs(2));
        shared.set_position_monotonic(Duration::from_millis(500));
        assert!(
            shared.position() >= Duration::from_secs(2),
            "a stale frame cannot move the play head back"
        );
    }

    /// Captures the recovery handle the player hands to its decoder.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    struct SessionDecoder {
        captured: Arc<StdMutex<Option<crate::PlaybackSession>>>,
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    impl VideoDecoder for SessionDecoder {
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

        fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            Ok(None)
        }

        fn attach_session(&mut self, session: crate::PlaybackSession) {
            *self.captured.lock().unwrap() = Some(session);
        }
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    #[test]
    fn a_player_exposes_a_lifecycle_that_closes_last() {
        let captured = Arc::new(StdMutex::new(None));
        let mut player = VideoPlayer::with_decoder(SessionDecoder {
            captured: Arc::clone(&captured),
        })
        .expect("the player opens");

        assert_eq!(player.playback_state(), PlaybackState::Active);
        assert_eq!(
            player.recovery_events(),
            vec![SessionEvent::Opened],
            "opening a player records one lifecycle event"
        );

        // A decoder reports a lost device. The picture remains untouched; only
        // the recovery lifecycle moves into its retrying phase.
        let session = captured
            .lock()
            .unwrap()
            .clone()
            .expect("the decoder received the session");
        session.failed("test.device", "output went away");
        assert_eq!(player.playback_state(), PlaybackState::Recovering);
        assert!(
            player.recovery_events().iter().any(|event| matches!(
                event,
                SessionEvent::Failed {
                    stage: "test.device",
                    ..
                }
            )),
            "the failure is visible in the recovery history"
        );

        player.close();
        assert_eq!(player.playback_state(), PlaybackState::Closed);
        assert_eq!(player.recovery_events().last(), Some(&SessionEvent::Closed));

        // A detached worker's late failure must not be appended after `Closed`.
       session.failed("test.device", "late failure after close");
        assert_eq!(
            player.recovery_events().last(),
            Some(&SessionEvent::Closed),
            "Closed stays last even after a late decoder failure"
        );
    }
}
