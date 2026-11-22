//! Native audio, played with the same in-process backend the rest of the app
//! uses.
//!
//! The picture decoder and this module read the same bytes: a plain source's
//! file, or the temporary file a window of HLS fragments is joined into. That
//! keeps sound on the picture's timeline without a separate clock to reconcile.
//!
//! Sound is strictly best effort. Opening an audio device can fail on a machine
//! with no output, and a container can carry no audio track at all. Neither may
//! stop the picture, so every call here swallows the backend's errors and keeps
//! the state the player reports.

use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
use crate::PlaybackSession;
use cranpose_media::SoftwareMediaPlayer;
use cranpose_services::media::{MediaItem, MediaPlayer};

// Only desktop targets that render generic PCM through cpal can ask which
// output is currently the system default. Android uses AAudio and iOS uses
// AVAudioEngine; neither exposes a portable default-output query here.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
use cpal::{
    SampleFormat,
    traits::{DeviceTrait, HostTrait},
};

use crate::{DecodedFrame, FrameRate, PixelFormat, VideoDecoder, VideoError, VideoInfo, YuvColor};

/// Past this gap between the picture's clock and our own estimate of the sound's
/// play head, the picture has re-anchored (a late frame, a stall recovery, a
/// live rollover) and the sound must jump with it. Smaller differences are the
/// normal run-out of two independent clocks: seeking on every frame would click
/// and stutter, which is worse than a little drift.
const RESYNC_TOLERANCE: Duration = Duration::from_millis(250);

/// The shortest gap between two sound seeks. The media backend clamps speed to
/// a narrower range than the picture, so at extreme speeds the two heads drift
/// steadily apart; without a cooldown that would become a seek every few frames.
/// One seek per interval bounds the damage until the speed returns to normal.
const RESYNC_COOLDOWN: Duration = Duration::from_secs(2);

/// How often the decode thread may ask cpal which output is the system default.
/// Reopening an output can block, so this is a slow poll, not a per-frame query.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
const ROUTE_POLL_COOLDOWN: Duration = Duration::from_millis(500);

/// How long a failed prepare is left alone before another attempt.
///
/// This is a retry throttle, not a shutdown deadline: the default output device
/// can be briefly unavailable while a player that owns an older stream is still
/// tearing down after a source switch. Retrying later recovers sound that would
/// otherwise be silently lost for the life of the player. It also keeps a source
/// with no audio track from being reopened on every decoded frame.
const ATTACH_RETRY_COOLDOWN: Duration = Duration::from_millis(500);

/// The output a source was prepared against, and the PCM layout it was given.
///
/// Comparing the device alone is not enough: a replacement can report the same
/// identity but a different layout, and a different device can offer the same
/// layout. Only the latter may be rebound without converting the samples.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
#[derive(Clone, PartialEq, Eq)]
struct OutputRoute {
    device: cpal::DeviceId,
    channels: u16,
    sample_rate: u32,
    sample_format: SampleFormat,
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
impl OutputRoute {
    fn format(&self) -> (u16, u32, SampleFormat) {
        (self.channels, self.sample_rate, self.sample_format)
    }
}

/// Reads the current default output without ever failing the caller.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn current_output_route() -> Option<OutputRoute> {
    let device = cpal::default_host().default_output_device()?;
    let config = device.default_output_config().ok()?;
    Some(OutputRoute {
        device: device.id().ok()?,
        channels: config.channels(),
        sample_rate: config.sample_rate(),
        sample_format: config.sample_format(),
    })
}

/// Whether a replacement can be used as-is.
///
/// The tuple is `(channels, rate, sample_format)`, the same exact-format rule
/// the Windows Media Foundation path applies. A mismatch would need a converter
/// that resamples the sound and changes its pitch, so it is refused rather than
/// bending the audio to fit.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn rebindable(current: (u16, u32, SampleFormat), observed: (u16, u32, SampleFormat)) -> bool {
    current == observed
}

/// The address a file is opened by, in the form the media backend takes.
pub(crate) fn uri_for_path(path: &Path) -> String {
    cranpose_media::uri_for_path(path)
}

