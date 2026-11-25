//! The decode thread: it reads ahead of the clock and publishes the newest
//! picture.

use std::{
    any::Any,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use super::{Command, DEFAULT_FRAME, Shared, Slot, presentation_time};
#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
use crate::PlaybackSession;
use crate::{PixelFormat, VideoDecoder, VideoError, VideoFrame, time_wait::FineWait};

/// The longest single sleep while a picture waits for its time, so a command
/// reaches the thread promptly.
const SLICE: Duration = Duration::from_millis(10);

/// Near a frame deadline a sleep can round up to a whole operating-system timer
/// tick, which at 60 fps costs another frame. The final slice therefore uses a
/// precise wait instead of a coarse sleep.
const FINE_WAIT: Duration = Duration::from_millis(16);

/// The furthest a picture may be scheduled ahead of the clock before the thread
/// stops waiting for it. A damaged container can state a time hours away, and a
/// player that waits for it shows nothing at all.
const HORIZON: Duration = Duration::from_secs(2);

/// Whether a picture is so far behind the play head that the clock should be
/// re-anchored to it rather than treated as on time. One frame of slack is
/// normal — the clock is sampled between reads — so a picture is only late past
/// two frame times.
fn is_late(now: Duration, pts: Duration, rate: Option<crate::FrameRate>) -> bool {
    let tolerance = rate
        .and_then(|rate| rate.frame_duration())
        .unwrap_or(DEFAULT_FRAME)
        .saturating_mul(2);
    now.saturating_sub(pts) > tolerance
}

/// The readable part of a panic payload, for the diagnostic record.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "a non-text panic payload".to_owned()
    }
}

/// The thread that owns the platform decoder.
pub(crate) struct Engine {
    commands: Sender<Command>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    pub(crate) fn start(
        decoder: Box<dyn VideoDecoder>,
        shared: Arc<Shared>,
        slot: Arc<Mutex<Slot>>,
    ) -> Result<Self, VideoError> {
        let (commands, inbox) = mpsc::channel();
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        let session = shared.playback_session();
        let worker = Worker {
            inbox,
            decoder,
            shared,
            slot,
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            session,
            scratch: Vec::new(),
            fine_wait: FineWait::new(),
            planes: Vec::new(),
            last_pts: None,
            adopted: false,
            native_refused: false,
            ended: false,
            refresh: true,
            seeked_to: None,
            generation: 0,
        };
        let handle = thread::Builder::new()
            .name("cranpose-video".to_owned())
            .spawn(move || worker.run())
            .map_err(|error| {
                VideoError::Source(format!("cannot start the decode thread: {error}"))
            })?;
        Ok(Self {
            commands,
            thread: Mutex::new(Some(handle)),
        })
    }

    pub(crate) fn dispatch(&self, command: Command) -> Result<(), VideoError> {
        self.commands.send(command).map_err(|_| VideoError::Closed)
    }

    /// Nothing to do: the thread decodes on its own.
    pub(crate) fn pump(&self) {}

    pub(crate) fn shutdown(&mut self) {
        // Keep both signals. The flag is raised by `Shared::begin_shutdown`
        // before this call, so a decoder that observes it can leave a blocking
        // read; the command gives a deterministic exit between reads. Dropping
        // the handle detaches the worker instead of waiting for it: a decoder
        // stuck inside `read()` must not hold `close()` (or `Drop`) forever.
        // The worker owns the decoder and closes it when it finally returns;
        // the handle never owned either, so detaching cannot leak or double-drop
        // it. Events are sealed separately in `Shared::emit` by the same flag.
        let _ = self.commands.send(Command::Shutdown);
        let _ = self.thread.lock().take();
    }
}

struct Worker {
    inbox: Receiver<Command>,
    decoder: Box<dyn VideoDecoder>,
    shared: Arc<Shared>,
    slot: Arc<Mutex<Slot>>,
    /// The same recovery handle the decoder reports into, read on this thread
    /// so a device query or stream rebuild never touches the composition one.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    session: PlaybackSession,
    scratch: Vec<u8>,
    /// A precise short wait for the last slice before a frame deadline.
    fine_wait: FineWait,
    /// The buffer a planar decoder fills, kept so a picture costs no allocation.
    planes: Vec<u8>,
    last_pts: Option<Duration>,
    adopted: bool,
    ended: bool,
    /// Whether the decoder declared NV12 planes it cannot fill, so pictures are
    /// read through its RGBA path for the rest of the session.
    native_refused: bool,
    // A picture to decode and publish while the clock stands still: the first
    // one after opening, and the one a seek lands on.
    refresh: bool,
    // The position a seek asked for. A decoder lands on the key frame before
    // it, so the pictures between that key frame and the position are decoded
    // and not shown.
    seeked_to: Option<Duration>,
    // Counts the seeks, so a picture that waited for its time before one
    // landed is dropped rather than shown at the new position.
    generation: u64,
}

