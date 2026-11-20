//! Windows audio, read from the same Media Foundation source reader as the
//! picture.
//!
//! A single source reader demuxes and decodes both tracks. The video half of
//! that reader is driven by `windows.rs`; this module drives the audio half and
//! renders its PCM through `cpal`. Both halves read timestamps from the reader's
//! clock (100-nanosecond ticks), so rebasing each to its own first sample puts
//! audio and picture on one timeline. The play head is re-anchored to the frame
//! time the player publishes, which keeps the two clocks together through
//! pauses, speed changes and seeks.
//!
//! Everything here is best effort. If there is no audio track, no output device,
//! or Media Foundation refuses to produce float PCM, `setup` returns `None` and
//! the video decoder carries on unchanged. Nothing in this module can fail the
//! picture.
//!
//! The Media Foundation reader is only ever touched on the decode thread (the
//! caller of `pump` and the control methods). The `cpal` callback, which runs on
//! an audio device thread, only reads plain PCM and clock state behind locks.
//! No COM object crosses between the two.

#![expect(
    unsafe_code,
    reason = "Media Foundation is a COM API, so every call crosses an FFI boundary"
)]

use std::{
    collections::VecDeque,
    mem::size_of,
    ptr, slice,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use cpal::{
    Data, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use parking_lot::Mutex;
use windows::{
    Win32::Media::MediaFoundation::{
        IMFMediaType, IMFSample, IMFSourceReader, MF_MT_ALL_SAMPLES_INDEPENDENT,
        MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT,
        MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
        MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF_SOURCE_READERF_STREAMTICK,
        MFAudioFormat_Float, MFCreateMediaType, MFMediaType_Audio,
    },
    core::GUID,
};

/// How far ahead of the picture the PCM buffer is kept. It covers the sleep
/// between two video reads plus a device buffer, so sound stays continuous.
const LEAD: Duration = Duration::from_secs(1);
/// How much decoded history is kept behind the play head. The callback only
/// reads near the head; keeping a little behind absorbs speed changes.
const TAIL: Duration = Duration::from_secs(2);
/// How long one video read may spend decoding audio. The limit is wall-clock
/// time, not a sample count: a fast decoder fills the whole lead in one go
/// while a slow one still returns to the picture promptly.
const PUMP_BUDGET: Duration = Duration::from_millis(4);
/// How many samples a silence/gap edge is faded over. A raw jump from decoded
/// sound to silence (or back) is heard as a click or a beat.
const DECLICK: usize = 128;
/// The dead band within which picture and sound count as together. Nothing is
/// corrected inside it: every correction is a discontinuity, and one per
/// published picture is heard as a pulse even when each is tiny.
const SYNC_TOLERANCE: Duration = Duration::from_millis(60);
/// The media a start waits for before it begins, so the first callback never
/// runs dry and stutters. The decoder keeps filling towards `LEAD` afterwards.
const START_LEAD: Duration = Duration::from_millis(120);
/// How long a fresh start may wait for enough PCM before it plays with what it
/// has. A missing buffer must not mute sound forever when a new source starts.
const PRIME_TIMEOUT: Duration = Duration::from_millis(500);
/// The longest silence inserted to let a picture that fell behind catch up.
/// Past this the head is re-anchored instead, so a corrupt stamp cannot mute
/// sound for minutes.
const MAX_HOLD: Duration = Duration::from_secs(2);
/// A gap larger than this is treated as a new span rather than filled with
/// silence, so a bad timestamp cannot allocate an unbounded buffer.
const MAX_GAP: Duration = Duration::from_secs(10);
/// How far the sample cursor may wander from the mirrored clock before it is
/// pulled back. The cursor is what the device hears; the clock only decides
/// when to correct, so a little room keeps device scheduling jitter from
/// skipping or repeating samples.
const CURSOR_DRIFT: Duration = Duration::from_millis(40);
/// The shortest gap between attempts to follow a new default output device.
/// WASAPI reports a device change once, and the replacement can take a moment
/// to become available, so a failed rebind is retried on later video reads
/// rather than dropped. The cooldown keeps a failed retry off every frame.
const REBIND_COOLDOWN: Duration = Duration::from_millis(500);

/// Whether a `cpal` stream error means the system default output changed.
///
/// CPAL reports a new default as `StreamInvalidated` and a vanished default as
/// `DeviceNotAvailable`. Both are reasons to look for a device again; every
/// other error (busy, permission, format) is left alone so a transient error
/// cannot start a needless rebind loop.
fn is_device_change(error: &cpal::Error) -> bool {
    matches!(
        error.kind(),
        cpal::ErrorKind::StreamInvalidated | cpal::ErrorKind::DeviceNotAvailable
    )
}

/// Finds an output device whose mix format can actually be read.
///
/// `default_output_device()` is a *virtual* handle: cpal activates it through
/// `DEVINTERFACE_AUDIO_RENDER` and caches an `IAudioClient` in the handle. After
/// a headset is unplugged, Windows can keep resolving that path to the dead
/// endpoint for minutes, so `default_output_config()` fails while the speakers
/// are already audible. Enumerating concrete render endpoints and reading each
/// one's mix format finds the live device directly instead of waiting.
///
/// The default is tried first, so in the normal case this is one query and the
/// user's chosen device is used. Enumeration is only the fallback.
fn output_device_with_config() -> Option<(cpal::Device, cpal::SupportedStreamConfig)> {
    let host = cpal::default_host();
    if let Some(device) = host.default_output_device() {
        if let Ok(config) = device.default_output_config() {
            return Some((device, config));
        }
    }
    if let Ok(devices) = host.devices() {
        for device in devices {
            if let Ok(config) = device.default_output_config() {
                return Some((device, config));
            }
        }
    }
    None
}

/// What a replacement default output device needs before this source can play
/// on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RebindPlan {
    /// The layout is identical: only the stream moves.
    Keep,
    /// Same channels and rate, different sample format. The decoded PCM is
    /// already float, so only the stream has to be built for the new format.
    Reformat,
    /// A different rate or channel count: the reader has to produce PCM in the
    /// new layout before any stream can play it.
    Renegotiate,
}

/// What to do when the system default output changes from `current` to
/// `observed`, both as `(channels, rate, sample_format)`.
///
/// Refusing a device whose layout differs is what made a headset switch mute
/// both devices: the invalidated stream cannot be heard, and the replacement
/// was never built. A different layout is a reason to renegotiate the PCM, not
/// a reason to give up on sound.
fn rebind_plan(
    current: (usize, u32, SampleFormat),
    observed: (usize, u32, SampleFormat),
) -> RebindPlan {
    if current == observed {
        RebindPlan::Keep
    } else if (current.0, current.1) == (observed.0, observed.1) {
        RebindPlan::Reformat
    } else {
        RebindPlan::Renegotiate
    }
}

/// Only a genuine end-of-stream ends an audio span. A reader error is a
/// recoverable stall: the next video read retries the audio read, so one
/// transient failure cannot leave the picture running with sound gone forever.
fn is_terminal_audio_eof(flags: u32) -> bool {
    flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0
}

fn is_recoverable_audio_flags(flags: u32) -> bool {
    flags & (MF_SOURCE_READERF_ERROR.0 as u32) != 0
}

/// Whether the mirror clock may start at `position`.
///
/// The normal case is a full lead of PCM. At a true end-of-stream the last
/// partial run may still be played, but an empty buffer must never start a
/// running clock: that is what turns a read failure into permanent silence.
fn start_ready(ended: bool, filled: Option<Duration>, position: Duration) -> bool {
    let full_lead = filled.is_some_and(|end| end >= position.saturating_add(START_LEAD));
    full_lead || (ended && filled.is_some_and(|end| end > position))
}

/// The stream index the reader's audio selector stands for.
fn audio() -> u32 {
    windows::Win32::Media::MediaFoundation::MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32
}

/// State shared between the decode thread and the audio device callback.
///
/// No Media Foundation object lives here; the callback only ever touches plain
/// data, which keeps COM on the decode thread where its apartment is joined.
struct AudioCore {
    /// Decoded, interleaved float PCM in media-time order.
    buffer: Mutex<PcmBuffer>,
    /// The play head the callback reads, mirroring the player's own clock.
    clock: Mutex<MirrorClock>,
    /// Where the device itself is, advanced by whole callback buffers.
    cursor: Mutex<Cursor>,
    /// One scratch frame, reused so the device callback never allocates.
    frame: Mutex<Vec<f32>>,
    /// One de-click state per device channel, also reused by the callback.
    gates: Mutex<Vec<DeClick>>,
    muted: AtomicBool,
    /// The volume as `f32` bits, so it can live in an atomic.
    volume: AtomicU32,
    channels: usize,
    rate: u32,
    /// Set by the first output callback, which proves the device is pulling
    /// PCM rather than the stream merely having been built.
    rendering: AtomicBool,
}

impl AudioCore {
    fn new(channels: usize, rate: u32) -> Self {
        Self {
            buffer: Mutex::new(PcmBuffer::new(rate)),
            clock: Mutex::new(MirrorClock::new()),
            cursor: Mutex::new(Cursor::new()),
            frame: Mutex::new(vec![0.0; channels]),
            gates: Mutex::new((0..channels).map(|_| DeClick::new()).collect()),
            muted: AtomicBool::new(false),
            volume: AtomicU32::new(1.0_f32.to_bits()),
            channels,
            rate,
            rendering: AtomicBool::new(false),
        }
    }

    fn position(&self) -> Duration {
        self.clock.lock().position()
    }

    /// Takes the play head, mute and volume from the core a replaced stream is
    /// still reading, so changing the PCM layout does not restart the sound.
    fn carry_over(&self, from: &Self) {
        let position = from.position();
        self.muted
            .store(from.muted.load(Ordering::Relaxed), Ordering::Relaxed);
        self.volume
            .store(from.volume.load(Ordering::Relaxed), Ordering::Relaxed);
        self.clock.lock().seek(position);
    }
}

/// Where the device is in media time, advanced by whole callback buffers.
///
/// Deriving every callback from the wall clock lets a late or early device
/// callback repeat or skip samples mid-waveform, which is heard as a click.
/// The cursor only follows the clock when the clock moves deliberately — a
/// start, a seek, a speed change, a correction — or when the two have wandered
/// far enough apart that the picture would notice.
struct Cursor {
    media: Duration,
    epoch: u64,
}

impl Cursor {
    fn new() -> Self {
        Self {
            media: Duration::ZERO,
            epoch: 0,
        }
    }
}

/// One callback's view of the clock.
struct RenderState {
    position: Duration,
    speed: f64,
    /// Render silence: paused, waiting for the first picture, or holding the
    /// head still so a picture behind it can catch up.
    silent: bool,
    epoch: u64,
}

/// A playback clock that mirrors the player's clock.
///
/// The head advances at exactly the playback speed, never at a correction
/// factor: moving the sample reader at anything but `speed` resamples the
/// sound and changes its pitch. A picture that falls behind is given silence to
/// catch up in; a picture ahead skips once. Both are inaudible corrections
/// where resampling was an audible one.
struct MirrorClock {
    base: Duration,
    anchor: Instant,
    speed: f64,
    running: bool,
    /// Waiting for the first picture of this run before making a sound.
    waiting: bool,
    /// Wall-clock start of that wait. Media time is frozen while waiting, so it
    /// cannot measure its own deadline.
    wait_started: Option<Instant>,
    /// When a picture is behind the head, the instant the silence began and how
    /// long it lasts. During it the head stands still and the device is fed
    /// zeroes, so the picture catches up without any sample being changed.
    hold: Option<(Instant, Duration)>,
    /// A picture has stated where sound belongs, so a resume can start at once;
    /// a fresh open or a seek clears it and waits for the picture that lands.
    primed: bool,
    /// Bumped whenever wall time maps to media time discontinuously, so the
    /// device cursor abandons its old place instead of walking into a jump.
    epoch: u64,
}

impl MirrorClock {
    fn new() -> Self {
        Self {
            base: Duration::ZERO,
            anchor: Instant::now(),
            speed: 1.0,
            running: false,
            waiting: false,
            wait_started: None,
            hold: None,
            primed: false,
            epoch: 0,
        }
    }

    /// A starved start becomes a normal run after a bounded wait. The head is
    /// re-anchored so the wait is not heard as a jump when sound begins, and
    /// the epoch moves like every other discontinuity so the device cursor
    /// re-seeds at the same place.
    fn retire_expired_wait(&mut self) {
        if !self.waiting
            || !self
                .wait_started
                .is_some_and(|started| started.elapsed() >= PRIME_TIMEOUT)
        {
            return;
        }
        self.waiting = false;
        self.wait_started = None;
        self.primed = true;
        self.hold = None;
        self.anchor = Instant::now();
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn position(&mut self) -> Duration {
        self.retire_expired_wait();
        if !self.running || self.waiting {
            return self.base;
        }
        if let Some((started, duration)) = self.hold {
            if started.elapsed() < duration {
                return self.base;
            }
            // The silence is over. Count wall time from its end, not from
            // before it, so the hold neither skips nor repeats any media.
            self.anchor = started + duration;
            self.hold = None;
            self.epoch = self.epoch.wrapping_add(1);
        }
        let elapsed = self.anchor.elapsed().as_secs_f64() * self.speed;
        self.base
            .saturating_add(Duration::from_secs_f64(elapsed.max(0.0)))
    }

    fn play(&mut self) {
        if self.running {
            return;
        }
        self.running = true;
        // A start with no picture yet waits for one; a resume from pause starts
        // where the paused picture left the head.
        self.waiting = !self.primed;
        self.wait_started = self.waiting.then(Instant::now);
        self.hold = None;
        self.anchor = Instant::now();
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn pause(&mut self) {
        if !self.running {
            return;
        }
        let position = self.position();
        self.base = position;
        self.anchor = Instant::now();
        self.running = false;
        self.waiting = false;
        self.wait_started = None;
        self.hold = None;
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn set_speed(&mut self, speed: f32) {
        let position = self.position();
        self.base = position;
        self.anchor = Instant::now();
        self.speed = f64::from(speed);
        self.hold = None;
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn seek(&mut self, position: Duration) {
        self.base = position;
        self.anchor = Instant::now();
        self.hold = None;
        // A jump in the media has no audio yet: wait for the picture the seek
        // lands on, so sound starts at the picture rather than at the target.
        self.waiting = self.running;
        self.wait_started = self.waiting.then(Instant::now);
        self.primed = false;
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// Re-anchors the head to the picture the player just published.
    ///
    /// `ready` is false when too little sound is buffered to start yet; the
    /// head keeps waiting rather than running dry.
    fn sync_to(&mut self, position: Duration, ready: bool) {
        if !self.running {
            // A picture published while still is where playback will resume.
            self.base = position;
            self.anchor = Instant::now();
            self.primed = true;
            self.wait_started = None;
            return;
        }
        if self.waiting {
            if !ready {
                return;
            }
            self.base = position;
            self.anchor = Instant::now();
            self.waiting = false;
            self.wait_started = None;
            self.primed = true;
            self.hold = None;
            self.epoch = self.epoch.wrapping_add(1);
            return;
        }
        self.primed = true;

        if self.hold.is_some() {
            // Calling position retires a hold whose time is up.
            let _ = self.position();
            if self.hold.is_some() {
                if position.saturating_add(SYNC_TOLERANCE) >= self.base {
                    self.hold = None;
                    self.anchor = Instant::now();
                    self.epoch = self.epoch.wrapping_add(1);
                }
                return;
            }
        }

        let now = self.position();
        if position > now {
            if position - now > SYNC_TOLERANCE {
                // The picture is ahead. Skip the sound that was missed once,
                // then continue at the same rate — never resampled.
                self.base = position;
                self.anchor = Instant::now();
                self.epoch = self.epoch.wrapping_add(1);
            }
        } else if now - position > SYNC_TOLERANCE {
            // The picture is behind. Hold the head still and render silence
            // until it catches up. The samples are neither skipped nor
            // resampled, so the voice keeps its pitch. `gap` is media time;
            // the picture catches up at `speed` media seconds per wall second,
            // so the wall-time hold is gap / speed. Using gap directly made a
            // fast player overshoot and a slow one resume early, an alternating
            // hold/skip pulse that only appears away from 1x.
            let gap = now - position;
            let started = Instant::now();
            self.base = now;
            self.anchor = started;
            let hold = gap.div_f64(self.speed.max(0.001)).min(MAX_HOLD);
            self.hold = Some((started, hold));
            self.epoch = self.epoch.wrapping_add(1);
        }
    }

    fn render_state(&mut self) -> RenderState {
        let position = self.position();
        RenderState {
            position,
            speed: self.speed,
            silent: !self.running || self.waiting || self.hold.is_some(),
            epoch: self.epoch,
        }
    }
}

/// A click-free edge between decoded sound, silence, and decoded sound again.
///
/// Gaps in a media stream are normally filled with zero. Rendering that zero
/// immediately produces a beat, and resuming at full level produces another.
/// This keeps the last rendered value and fades it out, then ramps new samples
/// back in from silence.
struct DeClick {
    last: f32,
    ramp: usize,
    present: bool,
    started: bool,
}

impl DeClick {
    fn new() -> Self {
        Self {
            last: 0.0,
            ramp: DECLICK,
            present: false,
            started: false,
        }
    }

    /// Returns one rendered sample.
    fn sample(&mut self, value: f32, present: bool) -> f32 {
        let out = if present {
            if !self.present && self.started {
                // Sound returned after a gap: begin quietly and ramp back from
                // zero instead of jumping to half level, which is a click.
                self.ramp = DECLICK;
            }
            self.started = true;
            let used = DECLICK.saturating_sub(self.ramp);
            let gain = if self.ramp == 0 {
                1.0
            } else {
                (used + 1) as f32 / DECLICK as f32
            };
            self.ramp = self.ramp.saturating_sub(1);
            value * gain
        } else if self.last.abs() > 0.0 {
            self.ramp = DECLICK;
            self.last * 0.9
        } else {
            0.0
        };
        self.present = present;
        self.last = out;
        out
    }

    fn reset(&mut self) {
        self.last = 0.0;
        self.ramp = DECLICK;
        self.present = false;
        self.started = false;
    }
}

/// A contiguous run of interleaved PCM, addressed by its media presentation
/// time. `start` is the media time of `samples[0]`, and is absent until the
/// first decoded block arrives (or after a seek).
struct PcmBuffer {
    samples: VecDeque<f32>,
    start: Option<Duration>,
    rate: u32,
}

impl PcmBuffer {
    fn new(rate: u32) -> Self {
        Self {
            samples: VecDeque::new(),
            start: None,
            rate,
        }
    }

    /// The media time just past the last sample.
    fn filled_until(&self, channels: usize) -> Option<Duration> {
        if channels == 0 || self.samples.is_empty() {
            return None;
        }
        let frames = self.samples.len() / channels;
        self.start.map(|start| {
            start.saturating_add(Duration::from_secs_f64(
                frames as f64 / f64::from(self.rate),
            ))
        })
    }

    /// Appends a decoded block at `pts`, filling gaps with silence and dropping
    /// any overlap so the buffer stays one continuous run of media time.
    fn push(&mut self, pts: Duration, block: &[f32], channels: usize) {
        if block.is_empty() || channels == 0 {
            return;
        }
        let Some(start) = self.start else {
            self.start = Some(pts);
            self.samples.extend(block.iter().copied());
            return;
        };

        let frames = self.samples.len() / channels;
        let end = start.saturating_add(Duration::from_secs_f64(
            frames as f64 / f64::from(self.rate),
        ));
        if pts > end {
            let gap = frames_between(end, pts, self.rate);
            if gap > self.rate as usize * MAX_GAP.as_secs() as usize {
                self.samples.clear();
                self.start = Some(pts);
            } else {
                for _ in 0..gap.saturating_mul(channels) {
                    self.samples.push_back(0.0);
                }
            }
        } else {
            let overlap = frames_between(pts, end, self.rate);
            let skip = overlap.saturating_mul(channels);
            if skip >= block.len() {
                return;
            }
            if skip > 0 {
                self.samples.extend(block[skip..].iter().copied());
                return;
            }
        }
        self.samples.extend(block.iter().copied());
    }

    /// Drops samples older than `pts`, keeping the play head where it is.
    fn prune(&mut self, pts: Duration, channels: usize) {
        let Some(start) = self.start else {
            return;
        };
        if channels == 0 || pts <= start {
            return;
        }
        let available = self.samples.len() / channels;
        let frames = frames_between(start, pts, self.rate).min(available);
        if frames == 0 {
            return;
        }
        for _ in 0..frames.saturating_mul(channels) {
            self.samples.pop_front();
        }
        self.start = Some(start.saturating_add(Duration::from_secs_f64(
            frames as f64 / f64::from(self.rate),
        )));
    }

    fn reset(&mut self) {
        self.samples.clear();
        self.start = None;
    }
}

/// Reads one interleaved frame at `pts` into `out`. Returns false for a gap or a
/// position past what has been decoded, which the caller plays as silence.
fn frame_at(
    samples: &VecDeque<f32>,
    start: Duration,
    rate: u32,
    pts: Duration,
    out: &mut [f32],
) -> bool {
    if out.is_empty() || pts < start {
        return false;
    }
    let frames = frame_index(start, pts, rate);
    let base = match frames.checked_mul(out.len()) {
        Some(base) => base,
        None => return false,
    };
    if base + out.len() > samples.len() {
        return false;
    }
    for (index, sample) in out.iter_mut().enumerate() {
        *sample = samples[base + index];
    }
    true
}

/// The index of the sample whose time slice contains `pts`.
///
/// Rounding here would skip the first sample for the first half of its own
/// slice, and drift by one after an hour where `f64` can no longer address a
/// single sample. Integer nanoseconds keep the choice exact at every position.
fn frame_index(from: Duration, to: Duration, rate: u32) -> usize {
    if to <= from || rate == 0 {
        return 0;
    }
    let nanos = (to - from).as_nanos();
    let frames = nanos * u128::from(rate) / 1_000_000_000;
    usize::try_from(frames).unwrap_or(usize::MAX)
}

fn frames_between(from: Duration, to: Duration, rate: u32) -> usize {
    frame_index(from, to, rate)
}

/// The media position of output frame `index` in a callback that starts its
/// play head at `head`.
///
/// Both the head and the sample reader use `speed`, so consecutive callbacks
/// land on contiguous samples: the last frame of one and the first of the next
/// are exactly one sample apart, and every frame is a real sample. Nothing is
/// interpolated, so the pitch is the recorded one at normal speed.
fn media_time(head: Duration, speed: f64, rate_hz: f64, index: usize) -> Duration {
    if rate_hz <= 0.0 {
        return head;
    }
    let step = index as f64 / rate_hz * speed;
    head.saturating_add(Duration::from_secs_f64(step.max(0.0)))
}

/// The head of the next callback: one whole buffer further on.
///
/// At normal speed the step is computed in integer nanoseconds, so an hour of
/// callbacks cannot accumulate rounding drift; other speeds step fractionally
/// by definition.
fn advance_cursor(head: Duration, speed: f64, rate_hz: u32, frames: usize) -> Duration {
    if rate_hz == 0 {
        return head;
    }
    if speed == 1.0 {
        let nanos = (frames as u128 * 1_000_000_000) / u128::from(rate_hz);
        head.saturating_add(Duration::from_nanos(
            u64::try_from(nanos).unwrap_or(u64::MAX),
        ))
    } else {
        head.saturating_add(Duration::from_secs_f64(
            frames as f64 / f64::from(rate_hz) * speed,
        ))
    }
}

/// Maps a raw Media Foundation stamp onto the play head of its span.
///
/// The first sample of a span starts at `head`, whether Media Foundation stamps
/// it with an absolute media time after a seek or restarts it at zero. Later
/// samples keep their distance from that first stamp, so one span stays one
/// continuous run of media time that the callback can address.
fn span_pts(stamp: Duration, first: Option<Duration>, head: Duration) -> (Duration, Duration) {
    let first = first.unwrap_or(stamp);
    (head.saturating_add(stamp.saturating_sub(first)), first)
}

/// The media time at which the first sample after a stream discontinuity begins.
///
/// A stream tick (raised at each HLS fragment join) re-bases the sample stamps,
/// but the callback reads one continuous run of media time. The new span must
/// therefore continue where the buffered PCM already ends, not jump to the
/// play head — which carries the resampler's `drift` and would open a gap or an
/// overlap at every fragment boundary. Falls back to the head when nothing is
/// buffered yet.
fn tick_span_head(buffered_end: Option<Duration>, head: Duration) -> Duration {
    buffered_end.unwrap_or(head)
}

/// The audio half of the source reader, kept alive on the decode thread.
pub(super) struct Audio {
    core: Arc<AudioCore>,
    /// The output stream. Dropping it stops the callback and releases the device.
    _stream: cpal::Stream,
    channels: usize,
    /// The sample format the stream was built with. A replacement device may
    /// report another one: the PCM stays float and the callback writes it in
    /// that format, so only a rate or channel change needs the reader again.
    format: SampleFormat,
    /// Set from the `cpal` error callback when the default output device changes.
    /// Only this flag crosses from the device thread to the decode thread; no
    /// Media Foundation object ever does.
    device_changed: Arc<AtomicBool>,
    /// Whether a failed rebind has already been logged, so a device that stays
    /// away is retried quietly instead of writing two lines a second.
    rebind_warned: bool,
    /// When the last rebind was attempted, so a failed retry is throttled.
    last_rebind: Option<Instant>,
    /// The output device the live stream was built for. Compared with the
    /// system default on a video read so a route change is followed even when
    /// the device callback never reports one — which is exactly what happens
    /// when Windows swaps the default while the stream is quiet. A `cpal::Device`
    /// is an Eq + Send handle, so it is safe to keep on the decode thread.
    bound_device: Option<cpal::Device>,
    /// The PCM layout the live stream was built for. A device can keep its name
    /// and still change rate or channel count after a headset switch.
    bound_layout: (usize, u32, SampleFormat),
    /// Throttle for the proactive default-device check.
    last_route_check: Option<Instant>,
    /// The raw timestamp of the first sample in the current span of media time.
    span_first: Option<Duration>,
    /// The media time that first sample maps onto. A seek jumps it to the seek
    /// target; a discontinuity adopts the play head at that moment.
    span_head: Duration,
    scratch: Vec<f32>,
    ended: bool,
    /// Set once the first decoded block is recorded, so a session log states
    /// that sound was produced without a line per picture.
    logged_sample: bool,
}

/// Why setting up audio did not produce a live output stream.
///
/// The distinction matters for recovery. A source with no audio track must not
/// be retried forever, while an output device that was briefly busy after a
/// source switch must be.
enum SetupError {
    /// The source has no audio, or Media Foundation cannot give us PCM in a
    /// layout we can render without resampling. This is a property of the
    /// source, so no amount of retrying will change it.
    NoAudio(&'static str),
    /// The output device or stream was temporarily unavailable. The decode
    /// thread retries this on the session's recovery cooldown.
    Unavailable(&'static str),
}

impl SetupError {
    /// Whether a later setup attempt can still bring sound back.
    fn retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

/// The result of checking the PCM type Media Foundation accepted against the
/// layout requested for the output device.
///
/// A mismatch is a *device* failure, not a silent source: the requested rate and
/// channel count came from the default output, so a replacement headset may
/// accept a layout the previous speakers could not. Classifying this as
/// `NoAudio` is what stopped recovery forever after a route change.
fn negotiation_ok(
    accepted_float: bool,
    channels_match: bool,
    rate_match: bool,
) -> Result<(), SetupError> {
    if accepted_float && channels_match && rate_match {
        Ok(())
    } else {
        Err(SetupError::Unavailable(
            "negotiated PCM layout does not match the output device",
        ))
    }
}

/// The outcome of one audio setup attempt.
pub(super) struct Setup {
    /// A live output stream, or `None` when one could not be built.
    pub audio: Option<Audio>,
    /// Whether the source itself carries an audio track this decoder owns.
    ///
    /// This stays true even when the device was busy, so the player does not
    /// pair a second audio player with the same source and the decode thread
    /// keeps trying to bring the sound back.
    pub source_has_audio: bool,
}

impl Audio {
    /// Selects the audio stream, asks Media Foundation for float PCM in the
    /// device's own format, and starts an output stream.
    ///
    /// Every failure path leaves the reader's video stream exactly as it was,
    /// forwards its exact reason to the application's diagnostic sink, and is
    /// classified so the caller knows whether a later retry can help. Nothing
    /// here can fail the picture: `audio` is simply absent while a retry waits.
    pub(super) fn setup(reader: &IMFSourceReader) -> Setup {
        match Self::setup_checked(reader) {
            Ok(audio) => Setup {
                audio: Some(audio),
                source_has_audio: true,
            },
            Err(error @ (SetupError::NoAudio(_) | SetupError::Unavailable(_))) => {
                let reason = match &error {
                    SetupError::NoAudio(reason) | SetupError::Unavailable(reason) => reason,
                };
                crate::diag("WARN", "mf_audio.setup", reason);
                Setup {
                    audio: None,
                    source_has_audio: error.retryable(),
                }
            }
        }
    }

    fn setup_checked(reader: &IMFSourceReader) -> Result<Self, SetupError> {
        // Ask the source about its audio stream before touching any output
        // device. A busy device must not hide the fact that the source is
        // silent (or the reverse), and this single selection is idempotent on a
        // retry from the decode thread.
        // SAFETY: the selector is the constant the reader defines for its first
        // audio stream. A source with no audio refuses the selection.
        if unsafe { reader.SetStreamSelection(audio(), true) }.is_err() {
            return Err(SetupError::NoAudio("source has no selectable audio stream"));
        }

        let (device, config) = output_device_with_config()
            .ok_or(SetupError::Unavailable("no usable output device"))?;
        let channels = config.channels() as usize;
        let channels_u32 = u32::try_from(channels)
            .map_err(|_| SetupError::Unavailable("channel count out of range"))?;
        let rate = config.sample_rate();
        let format = config.sample_format();
        // Remember what this stream is bound to. The proactive check in `pump`
        // compares the current default against these so a route change is picked
        // up even without a device callback.
        let bound_device = Some(device.clone());
        let bound_layout = (channels, rate, format);
        if channels == 0 || rate == 0 {
            return Err(SetupError::Unavailable(
                "device reported zero channels or zero sample rate",
            ));
        }

        let Some(accepted) = request_float(reader, channels_u32, rate) else {
            let _ = unsafe { reader.SetStreamSelection(audio(), false) };
            // The requested layout came from the output device, not from the
            // source. A headset at a different rate can make this fail while
            // another device works, so it is a device failure to retry — never
            // a verdict that the source is silent.
            return Err(SetupError::Unavailable(
                "source audio cannot be negotiated as float PCM for this device",
            ));
        };
        // Media Foundation normally accepts the exact rate and channel count;
        // if it changes either, the PCM layout would differ from the device's,
        // so leave the picture alone instead of playing scrambled sound.
        let accepted_channels = attribute_u32(&accepted, &MF_MT_AUDIO_NUM_CHANNELS)
            .filter(|&value| value as usize == channels);
        let accepted_rate = attribute_u32(&accepted, &MF_MT_AUDIO_SAMPLES_PER_SECOND)
            .filter(|&value| value == rate);
        let accepted_float = attribute_guid(&accepted, &MF_MT_SUBTYPE)
            .is_some_and(|subtype| subtype == MFAudioFormat_Float);
        if let Err(error) = negotiation_ok(
            accepted_float,
            accepted_channels.is_some(),
            accepted_rate.is_some(),
        ) {
            let _ = unsafe { reader.SetStreamSelection(audio(), false) };
            return Err(error);
        }

        let core = Arc::new(AudioCore::new(channels, rate));
        let callback_core = Arc::clone(&core);
        let device_changed = Arc::new(AtomicBool::new(false));
        let change_flag = Arc::clone(&device_changed);
        let stream = device.build_output_stream_raw(
            config.config(),
            format,
            move |data: &mut Data, _: &cpal::OutputCallbackInfo| {
                render(data, &callback_core, format);
            },
            move |error| {
                // A default-output stream reports a change of the system default
                // through its error callback. WASAPI does not rebind the client
                // itself, so flag it for the decode thread to follow. Only a
                // bool is shared with that thread — no COM object crosses over.
                if is_device_change(&error) {
                    change_flag.store(true, Ordering::Release);
                }
            },
            None,
        );
        let Ok(stream) = stream else {
            let _ = unsafe { reader.SetStreamSelection(audio(), false) };
            return Err(SetupError::Unavailable(
                "output stream could not be built (device busy or unavailable)",
            ));
        };
        let mut output = Self {
            core,
            _stream: stream,
            channels,
            format,
            device_changed,
            rebind_warned: false,
            last_rebind: None,
            bound_device,
            bound_layout,
            last_route_check: None,
            span_first: None,
            span_head: Duration::ZERO,
            scratch: Vec::new(),
            ended: false,
            logged_sample: false,
        };
        crate::diag(
            "INFO",
            "mf_audio.setup",
            &format!("audio track open: {channels} channel(s) at {rate} Hz"),
        );
        // Fill the lead before the device starts asking for samples. Starting an
        // empty buffer makes the callback render zeros until the decode thread
        // catches up, which is heard as a click followed by a stuttering beat.
        output.pump(reader);
        if output._stream.play().is_err() {
            let _ = unsafe { reader.SetStreamSelection(audio(), false) };
            return Err(SetupError::Unavailable(
                "output stream could not be started",
            ));
        }

        Ok(output)
    }

    /// Follows a change of the system default output device by rebuilding the
    /// `cpal` stream, and the PCM layout with it when the replacement device
    /// cannot play the one the source was negotiated for.
    ///
    /// The Media Foundation reader, the decoded PCM and the play head all live
    /// in `AudioCore`, which the new stream shares, so a device with the same
    /// layout loses no sound and no timing. A device with another rate or
    /// channel count gets a freshly negotiated PCM layout and a new core that
    /// carries the play head, the mute and the volume across; a different
    /// sample format alone needs no renegotiation at all. Returns whether the
    /// stream was rebound; a failure is retried on a later video read, and
    /// never fails the picture.
    fn rebind(&mut self, reader: &IMFSourceReader) -> bool {
        let Some((device, config)) = output_device_with_config() else {
            return self.rebind_failed("no usable output device");
        };
        let channels = config.channels() as usize;
        let rate = config.sample_rate();
        let format = config.sample_format();
        if channels == 0 || rate == 0 {
            return self.rebind_failed("the replacement device reported no channels or no rate");
        }
        let plan = rebind_plan(
            (self.channels, self.core.rate, self.format),
            (channels, rate, format),
        );
        if plan == RebindPlan::Renegotiate && !self.renegotiate(reader, channels, rate) {
            return false;
        }
        self.format = format;

        let flag = Arc::clone(&self.device_changed);
        let core = Arc::clone(&self.core);
        let stream = device.build_output_stream_raw(
            config.config(),
            format,
            move |data: &mut Data, _: &cpal::OutputCallbackInfo| {
                render(data, &core, format);
            },
            move |error| {
                if is_device_change(&error) {
                    flag.store(true, Ordering::Release);
                }
            },
            None,
        );
        let Ok(stream) = stream else {
            return self.rebind_failed("replacement output stream could not be built");
        };
        // Start the replacement before dropping the old one, so the device
        // never goes quiet between the two. The shared clock is already running
        // at the right place; `render` reads straight from the same core.
        if stream.play().is_err() {
            return self.rebind_failed("replacement output stream could not be started");
        }
        let old = std::mem::replace(&mut self._stream, stream);
        // Tearing an invalidated endpoint down can block inside WASAPI, and
        // this runs on the decode thread that also paces the picture. Release
        // it on a thread of its own so a dead device can never stall playback;
        // the stream owns its callback, so it is still dropped exactly once.
        let _ = std::thread::Builder::new()
            .name("cranpose-video-retire".to_owned())
            .spawn(move || drop(old));
        // "Bind" the stream to the device it now renders into, so the next
        // proactive check compares against the right one and does not rebind
        // straight back to the device the user just left.
        self.bound_device = Some(device.clone());
        self.bound_layout = (channels, rate, format);
        self.last_route_check = Some(Instant::now());
        self.rebind_warned = false;
        crate::diag(
            "INFO",
            "mf_audio.rebind",
            &format!(
                "sound moved to the new default device: {channels} channel(s) at {rate} Hz, \
                 {format:?}, {plan:?}"
            ),
        );
        true
    }

    /// Asks Media Foundation for PCM in the replacement device's layout and
    /// hands the sound to a fresh core, carrying the play head and the controls
    /// over. The old core keeps feeding its stream until the caller drops it,
    /// so no callback ever reads a half-updated layout.
    fn renegotiate(&mut self, reader: &IMFSourceReader, channels: usize, rate: u32) -> bool {
        let Ok(requested) = u32::try_from(channels) else {
            return self.rebind_failed("the replacement device's channel count is out of range");
        };
        let Some(accepted) = request_float(reader, requested, rate) else {
            return self.rebind_failed("the source cannot produce PCM for the replacement device");
        };
        let accepted_channels = attribute_u32(&accepted, &MF_MT_AUDIO_NUM_CHANNELS)
            .filter(|&value| value as usize == channels);
        let accepted_rate = attribute_u32(&accepted, &MF_MT_AUDIO_SAMPLES_PER_SECOND)
            .filter(|&value| value == rate);
        let accepted_float = attribute_guid(&accepted, &MF_MT_SUBTYPE)
            .is_some_and(|subtype| subtype == MFAudioFormat_Float);
        if !accepted_float || accepted_channels.is_none() || accepted_rate.is_none() {
            return self.rebind_failed("the source refused the replacement device's PCM layout");
        }

        let previous = std::mem::replace(&mut self.core, Arc::new(AudioCore::new(channels, rate)));
        self.core.carry_over(&previous);
        // The media type change re-bases the stamps the reader delivers, just
        // as a fragment join does: map the next block onto the play head
        // rather than trusting a stamp that may start again at zero. Without
        // this the first block after the switch lands at the wrong media time,
        // and the sound stays silent for the rest of the session.
        self.span_first = None;
        self.span_head = self.core.position();
        self.channels = channels;
        self.ended = false;
        self.scratch.clear();
        crate::diag(
            "INFO",
            "mf_audio.rebind",
            &format!("the source now produces {channels} channel(s) at {rate} Hz"),
        );
        true
    }

    /// Records a rebind failure once per run of failures, then leaves the flag
    /// set so a later video read tries again.
    fn rebind_failed(&mut self, reason: &'static str) -> bool {
        if !self.rebind_warned {
            self.rebind_warned = true;
            crate::diag("WARN", "mf_audio.rebind", reason);
        }
        false
    }

    /// Whether the live stream is still bound to the system default output.
    ///
    /// The device callback is the fast path, but it is not guaranteed to fire:
    /// Windows can swap the default while a stream is quiet and then never send
    /// `StreamInvalidated`, leaving sound on an endpoint nobody hears. Checking
    /// on a video read makes the switch direct. A device that is present but
    /// cannot state a config yet is treated as a route in flux, so the rebind
    /// retries instead of assuming the old device is still correct.
    fn on_default_device(&self) -> bool {
        let Some((device, config)) = output_device_with_config() else {
            return false;
        };
        if self
            .bound_device
            .as_ref()
            .is_some_and(|bound| &device != bound)
        {
            return false;
        }
        (
            config.channels() as usize,
            config.sample_rate(),
            config.sample_format(),
        ) == self.bound_layout
    }

    /// Reads decoded PCM up to a little past the play head. Called once per
    /// video read, on the decode thread.
    pub(super) fn pump(&mut self, reader: &IMFSourceReader) {
        // Proactive route check. Do not wait for a callback that may never come.
        // Throttled to the same cooldown as a rebind, so a steady stream pays
        // one cheap device query per half second and no stream rebuild.
        let now = Instant::now();
        if self
            .last_route_check
            .is_none_or(|at| now.duration_since(at) >= REBIND_COOLDOWN)
        {
            self.last_route_check = Some(now);
            if !self.on_default_device() {
                self.device_changed.store(true, Ordering::Release);
            }
        }
        // Follow a default-output change on a video read rather than on the
        // device thread: rebuilding a stream can block, and the decode thread
        // is the only place that may touch the audio state. A failed attempt
        // keeps the flag set and is retried after a short cooldown, which also
        // covers a new device that appears a moment after the old one goes.
        if self.device_changed.load(Ordering::Acquire) {
            let now = Instant::now();
            let due = self
                .last_rebind
                .is_none_or(|at| now.duration_since(at) >= REBIND_COOLDOWN);
            if due {
                self.last_rebind = Some(now);
                // Clear the flag before rebuilding: a change that arrives while
                // the new stream is being built must stay flagged for the next
                // attempt, instead of being wiped by this attempt's success and
                // leaving the sound on a device the user just left. A failed
                // attempt re-arms it so the retry below still runs.
                self.device_changed.store(false, Ordering::Release);
                if !self.rebind(reader) {
                    self.device_changed.store(true, Ordering::Release);
                }
            }
        }
        if self.ended {
            return;
        }
        let target = self.core.position().saturating_add(LEAD);
        // A time budget, not a sample count: a decoder that is quick fills the
        // whole lead in one call, while one that is slow still returns to the
        // picture. Steady state checks the target and returns immediately.
        let deadline = Instant::now() + PUMP_BUDGET;
        loop {
            if self
                .core
                .buffer
                .lock()
                .filled_until(self.channels)
                .is_some_and(|end| end >= target)
            {
                return;
            }
            if Instant::now() >= deadline {
                return;
            }

            let mut flags = 0u32;
            let mut timestamp = 0i64;
            let mut sample: Option<IMFSample> = None;
            // SAFETY: the out pointers refer to locals the call fills, and the
            // sample is released as the loop moves on.
            let result = unsafe {
                reader.ReadSample(
                    audio(),
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )
            };
            // Only a real end-of-stream is terminal. A transient reader error
            // used to latch `ended` too, and because nothing but `seek()`
            // cleared it, the PCM could never be refilled while the video clock
            // kept running: sound disappeared for the rest of the session.
            // Returning without latching lets the next video read retry the
            // audio read, inside this pump's existing wall-clock budget, with
            // no resampling and no change to the media timeline.
            if is_terminal_audio_eof(flags) {
                self.ended = true;
                return;
            }
            if result.is_err() || is_recoverable_audio_flags(flags) {
                return;
            }
            if flags & (MF_SOURCE_READERF_STREAMTICK.0 as u32) != 0 {
                // A discontinuity (an HLS fragment join) re-bases the sample
                // stamps, but the callback reads one continuous run of media
                // time. Continue the new span where the buffered PCM ends, not
                // at the drifted play head, so no gap or overlap is heard at the
                // join. Keep a sample delivered with the tick instead of dropping it.
                let buffered_end = self.core.buffer.lock().filled_until(self.channels);
                self.span_first = None;
                self.span_head = tick_span_head(buffered_end, self.core.position());
                if sample.is_none() {
                    continue;
                }
            }
            let Some(sample) = sample else {
                // No sample ready yet; the next video read will try again.
                return;
            };

            let ticks = unsafe { sample.GetSampleTime() }.ok().unwrap_or(timestamp);
            let stamp = Duration::from_nanos(u64::try_from(ticks).unwrap_or(0).saturating_mul(100));
            let (pts, first) = span_pts(stamp, self.span_first, self.span_head);
            self.span_first = Some(first);

            if !copy_floats(&sample, &mut self.scratch) {
                continue;
            }
            self.core
                .buffer
                .lock()
                .push(pts, &self.scratch, self.channels);
            if !self.logged_sample {
                self.logged_sample = true;
                crate::diag(
                    "INFO",
                    "mf_audio.sample",
                    &format!(
                        "the first decoded block of {} float sample(s) was buffered at {pts:?}",
                        self.scratch.len()
                    ),
                );
            }
        }
    }

    pub(super) fn play(&mut self) {
        // A previous end-of-stream must not survive a play after pause: the
        // reader is still open and a resumed session may have more to deliver.
        // A genuinely exhausted reader reports the end again on the next pump,
        // so nothing is heard that the source does not have.
        self.ended = false;
        self.core.clock.lock().play();
    }

    pub(super) fn pause(&self) {
        self.core.clock.lock().pause();
    }

    pub(super) fn set_speed(&self, speed: f32) {
        self.core.clock.lock().set_speed(speed);
    }

    /// Starts a new span of media: clears the old PCM and moves the play head.
    /// The reader itself is seeked by the video half, which seeks all streams.
    pub(super) fn seek(&mut self, position: Duration) {
        self.core.clock.lock().seek(position);
        self.core.buffer.lock().reset();
        for gate in self.core.gates.lock().iter_mut() {
            gate.reset();
        }
        self.span_first = None;
        self.span_head = position;
        self.ended = false;
    }

    /// Re-anchors the audio play head to the picture the player just published.
    pub(super) fn sync_to(&self, position: Duration) {
        // Start only once enough sound is decoded to keep the device fed; the
        // next picture tries again. At the very end of a track, start with what
        // is left rather than never starting at all. The buffer guard is
        // dropped before the clock is locked, so the two locks never nest in
        // the opposite order from the device callback.
        let filled = self.core.buffer.lock().filled_until(self.channels);
        let ready = start_ready(self.ended, filled, position);
        self.core.clock.lock().sync_to(position, ready);
    }

    pub(super) fn set_muted(&self, muted: bool) {
        self.core.muted.store(muted, Ordering::Relaxed);
    }

    pub(super) fn set_volume(&self, volume: f32) {
        self.core.volume.store(volume.to_bits(), Ordering::Relaxed);
    }
}

/// Asks the audio stream for interleaved 32-bit float PCM at the device's rate
/// and channel count, and returns the type Media Foundation accepted.
fn request_float(reader: &IMFSourceReader, channels: u32, rate: u32) -> Option<IMFMediaType> {
    // SAFETY: the type is created here, filled with constant attribute keys and
    // handed to the reader, which copies it.
    unsafe {
        let output = MFCreateMediaType().ok()?;
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).ok()?;
        output.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float).ok()?;
        output.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels).ok()?;
        output
            .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)
            .ok()?;
        output.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 32).ok()?;
        let block = channels.saturating_mul(size_of::<f32>() as u32);
        output.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block).ok()?;
        output
            .SetUINT32(
                &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                block.saturating_mul(rate),
            )
            .ok()?;
        output.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1).ok()?;
        reader.SetCurrentMediaType(audio(), None, &output).ok()?;
        reader.GetCurrentMediaType(audio()).ok()
    }
}

fn attribute_u32(media_type: &IMFMediaType, key: &GUID) -> Option<u32> {
    // SAFETY: the key is a constant and the value is an unsigned integer.
    unsafe { media_type.GetUINT32(key).ok() }
}

fn attribute_guid(media_type: &IMFMediaType, key: &GUID) -> Option<GUID> {
    // SAFETY: the key is a constant and the value is a GUID.
    unsafe { media_type.GetGUID(key).ok() }
}

/// Copies an audio sample's float PCM into `out`. Returns false when the sample
/// holds no usable floats.
fn copy_floats(sample: &IMFSample, out: &mut Vec<f32>) -> bool {
    // SAFETY: the buffer is a contiguous allocation handed out for the length of
    // the copy and unlocked on every path.
    unsafe {
        let Ok(buffer) = sample.ConvertToContiguousBuffer() else {
            return false;
        };
        let mut data: *mut u8 = ptr::null_mut();
        let mut length = 0u32;
        if buffer.Lock(&mut data, None, Some(&mut length)).is_err() || data.is_null() {
            return false;
        }
        let usable = (length as usize) / size_of::<f32>();
        if usable > 0 {
            let floats = slice::from_raw_parts(data as *const f32, usable);
            out.clear();
            out.extend_from_slice(floats);
        }
        let _ = buffer.Unlock();
        usable > 0
    }
}

/// Fills one output buffer from the PCM, starting exactly where the previous
/// callback stopped.
fn render(data: &mut Data, core: &AudioCore, format: SampleFormat) {
    let channels = core.channels;
    if channels == 0 {
        data.bytes_mut().fill(0);
        return;
    }
    let state = core.clock.lock().render_state();
    let volume = if core.muted.load(Ordering::Relaxed) {
        0.0
    } else {
        f32::from_bits(core.volume.load(Ordering::Relaxed))
    };
    if !core.rendering.swap(true, Ordering::Relaxed) {
        crate::diag(
            "INFO",
            "mf_audio.render",
            "the output device started pulling PCM",
        );
    }

    // The cursor is the device's own place in the media. It follows the clock
    // only when the clock moves deliberately or the two have wandered apart, so
    // scheduling jitter cannot repeat or skip a sample in the middle of a wave.
    let mut cursor = core.cursor.lock();
    let position = if state.silent {
        cursor.media = state.position;
        cursor.epoch = state.epoch;
        state.position
    } else {
        let wandered = cursor.media.abs_diff(state.position) > CURSOR_DRIFT;
        if cursor.epoch != state.epoch || wandered {
            cursor.media = state.position;
            cursor.epoch = state.epoch;
        }
        cursor.media
    };

    let mut buffer = core.buffer.lock();
    buffer.prune(position.saturating_sub(TAIL), channels);
    let mut frame = core.frame.lock();
    if frame.len() != channels {
        frame.resize(channels, 0.0);
    }
    let mut gates = core.gates.lock();
    if gates.len() != channels {
        gates.clear();
        gates.extend((0..channels).map(|_| DeClick::new()));
    }
    let rate = f64::from(core.rate);
    let frame = &mut *frame;
    let gates = &mut *gates;

    let filled = match format {
        SampleFormat::I8 => fill::<i8>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::I16 => fill::<i16>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::I24 => fill::<cpal::I24>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::I32 => fill::<i32>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::I64 => fill::<i64>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::U8 => fill::<u8>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::U16 => fill::<u16>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::U24 => fill::<cpal::U24>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::U32 => fill::<u32>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::U64 => fill::<u64>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::F32 => fill::<f32>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        SampleFormat::F64 => fill::<f64>(
            data,
            position,
            state.speed,
            state.silent,
            volume,
            &mut buffer,
            frame,
            gates,
            rate,
            channels,
        ),
        _ => {
            data.bytes_mut().fill(0);
            0
        }
    };
    if filled > 0 {
        cursor.media = advance_cursor(position, state.speed, core.rate, filled);
        cursor.epoch = state.epoch;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the device buffer, the PCM it reads and the state it renders are all distinct"
)]
fn fill<T>(
    data: &mut Data,
    position: Duration,
    speed: f64,
    silent: bool,
    volume: f32,
    buffer: &mut PcmBuffer,
    frame: &mut [f32],
    gates: &mut [DeClick],
    rate: f64,
    channels: usize,
) -> usize
where
    T: SizedSample + cpal::FromSample<f32>,
{
    let Some(out) = data.as_slice_mut::<T>() else {
        return 0;
    };
    let frames = out.len() / channels;
    for index in 0..frames {
        let base = index * channels;
        let values = &mut out[base..][..channels];
        if silent {
            // Zero, not the held sample: the head stands still and the room
            // must hear nothing rather than a tone stuck on one sample. The
            // gate fades whatever was last heard down to zero first.
            for (channel, slot) in values.iter_mut().enumerate() {
                *slot = T::from_sample(gates[channel].sample(0.0, false));
            }
            continue;
        }
        let media = media_time(position, speed, rate, index);
        let present = buffer
            .start
            .is_some_and(|start| frame_at(&buffer.samples, start, buffer.rate, media, frame));
        for (channel, (slot, value)) in values.iter_mut().zip(frame.iter()).enumerate() {
            let gate = &mut gates[channel.min(gates.len().saturating_sub(1))];
            let sample = if present { value * volume } else { 0.0 };
            *slot = T::from_sample(gate.sample(sample, present));
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_ended_buffer_does_not_start_a_silent_clock() {
        let position = Duration::from_secs(3);
        let full = position + START_LEAD;
        assert!(
            start_ready(false, Some(full), position),
            "a full lead starts"
        );
        assert!(
            !start_ready(false, Some(position + START_LEAD / 2), position),
            "a partial lead waits for the next pump"
        );
        assert!(
            !start_ready(true, None, position),
            "EOF with no PCM must not launch a running silent clock"
        );
        assert!(
            start_ready(true, Some(position + Duration::from_millis(20)), position),
            "the real tail may still play at EOF"
        );
    }

    #[test]
    fn only_end_of_stream_is_terminal_for_audio() {
        let eof = MF_SOURCE_READERF_ENDOFSTREAM.0 as u32;
        let error = MF_SOURCE_READERF_ERROR.0 as u32;
        assert!(is_terminal_audio_eof(eof), "a real EOF still ends the span");
        assert!(
            !is_terminal_audio_eof(error),
            "a reader error must not latch the span as ended"
        );
        assert!(
            is_recoverable_audio_flags(error),
            "a reader error stays retryable on the next video read"
        );
        assert!(
            !is_recoverable_audio_flags(eof),
            "EOF is not the transient-error path"
        );
    }

    #[test]
    fn frame_index_selects_the_sample_whose_time_contains_pts() {
        // At 48 kHz one sample lasts 20_833.333 ns. A point just before the
        // second sample must still read the first one; rounding would skip it.
        assert_eq!(
            frame_index(Duration::ZERO, Duration::from_nanos(20_832), 48_000),
            0
        );
        assert_eq!(
            frame_index(Duration::ZERO, Duration::from_nanos(20_834), 48_000),
            1
        );
        assert_eq!(
            frame_index(Duration::ZERO, Duration::from_millis(1), 48_000),
            48
        );
        // An hour in, f64 arithmetic can no longer address single samples.
        let start = Duration::from_secs(3600);
        assert_eq!(
            frame_index(start, start + Duration::from_millis(1), 192_000),
            192
        );
    }

    /// Moves the test clock's wall anchor back, so time passes without a sleep.
    fn elapse(clock: &mut MirrorClock, by: Duration) {
        clock.anchor = clock
            .anchor
            .checked_sub(by)
            .expect("the test clock has that much history");
    }

    #[test]
    fn declick_fades_gaps_and_ramps_back_in() {
        let mut gate = DeClick::new();
        let first = gate.sample(1.0, true);
        assert!(
            (first - 1.0 / 128.0).abs() < 1e-6,
            "the first sample ramps in: {first}"
        );
        for _ in 0..127 {
            gate.sample(1.0, true);
        }
        let steady = gate.sample(1.0, true);
        assert_eq!(steady, 1.0);

        let gap1 = gate.sample(0.0, false);
        assert!(
            gap1 > 0.0 && gap1 < steady,
            "a gap fades instead of clicking: {gap1}"
        );
        let gap2 = gate.sample(0.0, false);
        assert!(gap2 < gap1, "a longer gap keeps fading: {gap2}");
        let resumed = gate.sample(1.0, true);
        assert!(
            resumed > 0.0 && resumed < 0.2,
            "sound ramps back in from silence rather than jumping to half level: {resumed}"
        );
    }

    #[test]
    fn sound_waits_for_the_first_picture_and_starts_at_its_time() {
        let mut clock = MirrorClock::new();
        clock.play();
        assert!(
            clock.render_state().silent,
            "no sound before a picture states where it belongs"
        );
        assert_eq!(clock.position(), Duration::ZERO);

        // The first picture of a network source arrives 220 ms after play. The
        // sound starts there instead of 220 ms ahead of the picture.
        clock.sync_to(Duration::from_millis(220), true);
        let state = clock.render_state();
        assert!(!state.silent);
        assert_eq!(state.speed, 1.0);
        assert!(
            state.position >= Duration::from_millis(220)
                && state.position < Duration::from_millis(230),
            "sound begins at the picture: {:?}",
            state.position
        );
    }

    #[test]
    fn sound_keeps_waiting_until_enough_is_buffered() {
        let mut clock = MirrorClock::new();
        clock.play();
        clock.sync_to(Duration::from_millis(100), false);
        assert!(
            clock.render_state().silent,
            "a thin buffer must not start and stutter"
        );
        clock.sync_to(Duration::from_millis(100), true);
        assert!(!clock.render_state().silent);
    }

    #[test]
    fn a_prime_with_no_buffer_stops_waiting_after_a_deadline() {
        let mut clock = MirrorClock::new();
        clock.play();
        clock.sync_to(Duration::from_millis(100), false);
        assert!(clock.render_state().silent, "a starved prime starts silent");

        // Move the start of the wait into the past without sleeping.
        let started = clock.wait_started.expect("play started a prime wait");
        clock.wait_started = Some(started.checked_sub(PRIME_TIMEOUT).expect("a real deadline"));
        clock.sync_to(Duration::from_millis(100), false);
        let state = clock.render_state();
        assert!(
            !state.silent,
            "a buffer that never primes must not mute the sound forever"
        );
        assert!(
            state.position < Duration::from_millis(110),
            "the expired wait must not be heard as a jump: {:?}",
            state.position
        );
    }

    #[test]
    fn a_picture_within_the_dead_band_does_not_change_the_rate() {
        let mut clock = MirrorClock::new();
        clock.play();
        clock.sync_to(Duration::from_millis(100), true);
        elapse(&mut clock, Duration::from_millis(50));
        // The picture is 20 ms behind the head, inside the dead band.
        clock.sync_to(Duration::from_millis(130), true);
        let state = clock.render_state();
        assert!(!state.silent);
        assert_eq!(state.speed, 1.0, "sync never resamples the sound");
        assert!(
            state.position >= Duration::from_millis(150),
            "a normal frame jitter leaves the head alone: {:?}",
            state.position
        );
    }

    #[test]
    fn a_late_picture_holds_the_head_with_silence_instead_of_slowing_it() {
        let mut clock = MirrorClock::new();
        clock.play();
        clock.sync_to(Duration::ZERO, true);
        elapse(&mut clock, Duration::from_millis(250));
        // The picture is 150 ms behind the head.
        clock.sync_to(Duration::from_millis(100), true);
        let state = clock.render_state();
        assert!(state.silent, "sound waits in silence for the picture");
        let held = state.position;
        assert!(
            held >= Duration::from_millis(250) && held < Duration::from_millis(251),
            "the hold starts at the head, where the picture has not reached: {held:?}"
        );
        assert_eq!(state.speed, 1.0, "the rate never changes");

        // While the hold runs, the head stands still rather than repeating a
        // sample: the callback renders zeroes for this stretch.
        elapse(&mut clock, Duration::from_millis(80));
        assert_eq!(clock.position(), held);

        // The picture reaches the head; sound resumes at the same sample.
        clock.sync_to(Duration::from_millis(250), true);
        let state = clock.render_state();
        assert!(!state.silent);
        assert_eq!(state.speed, 1.0);
        assert!(
            state.position >= Duration::from_millis(250)
                && state.position < Duration::from_millis(260),
            "resuming does not rewind or skip: {:?}",
            state.position
        );
    }

    #[test]
    fn a_picture_ahead_moves_the_head_forward_once_without_resampling() {
        let mut clock = MirrorClock::new();
        clock.play();
        clock.sync_to(Duration::ZERO, true);
        elapse(&mut clock, Duration::from_millis(50));
        clock.sync_to(Duration::from_millis(500), true);
        let state = clock.render_state();
        assert!(!state.silent);
        assert_eq!(state.speed, 1.0);
        assert!(
            state.position >= Duration::from_millis(500),
            "a forward correction lands on the picture: {:?}",
            state.position
        );
    }

    #[test]
    fn a_seek_waits_for_the_picture_it_lands_on() {
        let mut clock = MirrorClock::new();
        clock.play();
        clock.sync_to(Duration::from_millis(100), true);
        clock.seek(Duration::from_secs(30));
        assert!(
            clock.render_state().silent,
            "a seek has no sound until its picture arrives"
        );
        clock.sync_to(Duration::from_secs(30), false);
        assert!(clock.render_state().silent, "a thin buffer keeps waiting");
        clock.sync_to(Duration::from_secs(30), true);
        let state = clock.render_state();
        assert!(!state.silent);
        assert_eq!(state.speed, 1.0);
        assert!(state.position >= Duration::from_secs(30));
    }

    #[test]
    fn a_seek_reanchors_pcm_to_the_play_head() {
        // Absolute timestamps after a seek: the first sample is at the seek
        // point, and later samples keep their distance from it.
        let head = Duration::from_secs(60);
        let (pts, first) = span_pts(Duration::from_secs(60), None, head);
        assert_eq!(
            pts, head,
            "the first sample of a span starts at the play head"
        );
        let (pts2, first2) = span_pts(Duration::from_millis(60_020), Some(first), head);
        assert_eq!(pts2, head + Duration::from_millis(20));
        assert_eq!(first2, Duration::from_secs(60));

        // Reset timestamps after a seek: the first sample is stamped zero, but
        // the head is still at 60s, so the buffer must start at 60s too.
        let (pts, _) = span_pts(Duration::ZERO, None, head);
        assert_eq!(
            pts, head,
            "a restarted timestamp is mapped onto the head too"
        );

        // Opening a file: the head is zero and the first sample carries the
        // processor delay, which is removed as before.
        let (pts, _) = span_pts(Duration::from_millis(280), None, Duration::ZERO);
        assert_eq!(pts, Duration::ZERO);
    }

    #[test]
    fn consecutive_callbacks_are_exactly_one_buffer_apart() {
        let rate: u32 = 48_000;
        let frames = 480usize; // 10 ms of audio at 48 kHz
        let head = Duration::from_secs(3);
        let one = Duration::from_nanos(1_000_000_000 / u64::from(rate));

        // At normal speed the step is integer nanoseconds, so even hours of
        // callbacks cannot accumulate rounding drift.
        let next = advance_cursor(head, 1.0, rate, frames);
        assert_eq!(next - head, Duration::from_millis(10));
        let last = media_time(head, 1.0, f64::from(rate), frames - 1);
        let first_next = media_time(next, 1.0, f64::from(rate), 0);
        assert_eq!(
            first_next - last,
            one,
            "the next callback starts exactly one sample after the last: {last:?} -> {first_next:?}"
        );

        // At other speeds the head steps fractionally; the boundary still
        // neither repeats nor skips a whole frame beyond the speed itself.
        let next = advance_cursor(head, 2.0, rate, frames);
        assert_eq!(next - head, Duration::from_millis(20));
        let last = media_time(head, 2.0, f64::from(rate), frames - 1);
        let first_next = media_time(next, 2.0, f64::from(rate), 0);
        let expected = one * 2; // 2x reads two samples per output frame
        let gap = first_next - last;
        assert!(
            gap.abs_diff(expected) <= Duration::from_nanos(2),
            "2x boundary: {last:?} -> {first_next:?} (gap {gap:?}, expected {expected:?})"
        );
    }

    #[test]
    fn the_sample_reader_steps_at_the_playback_speed() {
        let rate = 48_000.0;
        let head = Duration::from_secs(3);
        let one = Duration::from_secs_f64(1.0 / rate);

        // At normal speed every output frame reads a genuine consecutive
        // sample: nothing is interpolated, so the pitch is the recorded one.
        let a = media_time(head, 1.0, rate, 0);
        let b = media_time(head, 1.0, rate, 1);
        assert_eq!(a, head);
        let step = b - a;
        assert!(
            step.abs_diff(one) <= Duration::from_nanos(1),
            "one frame advances one sample at 1x: {step:?}"
        );

        // A speed control changes how fast samples are consumed, and with it
        // the pitch: that is the honest tape-style behaviour of the control
        // itself, never a sync correction layered under normal playback.
        let half = media_time(head, 0.5, rate, 1) - media_time(head, 0.5, rate, 0);
        assert!(
            half.abs_diff(one / 2) <= Duration::from_nanos(1),
            "half speed steps half a sample per frame: {half:?}"
        );
    }

    #[test]
    fn a_stream_tick_continues_the_pcm_instead_of_jumping_to_the_head() {
        let rate: u32 = 48_000;
        let channels = 2;
        let block = vec![0.5f32; 100 * (rate as usize / 1000) * channels]; // 100 ms, interleaved
        let mut buffer = PcmBuffer::new(rate);
        buffer.push(Duration::ZERO, &block, channels);
        let joined_at = buffer
            .filled_until(channels)
            .expect("audio before the join");

        // At the fragment join MF restarts the stamp at zero and raises a stream
        // tick. The play head has drifted well past the fragment boundary.
        let drifted_head = joined_at + Duration::from_millis(300);
        let head = tick_span_head(buffer.filled_until(channels), drifted_head);
        assert_eq!(
            head, joined_at,
            "the new span must continue the buffered audio, not jump to the drifted head"
        );

        let (pts, first_stamp) = span_pts(Duration::ZERO, None, head);
        assert_eq!(
            pts, joined_at,
            "the first sample after the tick starts at the buffer end"
        );
        assert_eq!(first_stamp, Duration::ZERO);

        let second = vec![0.25f32; block.len()];
        buffer.push(pts, &second, channels);
        let after = buffer.filled_until(channels).expect("audio after the join");
        let expected = joined_at + Duration::from_millis(100);
        assert!(
            after <= expected + Duration::from_millis(1),
            "the join must not open a gap: {joined_at:?} -> {after:?}"
        );
    }

    #[test]
    fn a_huge_timestamp_gap_is_bounded_instead_of_allocating_forever() {
        let rate: u32 = 48_000;
        let channels = 2;
        let mut buffer = PcmBuffer::new(rate);
        buffer.push(Duration::ZERO, &vec![0.5; channels * 2], channels);
        // A corrupt stamp a day ahead must not try to insert a day of silence.
        let far = Duration::from_secs(86_400);
        buffer.push(far, &vec![0.5; channels * 2], channels);
        assert_eq!(
            buffer.filled_until(channels),
            Some(far + Duration::from_secs_f64(2.0 / rate as f64)),
            "the buffer restarts at the stamp rather than filling the huge gap"
        );
        assert_eq!(
            buffer.samples.len(),
            channels * 2,
            "no silence was materialised"
        );
    }

    #[test]
    fn only_device_change_errors_trigger_a_rebind() {
        // A new default and a vanished default both need the device looked up
        // again; busy/permission/other errors must not start a rebind loop.
        assert!(is_device_change(&cpal::Error::new(
            cpal::ErrorKind::StreamInvalidated
        )));
        assert!(is_device_change(&cpal::Error::new(
            cpal::ErrorKind::DeviceNotAvailable
        )));
        assert!(!is_device_change(&cpal::Error::new(
            cpal::ErrorKind::DeviceBusy
        )));
        assert!(!is_device_change(&cpal::Error::new(
            cpal::ErrorKind::PermissionDenied
        )));
        assert!(!is_device_change(&cpal::Error::new(cpal::ErrorKind::Other)));
    }

    #[test]
    fn a_rebind_plan_renegotiates_instead_of_leaving_sound_dead() {
        let current = (2, 48_000, SampleFormat::F32);
        // The exact layout only moves the stream: no conversion, no pitch change.
        assert_eq!(rebind_plan(current, current), RebindPlan::Keep);
        // Another sample format plays the same float PCM through the callback's
        // own writer, so nothing has to be negotiated again.
        assert_eq!(
            rebind_plan(current, (2, 48_000, SampleFormat::I16)),
            RebindPlan::Reformat
        );
        // Another rate or channel count is the common headset switch. Refusing
        // it left the stream bound to the invalidated endpoint, which is silence
        // on both devices until the source changed; it must renegotiate instead.
        assert_eq!(
            rebind_plan(current, (2, 44_100, SampleFormat::F32)),
            RebindPlan::Renegotiate
        );
        assert_eq!(
            rebind_plan(current, (6, 48_000, SampleFormat::F32)),
            RebindPlan::Renegotiate
        );
        assert_eq!(
            rebind_plan(current, (1, 16_000, SampleFormat::I16)),
            RebindPlan::Renegotiate
        );
    }

    #[test]
    fn a_layout_change_keeps_the_play_head_and_the_controls() {
        let previous = AudioCore::new(2, 48_000);
        previous.clock.lock().seek(Duration::from_secs(125));
        previous.muted.store(true, Ordering::Relaxed);
        previous.volume.store(0.25_f32.to_bits(), Ordering::Relaxed);

        // The replacement device wants mono at 44.1 kHz, so the reader will be
        // renegotiated and the sound moves to a fresh core.
        let replacement = AudioCore::new(1, 44_100);
        replacement.carry_over(&previous);

        assert_eq!(
            replacement.position(),
            Duration::from_secs(125),
            "a device switch must not restart the sound"
        );
        assert!(replacement.muted.load(Ordering::Relaxed));
        assert_eq!(
            f32::from_bits(replacement.volume.load(Ordering::Relaxed)),
            0.25
        );
    }

    #[test]
    fn a_busy_output_device_is_retryable_but_a_silent_source_is_not() {
        // [SOURCE-VERIFIED] SetupError drives whether the decode thread keeps
        // scheduling recovery. A silent clip settles immediately; a device that
        // was busy during a switch must come back without a new open.
        assert!(SetupError::Unavailable("device busy").retryable());
        assert!(!SetupError::NoAudio("no stream").retryable());
    }

    #[test]
    fn a_headset_layout_mismatch_stays_retryable() {
        // [ROOT CAUSE] Switching to a headset at a different rate used to be
        // classified as NoAudio, which set source_has_audio = false in the
        // caller and stopped recovery forever: silence on both devices.
        let mismatch = negotiation_ok(false, true, true).unwrap_err();
        assert!(
            mismatch.retryable(),
            "a layout the current device cannot play must be retried for the next one"
        );
        assert!(negotiation_ok(true, false, true).unwrap_err().retryable());
        assert!(negotiation_ok(true, true, false).unwrap_err().retryable());
        assert!(
            negotiation_ok(true, true, true).is_ok(),
            "an exact PCM match needs no retry"
        );
    }
}