/// A best-effort audio player tied to one source at a time.
///
/// Failures leave `ready` false and are otherwise ignored, so a decoder that
/// carries no audio behaves exactly like one without this module.
pub(crate) struct Audio {
    player: SoftwareMediaPlayer,
    /// Audio group for mutual exclusion. Group 0 (default) means newest wins.
    group: u64,
    /// Our claim token in the group; 0 until the first audible use.
    owner: u64,
    /// Whether arbitration currently suppresses this player's sound.
    suppressed: bool,
    /// The source currently attached, ready or not.
    source: Option<String>,
    /// The last source a prepare was attempted for, so a source that has no
    /// audio track is not reopened on every frame.
    attempted: Option<String>,
    ready: bool,
    /// When a failed prepare for `attempted` may be retried. A device that was
    /// busy at switch time can be available a moment later (see
    /// [`ATTACH_RETRY_COOLDOWN`]).
    retry_at: Option<Instant>,
    playing: bool,
    muted: bool,
    volume: f32,
    speed: f32,
    /// The media position the sound's play head was last anchored at. The media
    /// backend exposes no position query, so this mirrors its clock from wall
    /// time and the speed it actually runs at (see [`Audio::head`]).
    head: Duration,
    /// The wall instant `head` was anchored at.
    head_at: Instant,
    /// Wall instant of the last re-anchor seek, for [`RESYNC_COOLDOWN`].
    last_seek_at: Option<Instant>,
    /// The output the backend selected when this source was prepared.
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    route: Option<OutputRoute>,
    /// Wall instant of the last default-output query, for [`ROUTE_POLL_COOLDOWN`].
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    last_route_check: Option<Instant>,
    /// The player's recovery record, when this build has one.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    session: Option<PlaybackSession>,
    /// Last observed audio-route generation. A change means the output device
    /// was swapped (headphones unplugged, Bluetooth connected); on iOS this is
    /// how the generic backend re-binds without freezing the picture. Only iOS
    /// reads it today, so other targets see an allowed dead field.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    #[allow(dead_code)]
    route_generation: u64,
    /// Whether this source's first failed prepare was already reported. A
    /// container with no audio track fails every time; reporting each failure
    /// would keep a silent picture in recovery forever. One report arms the
    /// worker, and a failed recovery re-arms it with backoff.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    session_failed_reported: bool,
}

impl Audio {
    pub(crate) fn new() -> Self {
        Self {
            player: SoftwareMediaPlayer::new(),
            group: crate::arbitration::DEFAULT_GROUP,
            owner: 0,
            suppressed: false,
            source: None,
            attempted: None,
            ready: false,
            retry_at: None,
            playing: false,
            muted: false,
            volume: 1.0,
            speed: 1.0,
            head: Duration::ZERO,
            head_at: Instant::now(),
            last_seek_at: None,
            #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
            route: None,
            #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
            last_route_check: None,
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            session: None,
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            route_generation: crate::session::route_change_generation(),
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            session_failed_reported: false,
        }
    }