impl Worker {
    /// Keeps a decoder panic from silently killing the worker.
    ///
    /// The worker owns the only handle to the platform decoder. Without this
    /// boundary a panic inside a backend call unwound out of the thread, left
    /// the last published picture on screen and reported no failure: the feed
    /// looked frozen but the player claimed to be healthy. The panic now reaches
    /// the same state and event path as any other decode error, and the decoder
    /// is still closed on the way out. A panic while closing cannot escape the
    /// thread either.
    fn run(mut self) {
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| self.decode_loop()));
        if let Err(payload) = outcome {
            let message = panic_message(payload.as_ref());
            crate::diag("ERROR", "decode.panic", &message);
            self.shared.fail(VideoError::Decode(format!(
                "the decoder panicked: {message}"
            )));
        }
        if panic::catch_unwind(AssertUnwindSafe(|| self.decoder.close())).is_err() {
            crate::diag(
                "ERROR",
                "decode.panic",
                "the decoder panicked while closing",
            );
        }
    }

    fn decode_loop(&mut self) {
        loop {
            // Nothing is moving and nothing is pending, so this thread has
            // nothing to do and must cost nothing. It used to wake every 50 ms
            // to re-read the state, which on a paused player is twenty wake-ups
            // a second, on a desktop or a phone, for no picture at all. A feed
            // with several parked players multiplied that. Every state change
            // reaches this thread as a command (`play` sends `Play`, `seek`
            // sends `Seek`, `close` sends `Shutdown`) and the shutdown flag is
            // raised only by a close that then sends one, so blocking here
            // cannot miss a wake-up.
            // `close` raises this before joining, so the thread leaves even if
            // no command made it through the queue.
            if self.shared.is_shutting_down() {
                break;
            }
            let mut shutdown = false;
            while let Ok(command) = self.inbox.try_recv() {
                if !self.handle(command) {
                    shutdown = true;
                    break;
                }
            }
            if shutdown {
                break;
            }

            // A failed audio open, a lost device or a route change arms the
            // session's retry. Attempt it here, on the decode thread, so no
            // device enumeration or stream rebuild ever runs on the thread
            // that draws. The decoder decides what "recover" means: the
            // generic audio wrapper reopens its source, the Windows reader
            // rebinds its stream, and a decoder with no sound is a no-op.
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            self.maybe_recover();

            if self.shared.is_playing() && !self.ended {
                if !self.step(true) {
                    break;
                }
                continue;
            }

            if self.refresh {
                self.refresh = false;
                if !self.step(false) {
                    break;
                }
                continue;
            }

            // When a retry is armed, wake in time to attempt it; with no retry
            // pending, block exactly as before so a parked player costs
            // nothing. The cap keeps the worker responsive to commands even
            // if a retry is still a long way off.
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            let received = match self.session.next_retry() {
                Some(at) => self.inbox.recv_timeout(
                    at.saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(50)),
                ),
                None => self
                    .inbox
                    .recv()
                    .map_err(|_| RecvTimeoutError::Disconnected),
            };
            #[cfg(not(all(not(target_os = "linux"), not(target_arch = "wasm32"))))]
            let received = self
                .inbox
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected);
            match received {
                Ok(command) => {
                    if !self.handle(command) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    /// Attempts one due recovery and records its outcome.
    ///
    /// `begin_recovery` both checks the cooldown and numbers the attempt, so
    /// calling this on every pass of the loop cannot tighten the retry rate.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn maybe_recover(&mut self) {
        if self.shared.is_shutting_down() {
            return;
        }
        if self.session.begin_recovery().is_some() {
            let source_failure = self.shared.has_recoverable_failure();
            if self.decoder.recover() {
                self.session.recovered();
                // Always schedule a fresh read after a rebuild. `recover()` only
                // reopens the reader; it cannot itself prove that sound and
                // pictures came back. The previous gate (`source_failure` only)
                // left an iOS `Decode` failure stranded in `Failed` with the
                // worker parked in `recv()` — no picture, no retry, no exit.
                // `note_frame` clears the failure once a new frame is published.
                let _ = source_failure;
                // Arm the public Failed→Playing transition for backends whose
                // failure was non-transient (Apple/Android Decode errors). The
                // next published frame then clears the visible Failed state.
                self.shared.recovery_succeeded();
                self.refresh = true;
            } else {
                // Re-arm with the session's backoff. The decoder stays open
                // and the picture keeps playing; only sound tries again.
                self.session
                    .failed("audio.recover", "a recovery attempt did not restore audio");
            }
        }
    }

    /// Returns false when the player is going away.
    fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Play => self.decoder.play(),
            Command::Pause => self.decoder.pause(),
            Command::SetSpeed(speed) => self.decoder.set_speed(speed),
            Command::SetMuted(muted) => self.decoder.set_muted(muted),
            Command::SetVolume(volume) => self.decoder.set_volume(volume),
            Command::SetAudioGroup(group) => self.decoder.set_audio_group(group),
            Command::SelectAudioTrack(index) => self.decoder.set_audio_track(index),
            Command::SelectSubtitleTrack(index) => self.decoder.set_subtitle_track(index),
            Command::SetLoopRange(range) => self.decoder.set_loop_range(range),
            Command::Stop => {
                // A backend that owns a platform clock keeps it running until it
                // is told to stop. Seeking alone only moves its media time.
                self.decoder.pause();
                self.rewind(Duration::ZERO);
            }
            Command::Seek(position) => self.rewind(position),
            Command::Shutdown => return false,
        }
        true
    }

    fn rewind(&mut self, position: Duration) {
        self.ended = false;
        self.last_pts = None;
        self.refresh = true;
        self.seeked_to = Some(position);
        self.generation = self.generation.wrapping_add(1);
        if let Err(error) = self.decoder.seek(position) {
            // A live stream cannot seek back, and a network read can fail. The
            // state carries the reason, and a later seek clears it.
            self.shared.fail(error);
        }
    }

    /// Decodes one picture, waits for its time when the clock runs, and shows
    /// it.
    ///
    /// Returns false when the player is going away.
    fn step(&mut self, pace: bool) -> bool {
        loop {
            if self.shared.is_shutting_down() {
                return false;
            }
            // A decoder such as HLS learns live/seekable only after the playlist
            // is fetched on the first read. Keep the shared snapshot current on
            // every read so the live-empty-window retry gate sees the truth even
            // when the stream fails before its first picture. Both accessors are
            // cached field reads and the setter only writes two bools.
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            self.shared
                .adopt_capabilities(self.decoder.is_live(), self.decoder.is_seekable());
            // A network backend states how far it has filled its window; a file
            // backend says idle. Publishing it here keeps a spinner honest
            // without a second thread polling the decoder.
            let buffering = self.decoder.buffering();
            if !buffering.is_idle() || !self.shared.buffering().is_idle() {
                self.shared
                    .set_buffering(buffering.state(), buffering.fraction());
            }
            let native = !self.native_refused && self.decoder.output_format() == PixelFormat::Nv12;
            let mut buffer = std::mem::take(&mut self.scratch);
            let mut planes = std::mem::take(&mut self.planes);
            if native {
                if planes.is_empty() {
                    planes = self.slot.lock().spare_planes();
                }
            } else if buffer.is_empty() {
                buffer = self.slot.lock().spare();
            }
            let started = Instant::now();
            let read = if native {
                self.decoder.read_nv12(&mut planes)
            } else {
                self.decoder.read(&mut buffer)
            };
            let decode = started.elapsed();
            // `close()` may have happened while this read was in flight. Nothing
            // after it may publish a frame, move the clock, or record an event:
            // `Closed` is emitted by the caller as the last event of the session.
            // Returning false leaves the loop, and the worker still closes the
            // decoder it owns on the way out.
            if self.shared.is_shutting_down() {
                self.scratch = buffer;
                self.planes = planes;
                return false;
            }
            let frame = match read {
                Ok(Some(frame)) => frame,
                Ok(None) => {
                    self.scratch = buffer;
                    self.planes = planes;
                    // A live source that produced no picture after a rebuild
                    // has not actually recovered. Re-arm the same bounded
                    // retry instead of ending the feed or leaving it failed
                    // with no attempt waiting. A finite file may end here.
                    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
                    if self.shared.has_recoverable_failure() && self.shared.is_live() {
                        self.shared.fail(VideoError::Source(
                            "the rebuilt source produced no picture".into(),
                        ));
                        return true;
                    }
                    if self.shared.is_looping()
                        && self.shared.is_playing()
                        && self.shared.is_seekable()
                    {
                        // The source reached its end and repeats. Rewinding
                        // through the open decoder costs one seek; reopening the
                        // file would cost a read, a device setup and a fresh
                        // picture buffer, every time round. A repeating section
                        // goes back to its own start instead of the file's.
                        let target = self
                            .shared
                            .loop_range()
                            .map_or(Duration::ZERO, |range| range.start());
                        self.shared.set_position(target);
                        self.rewind(target);
                        self.shared.emit_looped(target);
                        continue;
                    }
                    self.ended = true;
                    // The media may have ended, but a platform clock the decoder
                    // owns has not. Leaving it running silently desynchronises
                    // the next play, and on Windows keeps the audio device busy.
                    self.decoder.pause();
                    self.shared.finish();
                    return true;
                }
                Err(VideoError::UnsupportedOperation { .. }) if native => {
                    // A decoder that declares NV12 and leaves `read_nv12` at its
                    // default cannot produce planes. Falling back to its RGBA
                    // read keeps such a backend playing instead of ending the
                    // source at the first picture.
                    self.native_refused = true;
                    self.shared.set_pixel_format(PixelFormat::Rgba8);
                    self.scratch = buffer;
                    self.planes = planes;
                    continue;
                }
                Err(error) => {
                    self.scratch = buffer;
                    self.planes = planes;
                    self.shared.fail(error);
                    return true;
                }
            };

            let expected = if native {
                frame.width as usize * frame.height as usize * 3 / 2
            } else {
                frame.width as usize * frame.height as usize * 4
            };
            let length = if native { planes.len() } else { buffer.len() };
            if length != expected {
                self.scratch = buffer;
                self.planes = planes;
                self.shared.fail(VideoError::Decode(format!(
                    "the decoder filled {length} bytes for a {}x{} frame",
                    frame.width, frame.height
                )));
                return true;
            }

            if !self.adopted {
                self.adopted = true;
                let info = self.decoder.info();
                if info.width > 0 && info.height > 0 {
                    self.shared.adopt_info(info);
                }
            }

            let pts = presentation_time(self.last_pts, frame.pts, self.decoder.frame_rate());
            if let Some(target) = self.seeked_to
                && self.before_the_seek(pts, target)
            {
                // The decoder started from the key frame before the position
                // asked for, and these pictures come before it, or a platform
                // still hands out the pictures it buffered before the seek.
                // Decoding on costs less than showing a picture from the
                // wrong second.
                self.scratch = buffer;
                self.planes = planes;
                self.last_pts = Some(pts);
                continue;
            }
            let late = self.shared.is_playing()
                && is_late(self.shared.position(), pts, self.decoder.frame_rate());
            if late {
                // The wall clock has run past this picture's time. This is the
                // normal state for a network source: the reader buffers before
                // handing out the first frame, and a decode can take longer than
                // one frame interval, so the clock leads the media. There is no
                // newer picture to skip to yet — this thread reads one frame at
                // a time — so dropping this one only makes the next one later
                // too. The old policy dropped up to MAX_LATE_SKIPS frames before
                // showing one; on a remote MP4 that meant a single stale picture
                // every four seconds while the clock raced on, which is the
                // "slow and distorted" playback users saw. Re-anchor the clock
                // (and, through sync_clock below, the paired audio) to this
                // picture instead and show it. Pictures from before a seek are
                // already filtered out by `before_the_seek` above.
                self.shared.set_position(pts);
                // A run of these is the signature of a decoder that cannot keep
                // up, so count and record them for the diagnostics snapshot.
                self.shared.note_reanchor(pts);
            }

            // A repeating section ends before the media does. The play head
            // passing the section's end is the whole trigger: rewind through the
            // open decoder rather than waiting for the file's own end, which for
            // a passage in the middle of a long file would never come.
            if let Some(range) = self.shared.loop_range()
                && self.shared.is_playing()
                && self.shared.is_seekable()
                && range.is_past_end(pts)
            {
                self.scratch = buffer;
                self.planes = planes;
                self.last_pts = None;
                self.shared.set_position(range.start());
                self.rewind(range.start());
                self.shared.emit_looped(range.start());
                continue;
            }

            let generation = self.generation;
            if pace && !self.wait_until(pts) {
                self.scratch = buffer;
                self.planes = planes;
                return false;
            }
            if generation != self.generation {
                // A seek landed while this picture waited for its time, and
                // the picture belongs to the position the player left.
                self.scratch = buffer;
                self.planes = planes;
                return true;
            }
            self.seeked_to = None;

            // A planar picture is converted here, on the decode thread, unless
            // the screen asked to sample the planes itself. Converting on the
            // thread that draws would put a millisecond between two window
            // frames, which is the stutter this path exists to remove.
            let published = if native {
                VideoFrame::from_nv12(
                    frame.width,
                    frame.height,
                    pts,
                    std::mem::take(&mut planes),
                    self.decoder.nv12_color(),
                )
            } else {
                VideoFrame::from_rgba8(frame.width, frame.height, pts, std::mem::take(&mut buffer))
            };
            let Some(mut published) = published else {
                self.scratch = buffer;
                self.planes = planes;
                // A decoder can describe a picture its buffer cannot hold — a
                // zero-sized frame, for example. Dropping it in silence left the
                // screen blank while the player reported no failure at all, so
                // report it as the decode failure it is, just as a length
                // mismatch is reported above.
                self.shared.fail(VideoError::Decode(format!(
                    "the decoder described an invalid {}x{} frame",
                    frame.width, frame.height
                )));
                return true;
            };
            let mut converted = Duration::ZERO;
            if native && !self.shared.native_nv12() {
                // The buffer comes out of the pool, so a picture costs neither a
                // fresh allocation nor a clear of eight megabytes: the kernel
                // overwrites every byte it writes.
                let mut rgba = self.slot.lock().spare();
                let started = Instant::now();
                published.convert_into(&mut rgba);
                converted = started.elapsed();
                if let Some(returned) = published.cache_rgba(rgba) {
                    buffer = returned;
                }
                // The planes have done their turn, so the same buffer decodes
                // the next picture.
                if let Some(returned) = published.release_planes() {
                    planes = returned;
                }
            }
            self.scratch = buffer;
            self.planes = planes;
            self.shared.note_decode(decode);
            if !converted.is_zero() {
                self.shared.note_convert(converted);
            }
            let published_at = Instant::now();
            self.slot.lock().publish(published);
            self.shared.note_frame();
            // Record the authoritative publication before anything that can
            // block (a command, a clock write, the decoder's own audio sync),
            // so a pacing probe measures the picture, not the probe's polling.
            self.shared.note_publication(published_at, pts);
            self.shared.set_position_monotonic(pts);
            // A backend that renders its own audio re-anchors its play head to
            // the same picture time, so sound and picture cannot drift apart.
            self.decoder.sync_clock(pts);
            self.last_pts = Some(pts);
            return true;
        }
    }

    /// Whether a picture read after a seek belongs to the position the player
    /// left rather than to the one it asked for.
    ///
    /// Only pictures that come *before* the position are stale.
    ///
    /// A decoder lands on a key frame at or before the position it is given, so
    /// the pictures between that key frame and the position have to be decoded
    /// and not shown. That is what this rule keeps.
    ///
    /// It used to drop pictures that were a whole frame or more *past* the
    /// position as well, on the theory that they were ones the platform had
    /// buffered before the seek. Media Foundation does not only land backwards:
    /// on a dense GOP it can land on the next key frame, and then every picture
    /// the decoder produced was past the position and was dropped, so the seek
    /// published nothing until the source ended. Measured on a 1080p file, a
    /// scrub took 1.3 s per seek and showed no picture at all. A picture after
    /// the position is the answer to the seek, not a stale frame, and the clock
    /// re-anchors to it as it would to any other late picture.
    fn before_the_seek(&self, pts: Duration, target: Duration) -> bool {
        let tolerance = self
            .decoder
            .frame_rate()
            .and_then(|rate| rate.frame_duration())
            .unwrap_or(DEFAULT_FRAME);
        pts.saturating_add(tolerance) < target
    }

    /// Holds the picture until the clock reaches it, so a file plays at the
    /// speed it was recorded at rather than as fast as it decodes.
    fn wait_until(&mut self, target: Duration) -> bool {
        // The clock never runs past the end of the media, so a picture at the
        // end or beyond it is due as soon as it is decoded.
        if self
            .shared
            .duration()
            .is_some_and(|duration| target >= duration)
        {
            return true;
        }
        loop {
            if self.shared.is_shutting_down() {
                return false;
            }
            if !self.shared.is_playing() {
                return true;
            }
            let now = self.shared.position();
            if now >= target {
                return true;
            }
            let remaining = target - now;
            if remaining > HORIZON {
                return true;
            }
            // `remaining` is media time: the clock advances by `speed` seconds
            // per wall second. The receive below is wall time, so convert before
            // waiting. Re-reading the speed on every pass means a SetSpeed that
            // arrives while we wait takes effect on the next pass, instead of
            // publishing every picture late (fast) or spinning (slow).
            let speed = f64::from(self.shared.speed()).max(0.001);
            let wall = remaining.div_f64(speed);
            if wall > FINE_WAIT {
                match self.inbox.recv_timeout((wall - FINE_WAIT).min(SLICE)) {
                    Ok(command) => {
                        if !self.handle(command) {
                            return false;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return false,
                }
                continue;
            }
            self.fine_wait.wait(wall);
            if let Ok(command) = self.inbox.try_recv() {
                if !self.handle(command) {
                    return false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::{DecodedFrame, FrameRate, VideoInfo, VideoState};

    /// A decoder with one picture at a fixed time, then the end of the media.
    struct OneFrame {
        pts: Duration,
        done: bool,
    }

    impl VideoDecoder for OneFrame {
        fn info(&self) -> VideoInfo {
            VideoInfo::default()
        }

        fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
            Ok(())
        }

        fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            if self.done {
                return Ok(None);
            }
            self.done = true;
            rgba.resize(2 * 2 * 4, 0);
            Ok(Some(DecodedFrame {
                pts: self.pts,
                width: 2,
                height: 2,
            }))
        }
    }

    struct CountingPause {
        pauses: Arc<AtomicUsize>,
    }

    impl VideoDecoder for CountingPause {
        fn info(&self) -> VideoInfo {
            VideoInfo::default()
        }

        fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
            Ok(())
        }

        fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            Ok(None)
        }

        fn pause(&mut self) {
            self.pauses.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn worker(pauses: Arc<AtomicUsize>) -> Worker {
        let (commands, inbox) = mpsc::channel();
        let _ = commands;
        Worker {
            inbox,
            decoder: Box::new(CountingPause { pauses }),
            shared: Arc::new(Shared::new(VideoInfo::default(), false, true, false, 1.0)),
            slot: Arc::new(Mutex::new(Slot::new())),
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            session: crate::PlaybackSession::new(),
            scratch: Vec::new(),
            fine_wait: FineWait::new(),
            planes: Vec::new(),
            last_pts: None,
            adopted: false,
            native_refused: false,
            ended: false,
            refresh: false,
            seeked_to: None,
            generation: 0,
        }
    }

    #[test]
    fn stop_stops_a_decoder_that_owns_a_platform_clock() {
        let pauses = Arc::new(AtomicUsize::new(0));
        let mut worker = worker(Arc::clone(&pauses));
        worker.handle(Command::Stop);
        worker.handle(Command::Pause);
        assert_eq!(
            pauses.load(Ordering::SeqCst),
            2,
            "stop and pause must both stop a platform clock"
        );
    }

    #[test]
    fn end_of_media_stops_a_decoder_that_owns_a_platform_clock() {
        let pauses = Arc::new(AtomicUsize::new(0));
        let mut worker = worker(Arc::clone(&pauses));
        assert!(worker.step(false));
        assert_eq!(
            pauses.load(Ordering::SeqCst),
            1,
            "the last frame must leave no clock running"
        );
    }

    #[test]
    fn pictures_far_behind_the_clock_are_detected_as_late() {
        let rate = Some(FrameRate {
            numerator: 15,
            denominator: 1,
        });
        assert!(is_late(
            Duration::from_millis(200),
            Duration::from_millis(50),
            rate
        ));
        assert!(!is_late(
            Duration::from_millis(60),
            Duration::from_millis(50),
            rate
        ));
    }

    #[test]
    fn a_live_stall_reanchors_the_clock_instead_of_dropping_every_frame() {
        let frame_pts = Duration::from_secs(10);
        // The wall clock ran on while the decode thread was blocked fetching a
        // new live window, so it is two seconds past the next picture's time.
        let stalled = frame_pts + Duration::from_secs(2);
        let (commands, inbox) = mpsc::channel();
        let _commands = commands;
        let shared = Arc::new(Shared::new(VideoInfo::default(), true, false, false, 1.0));
        shared.start();
        shared.set_position(stalled);
        let slot = Arc::new(Mutex::new(Slot::new()));
        let mut worker = Worker {
            inbox,
            decoder: Box::new(OneFrame {
                pts: frame_pts,
                done: false,
            }),
            shared: Arc::clone(&shared),
            slot: Arc::clone(&slot),
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            session: crate::PlaybackSession::new(),
            scratch: Vec::new(),
            fine_wait: FineWait::new(),
            planes: Vec::new(),
            last_pts: None,
            adopted: false,
            native_refused: false,
            ended: false,
            refresh: false,
            seeked_to: None,
            generation: 0,
        };
        assert!(worker.step(false));
        assert!(
            slot.lock().take().is_some(),
            "a picture after a live network stall must be published, not dropped"
        );
        assert!(
            shared.position() <= frame_pts + Duration::from_millis(50),
            "the clock must re-anchor to the reopened window: {:?}",
            shared.position()
        );
    }

    #[test]
    fn a_network_file_reanchors_when_the_first_frame_arrives_late() {
        // A remote MP4 buffers before the reader hands out its first picture.
        // The wall clock started at play() and is now well past the picture's
        // time. The old live-only re-anchor did not apply here, so every frame
        // was dropped until MAX_LATE_SKIPS had elapsed — one stale frame every
        // few seconds. A regular (non-live) file must re-anchor too.
        let frame_pts = Duration::ZERO;
        let buffered = Duration::from_millis(220);
        let (commands, inbox) = mpsc::channel();
        let _commands = commands;
        let shared = Arc::new(Shared::new(
            VideoInfo::default(),
            false, // not live
            true,
            false,
            1.0,
        ));
        shared.start();
        shared.set_position(buffered);
        let slot = Arc::new(Mutex::new(Slot::new()));
        let mut worker = Worker {
            inbox,
            decoder: Box::new(OneFrame {
                pts: frame_pts,
                done: false,
            }),
            shared: Arc::clone(&shared),
            slot: Arc::clone(&slot),
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            session: crate::PlaybackSession::new(),
            scratch: Vec::new(),
            fine_wait: FineWait::new(),
            planes: Vec::new(),
            last_pts: None,
            adopted: false,
            native_refused: false,
            ended: false,
            refresh: false,
            seeked_to: None,
            generation: 0,
        };
        assert!(worker.step(false));
        assert!(
            slot.lock().take().is_some(),
            "the first frame of a buffered network file must be published, not dropped"
        );
        assert!(
            shared.position() <= frame_pts + Duration::from_millis(50),
            "the clock must re-anchor to the late first frame: {:?}",
            shared.position()
        );
    }

    /// A decoder that plays one picture and reports the end, then plays the
    /// same picture again after a seek — which is what a file does.
    struct Repeating {
        pts: Duration,
        done: bool,
    }

    impl VideoDecoder for Repeating {
        fn info(&self) -> VideoInfo {
            VideoInfo::default()
        }

        fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
            self.done = false;
            Ok(())
        }

        fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            if self.done {
                return Ok(None);
            }
            self.done = true;
            rgba.resize(2 * 2 * 4, 0);
            Ok(Some(DecodedFrame {
                pts: self.pts,
                width: 2,
                height: 2,
            }))
        }
    }

    #[test]
    fn a_looping_player_repeats_instead_of_ending() {
        let (commands, inbox) = mpsc::channel();
        let _commands = commands;
        let shared = Arc::new(Shared::new(VideoInfo::default(), false, true, false, 1.0));
        shared.set_looping(true);
        shared.start();
        let slot = Arc::new(Mutex::new(Slot::new()));
        let mut worker = Worker {
            inbox,
            decoder: Box::new(Repeating {
                pts: Duration::from_millis(40),
                done: false,
            }),
            shared: Arc::clone(&shared),
            slot: Arc::clone(&slot),
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            session: crate::PlaybackSession::new(),
            scratch: Vec::new(),
            fine_wait: FineWait::new(),
            planes: Vec::new(),
            last_pts: None,
            adopted: false,
            native_refused: false,
            ended: false,
            refresh: false,
            seeked_to: None,
            generation: 0,
        };
        assert!(worker.step(false), "the first picture is published");
        assert!(slot.lock().take().is_some());

        // The source ended, and the loop reads the first picture again through
        // the decoder that is already open.
        assert!(worker.step(false), "the loop keeps the thread running");
        assert!(
            slot.lock().take().is_some(),
            "a looping source must publish after its end"
        );
        assert!(
            matches!(shared.state(), VideoState::Playing),
            "a looping source does not report the end: {:?}",
            shared.state()
        );
    }

    /// A backend that declares NV12 planes and never implements the planar
    /// read, which is what a third-party decoder looks like when it forgets it.
    struct DeclaredOnly;

    impl VideoDecoder for DeclaredOnly {
        fn info(&self) -> VideoInfo {
            VideoInfo::default()
        }

        fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
            Ok(())
        }

        fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            rgba.resize(2 * 2 * 4, 0x40);
            Ok(Some(DecodedFrame {
                pts: Duration::from_millis(40),
                width: 2,
                height: 2,
            }))
        }

        fn output_format(&self) -> PixelFormat {
            PixelFormat::Nv12
        }
    }

    #[test]
    fn a_decoder_that_declares_nv12_without_producing_it_still_plays() {
        let (commands, inbox) = mpsc::channel();
        let _commands = commands;
        let shared = Arc::new(Shared::new(VideoInfo::default(), false, true, false, 1.0));
        let slot = Arc::new(Mutex::new(Slot::new()));
        let mut worker = Worker {
            inbox,
            decoder: Box::new(DeclaredOnly),
            shared: Arc::clone(&shared),
            slot: Arc::clone(&slot),
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            session: crate::PlaybackSession::new(),
            scratch: Vec::new(),
            fine_wait: FineWait::new(),
            planes: Vec::new(),
            last_pts: None,
            adopted: false,
            native_refused: false,
            ended: false,
            refresh: false,
            seeked_to: None,
            generation: 0,
        };
        assert!(worker.step(false));
        let frame = slot
            .lock()
            .take()
            .expect("the RGBA path must publish a picture");
        assert_eq!(
            frame.format(),
            PixelFormat::Rgba8,
            "the layout must report what was really read"
        );
        assert_eq!(shared.diagnostics().pixel_format, PixelFormat::Rgba8);
        assert!(
            worker.native_refused,
            "the planar path is off after one try"
        );
    }

    /// A source that fails once and then becomes readable, which is what a live
    /// network window does while it is being replaced.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    struct FailsOnceThenPlays {
        done: bool,
        recovered: Arc<AtomicUsize>,
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    impl VideoDecoder for FailsOnceThenPlays {
        fn info(&self) -> VideoInfo {
            VideoInfo::default()
        }

        fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
            Ok(())
        }

        fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            if self.done {
                return Ok(None);
            }
            self.done = true;
            if self.recovered.load(Ordering::SeqCst) == 0 {
                return Err(VideoError::Source("window unavailable".into()));
            }
            rgba.resize(2 * 2 * 4, 0x40);
            Ok(Some(DecodedFrame {
                pts: Duration::ZERO,
                width: 2,
                height: 2,
            }))
        }

        fn recover(&mut self) -> bool {
            self.done = false;
            self.recovered.fetch_add(1, Ordering::SeqCst);
            true
        }
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    #[test]
    fn a_transient_decode_failure_returns_to_playing_after_recovery() {
        use crate::{PlaybackState, SessionEvent};

        let (commands, inbox) = mpsc::channel();
        let _commands = commands;
        let shared = Arc::new(Shared::new(VideoInfo::default(), false, true, false, 1.0));
        shared.start();
        let slot = Arc::new(Mutex::new(Slot::new()));
        let recovered = Arc::new(AtomicUsize::new(0));
        let mut worker = Worker {
            inbox,
            decoder: Box::new(FailsOnceThenPlays {
                done: false,
                recovered: Arc::clone(&recovered),
            }),
            shared: Arc::clone(&shared),
            slot: Arc::clone(&slot),
            session: shared.playback_session(),
            scratch: Vec::new(),
            fine_wait: FineWait::new(),
            planes: Vec::new(),
            last_pts: None,
            adopted: false,
            native_refused: false,
            ended: false,
            refresh: false,
            seeked_to: None,
            generation: 0,
        };

        assert!(worker.step(false));
        assert!(
            matches!(shared.state(), VideoState::Failed(_)),
            "the failed read must still be visible: {:?}",
            shared.state()
        );
        assert_eq!(worker.session.state(), PlaybackState::Recovering);

        worker.session.expire_retry_for_test();
        worker.maybe_recover();
        assert_eq!(recovered.load(Ordering::SeqCst), 1);
        assert!(
            matches!(shared.state(), VideoState::Failed(_)),
            "a rebuilt reader is not proof yet: {:?}",
            shared.state()
        );
        assert!(worker.refresh, "recovery must schedule a fresh read");

        assert!(worker.step(false));
        assert!(
            matches!(shared.state(), VideoState::Playing),
            "the new picture must clear the transient failure: {:?}",
            shared.state()
        );
        assert!(
            slot.lock().take().is_some(),
            "the frame after recovery must be published"
        );
        assert!(
            worker
                .session
                .events()
                .contains(&SessionEvent::Recovered { attempt: 1 }),
            "the successful retry must be recorded"
        );
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    struct FailsThenEmpty {
        failed: bool,
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    impl VideoDecoder for FailsThenEmpty {
        fn info(&self) -> VideoInfo {
            VideoInfo::default()
        }

        fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
            Ok(())
        }

        fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
            if self.failed {
                Ok(None)
            } else {
                self.failed = true;
                Err(VideoError::Source("window unavailable".into()))
            }
        }

        fn recover(&mut self) -> bool {
            true
        }
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    #[test]
    fn a_live_recovery_without_a_picture_rearms_instead_of_ending() {
        use crate::PlaybackState;

        let (commands, inbox) = mpsc::channel();
        let _commands = commands;
        // Live is the second argument; a finite clip may legitimately end.
        let shared = Arc::new(Shared::new(VideoInfo::default(), true, false, false, 1.0));
        shared.start();
        let slot = Arc::new(Mutex::new(Slot::new()));
        let mut worker = Worker {
            inbox,
            decoder: Box::new(FailsThenEmpty { failed: false }),
            shared: Arc::clone(&shared),
            slot: Arc::clone(&slot),
            session: shared.playback_session(),
            scratch: Vec::new(),
            fine_wait: FineWait::new(),
            planes: Vec::new(),
            last_pts: None,
            adopted: false,
            native_refused: false,
            ended: false,
            refresh: false,
            seeked_to: None,
            generation: 0,
        };

        assert!(worker.step(false));
        assert!(matches!(shared.state(), VideoState::Failed(_)));

        worker.session.expire_retry_for_test();
        worker.maybe_recover();
        assert!(worker.refresh);
        assert!(
            matches!(shared.state(), VideoState::Failed(_)),
            "the empty rebuild must not clear the failure"
        );

        assert!(worker.step(false));
        assert!(!worker.ended, "a live feed must not end on an empty window");
        assert!(
            matches!(shared.state(), VideoState::Failed(_)),
            "the empty window is still a failure: {:?}",
            shared.state()
        );
        assert_eq!(worker.session.state(), PlaybackState::Recovering);
        assert!(
            worker.session.next_retry().is_some(),
            "another attempt must be waiting"
        );
    }
}