    /// Hands this wrapper the player's recovery record.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub(crate) fn set_session(&mut self, session: PlaybackSession) {
        self.session = Some(session);
    }

    /// The sound's estimated current media position.
    ///
    /// The backend has no position query, so this banks the time elapsed since
    /// the last anchor at the speed the backend actually runs at. It clamps to
    /// the backend's own speed range so the estimate follows the sound the
    /// listener hears rather than a speed the backend refused to honour.
    fn head(&self) -> Duration {
        if !self.playing || !self.speed.is_finite() || self.speed <= 0.0 {
            return self.head;
        }
        let speed = f64::from(self.speed).clamp(0.25, 4.0);
        self.head
            .saturating_add(self.head_at.elapsed().mul_f64(speed))
    }

    /// Banks any elapsed time into `head` and starts a fresh anchor at `now`.
    /// Called whenever the clock's rate or position changes, so no wall time is
    /// counted at the wrong speed.
    fn reanchor(&mut self, position: Duration) {
        self.head = position;
        self.head_at = Instant::now();
    }

    /// Whether `source` is the one already tried, ready or not.
    pub(crate) fn is_source(&self, source: &str) -> bool {
        self.attempted.as_deref() == Some(source)
    }

    /// Whether the current source has a prepared output device.
    pub(crate) fn is_ready(&self) -> bool {
        self.ready
    }

    /// Opens `source` for audio without starting it, unless playback is already
    /// under way. Replaces any source held before.
    pub(crate) fn attach(&mut self, source: &str) {
        // A genuinely new source gets a fresh failure report. `retry()` reuses
        // the same source, so its suppression survives the internal detach.
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        if !self.is_source(source) {
            self.session_failed_reported = false;
        }
        if self.is_source(source) {
            // A source that prepared needs nothing more. One that failed is
            // retried only after its cooldown: the device can be briefly held
            // by a retiring player after a switch, but a source with no audio
            // track would otherwise be reopened on every decoded frame.
            let cooling = self.retry_at.is_some_and(|at| Instant::now() < at);
            if self.ready || cooling {
                return;
            }
        }
        // Keep a seek asked for before the source was open. On an HLS rollover
        // the caller has already detached, so this is zero and the new window
        // starts at its beginning as it should.
        let start = self.head;
        self.detach();
        self.attempted = Some(source.to_owned());
        self.source = Some(source.to_owned());
        self.head = start;
        match self.prepare(source) {
            None => {
                self.ready = true;
                self.retry_at = None;
                #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
                {
                    self.session_failed_reported = false;
                }
                // Record the device the backend actually picked now, so a later
                // default-output change has something to compare against.
                #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
                {
                    self.route = current_output_route();
                }
                self.player.set_volume(self.audible_volume());
                self.player.set_speed(self.speed);
                crate::diag(
                    "INFO",
                    "audio.attach",
                    &format!("sound attached for {source}, starting at {start:?}"),
                );
                if start > Duration::ZERO {
                    let _ = self.player.seek_to(start);
                }
                self.reanchor(start);
                if self.playing {
                    let _ = self.player.play();
                }
            }
            Some(reason) => {
                self.ready = false;
                self.retry_at = Some(Instant::now() + ATTACH_RETRY_COOLDOWN);
                // Arm the worker's recovery exactly once per source. Later
                // failures are re-armed by the worker itself when a scheduled
                // recovery does not restore the device.
                #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
                if !self.session_failed_reported {
                    if let Some(session) = &self.session {
                        session.failed("audio.prepare", reason);
                    }
                    self.session_failed_reported = true;
                }
                #[cfg(not(all(not(target_os = "linux"), not(target_arch = "wasm32"))))]
                let _ = reason;
            }
        }
    }

    /// Forces one reattach attempt, bypassing the per-frame throttle, so the
    /// worker's scheduled recovery is the timing authority. Returns whether
    /// sound is usable again. A source that was never attached needs no work.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    pub(crate) fn retry(&mut self) -> bool {
        let Some(source) = self.source.clone() else {
            return true;
        };
        self.retry_at = None;
        self.attach(&source);
        self.is_ready()
    }

    /// `None` on success, the human-readable cause on failure. The callers log
    /// it and, on native builds, feed it into the recovery session.
    fn prepare(&self, source: &str) -> Option<String> {
        match self.player.prepare(&MediaItem::new(source.to_owned())) {
            Ok(()) => None,
            Err(error) => {
                // The failure is deliberately swallowed so the picture keeps
                // running, but a field log needs the exact reason: a busy
                // output device and a source with no audio track look identical
                // to the player.
                let reason = format!("prepare failed for {source}: {error}");
                crate::diag("WARN", "audio.prepare", &reason);
                Some(reason)
            }
        }
    }

    /// Releases the source and its output device, keeping the volume, mute,
    /// speed and playing intent for the source that replaces it.
    pub(crate) fn detach(&mut self) {
        if self.ready {
            self.player.stop();
        }
        // Give up the audio-group claim so a surviving player in this group can
        // be promoted instead of staying muted after this one closes.
        if self.owner != 0 {
            crate::arbitration::release(self.group, self.owner);
            self.owner = 0;
            self.suppressed = false;
        }
        self.ready = false;
        self.retry_at = None;
        self.source = None;
        self.attempted = None;
        self.head = Duration::ZERO;
        self.head_at = Instant::now();
        self.last_seek_at = None;
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            self.route = None;
            self.last_route_check = None;
        }
    }

    /// Follows a change of the system default output by reopening the same
    /// source on the new device.
    ///
    /// This runs on the decode thread, never on the audio callback, and every
    /// failure is swallowed: a device that is still coming up simply leaves the
    /// old stream in place and is tried again after a short cooldown. The
    /// replacement is refused outright unless its PCM layout matches exactly,
    /// so no resampler or converter is ever exercised to bridge a change.
    pub(crate) fn observe_output_route(&mut self) {
        #[cfg(target_os = "ios")]
        {
            // iOS offers no portable default-output query here, but a route
            // change (headphones unplugged / Bluetooth) invalidates the generic
            // audio stream. AVAudioSession re-routes AVPlayer itself, but our
            // SoftwareMediaPlayer needs an explicit re-attach. Re-bind only the
            // sound — never touch the session phase or the picture — so a route
            // swap cannot freeze the video.
            let generation = crate::session::route_change_generation();
            if generation != self.route_generation {
                self.route_generation = generation;
                if let Some(source) = self.source.clone() {
                    let head = self.head();
                    let last_seek_at = self.last_seek_at;
                    self.detach();
                    self.head = head;
                    self.head_at = Instant::now();
                    self.last_seek_at = last_seek_at;
                    self.attach(&source);
                }
            }
        }
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "linux",
            target_os = "ios"
        )))]
        {
            // Android's AAudio backend and the browser own their own routing;
            // neither offers a portable default-output query here.
            let _ = &self;
        }
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            // Without a prepared source no device was bound, so there is
            // nothing to follow and nothing to disturb.
            let (Some(source), Some(bound)) = (self.source.clone(), self.route.clone()) else {
                return;
            };
            let now = Instant::now();
            if self
                .last_route_check
                .is_some_and(|at| now.duration_since(at) < ROUTE_POLL_COOLDOWN)
            {
                return;
            }
            self.last_route_check = Some(now);

            let Some(observed) = current_output_route() else {
                return;
            };
            if observed.device == bound.device {
                return;
            }
            if !rebindable(bound.format(), observed.format()) {
                // A converter would have to resample the samples, changing
                // pitch. Leave the existing stream alone, as the Windows path
                // does, rather than bending the audio to fit the new device.
                return;
            }

            // Bank elapsed time before the reopen, then hand it to `attach` as
            // the new source's start. The seek cooldown belongs to the picture,
            // not to the device, so it survives the reopen unchanged.
            let head = self.head();
            let last_seek_at = self.last_seek_at;
            self.detach();
            self.head = head;
            self.head_at = Instant::now();
            self.attach(&source);
            self.last_seek_at = last_seek_at;
            self.last_route_check = Some(now);
            // `attach` re-queries cpal; on a failed reopen keep the old route so
            // a device that is still appearing is retried after the cooldown.
            self.route = Some(if self.ready { observed } else { bound });
        }
    }

    pub(crate) fn play(&mut self) {
        if !self.playing {
            // Bank the time since the last anchor before the clock runs again;
            // a resume must not count paused wall time as sound played.
            let head = self.head();
            self.reanchor(head);
            self.playing = true;
        }
        // Claim ownership at first audible use, not at construction, so
        // preloading the next clip does not mute the current one.
        self.claim_if_needed();
        if self.ready && !self.suppressed {
            let _ = self.player.play();
        }
    }

    pub(crate) fn pause(&mut self) {
        if self.playing {
            // Freeze the estimate before the clock stops.
            self.head = self.head();
            self.head_at = Instant::now();
            self.playing = false;
        }
        if self.ready {
            self.player.pause();
        }
    }

    pub(crate) fn seek(&mut self, position: Duration) {
        self.reanchor(position);
        self.last_seek_at = None;
        if self.ready {
            let _ = self.player.seek_to(position);
        }
    }

    pub(crate) fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if self.ready {
            self.player.set_volume(self.audible_volume());
        }
    }

    pub(crate) fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if self.ready {
            self.player.set_volume(self.audible_volume());
        }
    }

    pub(crate) fn set_speed(&mut self, speed: f32) {
        // Bank time elapsed at the old speed before the rate changes, or that
        // span would be counted again at the new speed.
        let head = self.head();
        self.reanchor(head);
        self.speed = speed;
        if self.ready {
            self.player.set_speed(speed);
        }
    }

    /// Whether the gap between the sound's head and a picture time is a real
    /// jump rather than ordinary run-out between two clocks.
    fn needs_resync(head: Duration, position: Duration) -> bool {
        head.abs_diff(position) > RESYNC_TOLERANCE
    }

    /// Re-anchors the sound to a picture time after the player's clock jumps.
    ///
    /// The player calls this for every published frame. A frame close to our own
    /// estimate does nothing; only a genuine gap — a late frame the clock
    /// re-anchored to, a stall recovery, a live rollover — moves the sound, so
    /// ordinary run-out between two clocks never produces an audible seek.
    pub(crate) fn sync_clock(&mut self, position: Duration) {
        // Re-check group ownership every frame so a newer player in this group
        // mutes us promptly. This runs before the `!ready` guard because a
        // suppressed player may still be preparing.
        self.arbitrate();
        if !self.ready {
            return;
        }
        if !Self::needs_resync(self.head(), position) {
            return;
        }
        if self
            .last_seek_at
            .is_some_and(|at| at.elapsed() < RESYNC_COOLDOWN)
        {
            return;
        }
        let _ = self.player.seek_to(position);
        self.reanchor(position);
        self.last_seek_at = Some(Instant::now());
    }

    pub(crate) fn is_muted(&self) -> bool {
        self.muted
    }

    pub(crate) fn volume(&self) -> f32 {
        self.volume
    }

    fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }

    /// Volume honoring audio-group suppression, so a re-attach, mute or volume
    /// change cannot silently unmute a player whose group is owned by another.
    fn audible_volume(&self) -> f32 {
        if self.suppressed {
            0.0
        } else {
            self.effective_volume()
        }
    }

    /// Applies audio-group arbitration. Call on first audible use and per frame.
    /// Claims ownership on first sound; mutes (volume 0, never stop) when a newer
    /// player in the same group wins, and restores when this player is current
    /// again. Does not touch `playing`/clock bookkeeping.
    fn arbitrate(&mut self) {
        // Do NOT claim here: sync_clock calls this on every published frame,
        // including frames published while paused, so a non-audible player must
        // never steal a group from the one actually playing. The claim happens
        // only in `play()` via `claim_if_needed`.
        if self.owner == 0 {
            return;
        }
        let current = crate::arbitration::is_current(self.group, self.owner);
        // `suppressed` is true exactly when we are NOT current. No transition is
        // needed when current == !suppressed (the steady state). The previous
        // `current == suppressed` check was inverted and never muted a stale
        // player, so two same-group clips both kept sounding.
        if current == !self.suppressed {
            return;
        }
        self.suppressed = !current;
        if self.ready {
            self.player.set_volume(self.audible_volume());
        }
    }

    /// Claims this group's audio token on first audible use, then applies the
    /// current suppression state. Called only from `play()`.
    fn claim_if_needed(&mut self) {
        if self.owner == 0 {
            self.owner = crate::arbitration::claim(self.group);
        }
        self.arbitrate();
    }

    /// Moves this audio to a group, claiming a fresh token there. Releasing the
    /// old claim avoids leaking a group entry; suppression is derived from the
    /// new claim rather than assumed false.
    pub(crate) fn set_group(&mut self, group: u64) {
        // Always release the previous claim before claiming a new one, even
        // when the group is unchanged. Otherwise repeated reassignment stacks
        // orphaned tokens in the process-global registry (unbounded growth).
        if self.owner != 0 {
            crate::arbitration::release(self.group, self.owner);
        }
        self.group = group;
        self.owner = crate::arbitration::claim(group);
        self.suppressed = !crate::arbitration::is_current(group, self.owner);
        if self.ready {
            self.player.set_volume(self.audible_volume());
        }
    }
}

/// Whether the paired decoder must try to reopen sound on this read.
///
/// `attached` alone is a one-way latch and leaves a stream that failed a route
/// rebind stranded: the wrapper believes it had attached, while `Audio::ready`
/// is false because the replacement device never came up.
fn needs_attach(attached: bool, ready: bool) -> bool {
    !attached || !ready
}

/// Wraps a decoder for a source that is one file or one address so its audio
/// plays alongside the picture.
///
/// HLS already carries its own audio inside the joined window and bytes have no
/// address the audio backend can open, so both are left alone. On a source that
/// has no audio track the wrapper is still harmless: every audio call becomes
/// a no-op.
pub(crate) fn wrap_direct(
    inner: Box<dyn VideoDecoder>,
    source: &crate::VideoSource,
) -> Box<dyn VideoDecoder> {
    // A decoder that reads both streams from one source on one clock already
    // plays sound with the picture. Pairing another player here would play the
    // same audio twice, so step aside.
    if inner.carries_audio() || source.is_hls() || source.as_bytes().is_some() {
        return inner;
    }
    let uri = match source {
        crate::VideoSource::Path(path) => uri_for_path(path),
        crate::VideoSource::Url(url) => url.to_string(),
        crate::VideoSource::Bytes(_) => return inner,
    };
    Box::new(DirectAudioDecoder::new(inner, uri))
}

/// Drives the audio of a source that is one file, such as a local clip or a
/// direct media URL. HLS carries its own audio inside the stream, so it does
/// not use this wrapper.
pub(crate) struct DirectAudioDecoder {
    inner: Box<dyn VideoDecoder>,
    audio: Audio,
    source: String,
    attached: bool,
}

impl DirectAudioDecoder {
    pub(crate) fn new(inner: Box<dyn VideoDecoder>, source: String) -> Self {
        Self {
            inner,
            audio: Audio::new(),
            source,
            attached: false,
        }
    }

    /// Attaches on the decode thread, on the first read, so opening a player
    /// never waits on an audio device.
    fn ensure_attached(&mut self) {
        // `attached` records that an attempt was made, not that sound is alive.
        // A failed route rebind leaves it true with `ready` false; without the
        // second condition a later read could never retry when the device came
        // back and the route comparison saw no further change.
        if needs_attach(self.attached, self.audio.is_ready()) {
            self.audio.attach(&self.source);
            // Stay unattached when the prepare failed. A default output device
            // can be briefly busy while an older player is still tearing down
            // after a switch; `Audio::attach` throttles the retries so a later
            // read can recover the sound instead of losing it for this player.
            self.attached = self.audio.is_ready();
        }
        self.audio.observe_output_route();
    }
}

impl VideoDecoder for DirectAudioDecoder {
    fn info(&self) -> VideoInfo {
        self.inner.info()
    }

    fn frame_rate(&self) -> Option<FrameRate> {
        self.inner.frame_rate()
    }

    fn has_frame(&self) -> bool {
        self.inner.has_frame()
    }

    fn ready(&self) -> bool {
        self.inner.ready()
    }

    fn is_live(&self) -> bool {
        self.inner.is_live()
    }

    fn is_seekable(&self) -> bool {
        self.inner.is_seekable()
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        self.inner.seek(position)?;
        self.audio.seek(position);
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        self.ensure_attached();
        self.inner.read(rgba)
    }

    // The layout hooks are forwarded, not defaulted. Without them this wrapper
    // reported every source as RGBA8, so a `native-audio` build decoded a
    // planar source through the RGBA read: the planes were converted on the
    // decode thread into a fresh picture instead of a pooled one, and a screen
    // that samples NV12 itself never saw the plan.
    fn output_format(&self) -> PixelFormat {
        self.inner.output_format()
    }

    fn read_nv12(&mut self, nv12: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        self.ensure_attached();
        self.inner.read_nv12(nv12)
    }

    fn nv12_color(&self) -> YuvColor {
        self.inner.nv12_color()
    }

    fn play(&mut self) {
        self.ensure_attached();
        self.inner.play();
        self.audio.play();
    }

    fn pause(&mut self) {
        self.inner.pause();
        self.audio.pause();
    }

    fn set_speed(&mut self, speed: f32) {
        self.inner.set_speed(speed);
        self.audio.set_speed(speed);
    }

    fn set_muted(&mut self, muted: bool) {
        self.inner.set_muted(muted);
        self.audio.set_muted(muted);
    }

    fn set_volume(&mut self, volume: f32) {
        self.inner.set_volume(volume);
        self.audio.set_volume(volume);
    }

    fn set_audio_group(&mut self, group: u64) {
        // Forward to both owners: the platform decoder (which may carry audio)
        // and the generic SoftwareMediaPlayer paired with this source, so a
        // newer player in the same group mutes this one on either path.
        self.inner.set_audio_group(group);
        self.audio.set_group(group);
    }

    fn sync_clock(&mut self, position: Duration) {
        self.inner.sync_clock(position);
        self.audio.observe_output_route();
        self.audio.sync_clock(position);
    }

    fn is_muted(&self) -> bool {
        self.audio.is_muted()
    }

    fn volume(&self) -> f32 {
        self.audio.volume()
    }

    fn carries_audio(&self) -> bool {
        self.inner.carries_audio()
    }

    fn close(&mut self) {
        self.audio.detach();
        self.inner.close();
    }

    /// Forward the player's close signal to the decoder inside this wrapper.
    /// The default implementation ignores it, which meant a network decoder
    /// paired with native audio never saw a close and could hold the decode
    /// thread in a blocking read while `VideoPlayer::close` waited to join.
    fn observe_shutdown(&mut self, flag: Arc<AtomicBool>) {
        self.inner.observe_shutdown(flag);
    }

    /// Give both this wrapper and the decoder inside it the recovery handle.
    /// The wrapper reports failed audio opens into it; the inner decoder may own
    /// its own sound on other backends.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn attach_session(&mut self, session: crate::PlaybackSession) {
        self.audio.set_session(session.clone());
        self.inner.attach_session(session);
    }

    /// Reopen sound on the decode thread when the worker's cooldown elapses.
    /// Audio runs on its own player and clock, so a retry never touches the
    /// picture and never runs on the composition thread.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn recover(&mut self) -> bool {
        let inner = self.inner.recover();
        let audio = self.audio.retry();
        inner && audio
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VideoSource;

    struct Silent;

    impl VideoDecoder for Silent {
        fn info(&self) -> VideoInfo {
            VideoInfo::default()
        }

        fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
            Ok(())
        }

        fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            Ok(None)
        }
    }

    #[test]
    fn audio_keeps_state_without_a_device() {
        let mut audio = Audio::new();
        assert!(!audio.is_muted());
        assert_eq!(audio.volume(), 1.0);

        audio.set_muted(true);
        audio.set_volume(0.4);
        assert!(audio.is_muted());
        assert_eq!(audio.volume(), 0.4);

        // Muting and unmuting never changes the volume the caller chose.
        audio.set_muted(false);
        assert!(!audio.is_muted());
        assert_eq!(audio.volume(), 0.4);
    }

    fn wrapped(source: VideoSource) -> bool {
        let mut decoder = wrap_direct(Box::new(Silent), &source);
        decoder.set_muted(true);
        decoder.is_muted()
    }

    #[test]
    fn wrap_direct_pairs_a_file_or_url_only() {
        assert!(wrapped(VideoSource::path("clip.mp4")));
        assert!(wrapped(VideoSource::url("https://host/clip.mp4")));
        // HLS carries its audio in the stream and bytes have no address the
        // audio backend can open, so neither is paired.
        assert!(!wrapped(VideoSource::url("https://host/index.m3u8")));
        assert!(!wrapped(VideoSource::bytes(vec![0, 1, 2])));
    }

    fn anchored(head: Duration, seconds_ago: u64, playing: bool, speed: f32) -> Audio {
        let mut audio = Audio::new();
        audio.head = head;
        audio.head_at = Instant::now() - Duration::from_secs(seconds_ago);
        audio.playing = playing;
        audio.speed = speed;
        audio
    }

    #[test]
    fn head_advances_from_its_anchor_only_while_playing() {
        let audio = anchored(Duration::from_secs(10), 2, true, 1.0);
        let head = audio.head();
        assert!(
            head >= Duration::from_secs(12) && head < Duration::from_secs(13),
            "head must advance at speed: {head:?}"
        );

        let paused = anchored(Duration::from_secs(10), 2, false, 1.0);
        assert_eq!(
            paused.head(),
            Duration::from_secs(10),
            "a paused head stays put"
        );
    }

    #[test]
    fn head_uses_the_speed_the_backend_honours() {
        // The picture allows 8x but the media backend clamps to 4x. The estimate
        // must follow the sound, or it would chase a speed no device can play.
        let audio = anchored(Duration::ZERO, 2, true, 8.0);
        let head = audio.head();
        assert!(
            head >= Duration::from_secs(8) && head < Duration::from_secs(9),
            "head must use the clamped speed: {head:?}"
        );
    }

    #[test]
    fn pausing_banks_elapsed_time() {
        let mut audio = anchored(Duration::from_secs(10), 2, true, 1.0);
        audio.pause();
        assert!(
            audio.head >= Duration::from_secs(12) && audio.head < Duration::from_secs(13),
            "pause must freeze the head at its current position: {:?}",
            audio.head
        );
        let frozen = audio.head;
        assert_eq!(audio.head(), frozen, "a paused head does not move");
    }

    #[test]
    fn changing_speed_banks_time_at_the_old_rate() {
        let mut audio = anchored(Duration::from_secs(10), 2, true, 1.0);
        audio.set_speed(2.0);
        assert!(
            audio.head >= Duration::from_secs(12) && audio.head < Duration::from_secs(13),
            "the old speed must count before the new one starts: {:?}",
            audio.head
        );
        assert_eq!(audio.speed, 2.0);
    }

    #[test]
    fn only_genuine_gaps_need_a_resync() {
        let head = Duration::from_secs(10);
        assert!(!Audio::needs_resync(head, head + RESYNC_TOLERANCE));
        assert!(!Audio::needs_resync(
            head,
            head.saturating_sub(RESYNC_TOLERANCE)
        ));
        // A late frame re-anchors the clock backwards; a rollover can jump
        // forwards. Both are larger than the ordinary run-out between clocks.
        assert!(Audio::needs_resync(
            head,
            head + RESYNC_TOLERANCE + Duration::from_millis(1)
        ));
        assert!(Audio::needs_resync(
            head,
            head.saturating_sub(RESYNC_TOLERANCE + Duration::from_millis(1))
        ));
    }

    #[test]
    fn seek_moves_the_head_and_detach_resets_it() {
        let mut audio = Audio::new();
        audio.seek(Duration::from_secs(7));
        assert_eq!(audio.head, Duration::from_secs(7));
        assert!(audio.last_seek_at.is_none(), "a seek clears the cooldown");

        audio.detach();
        assert_eq!(audio.head, Duration::ZERO, "a new source starts at zero");
    }

    #[test]
    fn route_rebind_requires_an_exact_pcm_match() {
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            let bound = (2, 48_000, SampleFormat::F32);
            assert!(rebindable(bound, (2, 48_000, SampleFormat::F32)));
            assert!(!rebindable(bound, (1, 48_000, SampleFormat::F32)));
            assert!(!rebindable(bound, (2, 44_100, SampleFormat::F32)));
            assert!(!rebindable(bound, (2, 48_000, SampleFormat::I16)));
        }
    }

    #[test]
    fn route_poll_is_a_no_op_until_a_source_is_prepared() {
        let mut audio = Audio::new();
        audio.observe_output_route();
        assert!(audio.source.is_none());
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        assert!(audio.route.is_none());
        assert!(!audio.playing);
    }

    // A missing file fails in the media decoder before any output device is
    // opened, so these two tests run with no audio hardware present.
    const MISSING: &str = "__cranpose_no_such_media_7f2a9c__.mp4";

    #[test]
    fn a_failed_attach_is_retried_but_throttled() {
        let mut audio = Audio::new();
        audio.attach(MISSING);
        assert!(audio.is_source(MISSING));
        assert!(!audio.is_ready(), "a missing source cannot prepare");
        let scheduled = audio.retry_at.expect("a failed prepare schedules a retry");

        // Another call inside the cooldown must not reopen the source. This is
        // the case that would otherwise re-prepare on every decoded frame.
        audio.attach(MISSING);
        assert_eq!(
            audio.retry_at,
            Some(scheduled),
            "a failed source must not be reopened on every frame"
        );

        // Once the cooldown has elapsed the next read tries again, so a device
        // that was briefly busy after a switch can recover the sound.
        audio.retry_at = Some(Instant::now() - Duration::from_millis(1));
        audio.attach(MISSING);
        assert!(
            audio.retry_at.is_some_and(|at| at > Instant::now()),
            "a still-failing prepare must schedule the next retry"
        );
    }

    #[test]
    fn detach_clears_the_attach_retry_state() {
        let mut audio = Audio::new();
        audio.attach(MISSING);
        assert!(audio.retry_at.is_some());
        assert!(audio.is_source(MISSING));
        audio.detach();
        assert!(
            audio.retry_at.is_none(),
            "a new source starts with no retry pending"
        );
        assert!(!audio.is_source(MISSING));
        assert!(!audio.is_ready());
    }

    #[test]
    fn a_failed_rebind_keeps_the_next_read_trying_to_attach() {
        // [SOURCE-VERIFIED] A route rebind can fail after the wrapper latched
        // `attached`. That combination must still ask `Audio::attach` to retry
        // when the device returns, or the channel stays silent for good.
        assert!(needs_attach(false, false), "first read tries to attach");
        assert!(needs_attach(false, true), "a ready decoder must latch");
        // [HYPOTHESIS] A stream that dies without changing `ready` needs an
        // explicit `session.failed` from its backend; `needs_attach` alone
        // cannot see that condition. AAudio's callback is log-only in the
        // pinned dependency, so Android reports it through the reader hooks.
        assert!(!needs_attach(true, true), "a live stream needs no work");
        assert!(needs_attach(true, false), "a failed rebind must retry");
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    #[test]
    fn a_failed_prepare_is_reported_to_the_session_once_per_source() {
        use crate::{PlaybackSession, SessionEvent};

        let mut audio = Audio::new();
        let session = PlaybackSession::new();
        audio.set_session(session.clone());

        audio.attach(MISSING);
        let events = session.events();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    SessionEvent::Failed {
                        stage: "audio.prepare",
                        ..
                    }
                ))
                .count(),
            1,
            "the first failed prepare must arm worker recovery"
        );

        // The same source failing again does not re-arm on every read. The
        // worker owns the retry cadence once the first report has been made.
        audio.retry_at = Some(Instant::now() - Duration::from_millis(1));
        audio.attach(MISSING);
        assert_eq!(
            session
                .events()
                .iter()
                .filter(|event| matches!(
                    event,
                    SessionEvent::Failed {
                        stage: "audio.prepare",
                        ..
                    }
                ))
                .count(),
            1,
            "a repeated failed prepare must not spam the session"
        );

        const OTHER_MISSING: &str = "__cranpose_no_such_media_other__.mp4";
        audio.attach(OTHER_MISSING);
        assert_eq!(
            session
                .events()
                .iter()
                .filter(|event| matches!(
                    event,
                    SessionEvent::Failed {
                        stage: "audio.prepare",
                        ..
                    }
                ))
                .count(),
            2,
            "a genuinely new source gets its own single report"
        );
    }
}
