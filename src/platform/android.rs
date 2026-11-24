//! MediaExtractor and MediaCodec: the Android framework's own demuxer and
//! hardware decoder.
//!
//! `MediaExtractor` reads the track's samples and `MediaCodec` decodes them
//! with the device's hardware decoder. The codec hands out its picture as a
//! YUV420 image whose planes each state their own strides, and this file
//! converts it to RGBA. Reaching both classes costs no Java of its own: they
//! are framework classes, which the process's JVM resolves from any thread.
#![expect(
    unsafe_code,
    reason = "MediaCodec hands out its pixels through direct buffers, so the conversion reads them through a raw pointer"
)]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use jni::{
    Env, JavaVM, jni_sig, jni_str,
    objects::{Global, JByteBuffer, JObject, JObjectArray, JString, JValue},
    sys::jlong,
};

use super::super::temp_file::TempFile;
use crate::{
    ColorMatrix, ColorRange, DecodedFrame, FrameRate, PixelFormat, PlaybackSession, Rotation,
    VideoDecoder, VideoError, VideoInfo, VideoSource, YuvColor,
};

pub(crate) const NAME: &str = "MediaCodec";
pub(crate) const SUPPORTED: bool = true;

/// How long a dequeue waits for the codec, in microseconds. Long enough that a
/// hardware decoder that needs a moment does not spin, short enough that a
/// command reaches the codec promptly.
const TIMEOUT_US: jlong = 8_000;

/// How long the decoder may report that it has nothing yet before the read
/// fails. A hardware decoder can stall for a moment on its first picture, but
/// never for this long without a larger problem.
const EMPTY_DEADLINE: Duration = Duration::from_secs(30);

/// How long a failed JNI setup is cached before a plain `read` tries again.
///
/// The worker's session has its own, longer backoff for `recover()`. This short
/// delay only stops a caller that reads in a tight loop from hammering a device
/// that was busy a moment ago; it is never a permanent failure.
const SETUP_COOLDOWN: Duration = Duration::from_millis(400);

/// `MediaExtractor.SEEK_TO_PREVIOUS_SYNC`.
const SEEK_TO_PREVIOUS_SYNC: i32 = 0;

/// The most the demuxer may read for one sample.
const SAMPLE_CAPACITY: usize = 4 * 1024 * 1024;

/// The most the sample buffer may grow to. Any real sample is far smaller;
/// the ceiling stops a broken file from asking for unbounded memory.
const MAX_SAMPLE: usize = 128 * 1024 * 1024;

const MIME: &str = "mime";
const WIDTH: &str = "width";
const HEIGHT: &str = "height";
const ROTATION: &str = "rotation-degrees";
const FRAME_RATE: &str = "frame-rate";
const COLOR_STANDARD: &str = "color-standard";
const COLOR_RANGE: &str = "color-range";
const DURATION: &str = "durationUs";

/// The colour matrix a decoder's output states, which decides the coefficients
/// that turn luma and chroma into red, green and blue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Matrix {
    Bt601,
    Bt709,
}

/// Whether luma and chroma fill their whole range or the studio range a
/// broadcast signal uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Range {
    Limited,
    Full,
}

pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn VideoDecoder>, VideoError> {
    // Nothing here reaches the JVM. The player constructs this decoder on the
    // composition thread, so a path, URL or byte buffer is only held; writing a
    // spill file, finding the JVM and touching MediaExtractor/MediaCodec all
    // happen later, on the decode thread, the first time a frame is needed.
    Ok(Box::new(Reader::new(source.clone())))
}

fn vm() -> Result<JavaVM, VideoError> {
    if let Ok(vm) = JavaVM::singleton() {
        return Ok(vm);
    }
    let context = ndk_context::android_context();
    let raw = context.vm().cast();
    // SAFETY: the process's JVM lives for the whole process, and ndk-context
    // holds the pointer the runtime handed the application at start-up.
    Ok(unsafe { JavaVM::from_raw(raw) })
}

/// The player's shutdown signal plus this reader's own closed state.
///
/// Both are atomic so an in-flight setup can notice a close that happens while
/// it is between JNI calls, without needing the state mutex.
#[derive(Clone)]
struct StopFlag {
    shutdown: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
}

impl StopFlag {
    fn new() -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn observe(&mut self, shutdown: Arc<AtomicBool>) {
        self.shutdown = shutdown;
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    fn stopped(&self) -> bool {
        self.shutdown.load(Ordering::Acquire) || self.closed.load(Ordering::Acquire)
    }
}

/// The Android objects created during lazy setup on the decode thread.
struct Media {
    extractor: Global<JObject<'static>>,
    codec: Global<JObject<'static>>,
    flags: CodecFlags,
}

impl Media {
    /// Starts the configured decoder and rewinds the demuxer to its first
    /// sample. The codec must be started before it accepts a seek.
    fn start(&self, env: &mut Env<'_>, gate: &StopFlag) -> Result<(), VideoError> {
        stopped(gate)?;
        env.call_method(self.codec.as_obj(), jni_str!("start"), jni_sig!("()V"), &[])
            .map(|_| ())
            .map_err(|error| jni_error(env, "the decoder did not start", error))?;
        stopped(gate)?;
        self.rewind(env, gate)
    }

    /// Rewinds to the first sample of the file after a fresh setup.
    fn rewind(&self, env: &mut Env<'_>, gate: &StopFlag) -> Result<(), VideoError> {
        stopped(gate)?;
        self.seek_to(env, Duration::ZERO)?;
        stopped(gate)?;
        Ok(())
    }

    fn seek_to(&self, env: &mut Env<'_>, position: Duration) -> Result<(), VideoError> {
        let micros = i64::try_from(position.as_micros()).unwrap_or(i64::MAX);
        env.call_method(
            self.extractor.as_obj(),
            jni_str!("seekTo"),
            jni_sig!("(JI)V"),
            &[JValue::Long(micros), JValue::Int(SEEK_TO_PREVIOUS_SYNC)],
        )
        .map_err(|error| jni_error(env, "the demuxer cannot seek", error))?;
        // The codec keeps the samples it already queued, so both sides start
        // again at the key frame the demuxer landed on.
        env.call_method(self.codec.as_obj(), jni_str!("flush"), jni_sig!("()V"), &[])
            .map(|_| ())
            .map_err(|error| jni_error(env, "the decoder cannot restart", error))
    }
}

/// Everything that may change between lazy setup attempts. The player owns the
/// reader, but a mutex keeps a close that arrives during setup from racing with
/// the thread that is building the platform objects.
struct SetupState {
    media: Option<Arc<Media>>,
    spill: Option<TempFile>,
    failed_at: Option<Instant>,
}

impl SetupState {
    fn new() -> Self {
        Self {
            media: None,
            spill: None,
            failed_at: None,
        }
    }
}

struct Reader {
    source: VideoSource,
    state: Arc<Mutex<SetupState>>,
    gate: StopFlag,
    session: Option<PlaybackSession>,
    failure_reported_for: Option<u64>,
    info: VideoInfo,
    matrix: Matrix,
    range: Range,
    sample: Vec<u8>,
    finished: bool,
}

/// What one output dequeue produced.
///
/// Keeping the answer in Rust lets the caller delete the `BufferInfo` local
/// before it loops again; JNI local references live until the attached scope
/// ends unless they are released explicitly.
enum Dequeued {
    Frame(DecodedFrame),
    End,
    FormatChanged,
    BuffersChanged,
    TryAgain,
    Next,
}

// SAFETY: every Java reference is a global reference, which any thread may use,
// and the player calls the decoder from its decode thread only, one call at a
// time.
unsafe impl Send for Reader {}

impl Reader {
    fn new(source: VideoSource) -> Self {
        Self {
            source,
            state: Arc::new(Mutex::new(SetupState::new())),
            gate: StopFlag::new(),
            session: None,
            failure_reported_for: None,
            info: VideoInfo::default(),
            matrix: Matrix::Bt601,
            range: Range::Limited,
            sample: Vec::new(),
            finished: false,
        }
    }

    /// Builds the demuxer and decoder on the decode thread.
    ///
    /// A failed attempt only starts a short cooldown for ordinary reads; the
    /// worker's session keeps the authoritative backoff for `recover()`. Every
    /// real attempt reports one failure for the current session generation,
    /// under `android.recover`.
    fn ensure_ready(&mut self, force: bool) -> Result<(), VideoError> {
        if self.gate.stopped() {
            return Err(VideoError::Source("the player is closing".to_owned()));
        }
        {
            let state = self.state.lock().unwrap();
            if state.media.is_some() {
                return Ok(());
            }
            if !force && !cooldown_elapsed(state.failed_at) {
                return Err(VideoError::Source(
                    "the Android decoder is waiting to retry".to_owned(),
                ));
            }
        }

        let gate = self.gate.clone();
        let attempted = with_vm(&gate, |env| {
            stopped(&gate)?;
            let (address, spill) = source_address(&self.source)?;
            stopped(&gate)?;
            let (media, info) = setup_media(env, &address, &gate)?;
            {
                let mut state = self.state.lock().unwrap();
                // A close may have landed while Java was building the objects.
                // Recheck under the same lock that installs them.
                if gate.stopped() {
                    release_media_refs(env, &media);
                    return Err(VideoError::Source("the player is closing".to_owned()));
                }
                if state.media.is_some() {
                    // Another call won the race. Release this attempt's objects
                    // and let its spill file go out of scope.
                    release_media_refs(env, &media);
                } else {
                    state.media = Some(Arc::new(media));
                    state.spill = spill;
                    state.failed_at = None;
                }
            }
            self.info = info;
            self.finished = false;
            Ok(())
        });
        if let Err(error) = &attempted {
            if !gate.stopped() {
                self.state.lock().unwrap().failed_at = Some(Instant::now());
                self.report_setup_failure(error);
            }
        }
        attempted
    }

    // A read can fail many times before the player runs recovery. The session
    // already holds the first failure, so later reads stay quiet until a new
    // recovery generation starts.
    fn report_setup_failure(&mut self, error: &VideoError) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let generation = session.generation();
        if self.failure_reported_for == Some(generation) {
            return;
        }
        let reason = crate::diagnostics::redact_for_report(&error.to_string());
        crate::diag("WARN", "android.recover", &reason);
        session.failed("android.recover", reason);
        self.failure_reported_for = Some(generation);
    }

    /// Borrows the lazy platform objects for one decode-thread operation.
    ///
    /// The shared reference is cloned under the state lock and dropped inside
    /// the attached JNI scope, so a concurrent close cannot pull the objects
    /// out from under a call that is already using them.
    fn with_media<T>(
        &mut self,
        action: impl FnOnce(&mut Self, &mut Env<'_>, &Media) -> Result<T, VideoError>,
    ) -> Result<T, VideoError> {
        self.ensure_ready(false)?;
        let media = {
            let state = self.state.lock().unwrap();
            state
                .media
                .clone()
                .ok_or_else(|| VideoError::Source("the decoder is not ready".to_owned()))?
        };
        let gate = self.gate.clone();
        with_vm(&gate, |env| {
            stopped(&gate)?;
            let result = action(self, env, &media);
            drop(media);
            result
        })
    }

    fn seek_to(
        &mut self,
        env: &mut Env<'_>,
        media: &Media,
        position: Duration,
    ) -> Result<(), VideoError> {
        media.seek_to(env, position)?;
        self.finished = false;
        Ok(())
    }

    fn read_picture(
        &mut self,
        env: &mut Env<'_>,
        media: &Media,
        buffer: &mut Vec<u8>,
        native: bool,
    ) -> Result<Option<DecodedFrame>, VideoError> {
        let flags_read = media.flags;
        // A codec that has not produced its first picture yet answers
        // `INFO_TRY_AGAIN_LATER`. That is not the end of the stream: feeding
        // more input and asking again is what makes it produce one. Only a
        // codec that stays empty for the whole deadline is reported as failed.
        let began = Instant::now();
        loop {
            stopped(&self.gate)?;
            if !self.finished {
                self.feed(env, media, flags_read.end_of_stream)?;
            }
            stopped(&self.gate)?;
            match self.dequeue(env, media, buffer, native, flags_read)? {
                Dequeued::Frame(frame) => return Ok(Some(frame)),
                Dequeued::End => return Ok(None),
                Dequeued::FormatChanged => self.adopt_output_format(env, media)?,
                Dequeued::BuffersChanged | Dequeued::Next => {}
                Dequeued::TryAgain => {
                    if began.elapsed() >= EMPTY_DEADLINE {
                        return Err(VideoError::Decode(
                            "the Android decoder produced no picture for 30 seconds".to_owned(),
                        ));
                    }
                }
            }
        }
    }

    /// dequeues one output buffer and releases its `BufferInfo` local before
    /// reporting the result to the read loop.
    fn dequeue(
        &mut self,
        env: &mut Env<'_>,
        media: &Media,
        buffer: &mut Vec<u8>,
        native: bool,
        flags_read: CodecFlags,
    ) -> Result<Dequeued, VideoError> {
        let buffer_info = env
            .new_object(
                jni_str!("android/media/MediaCodec$BufferInfo"),
                jni_sig!("()V"),
                &[],
            )
            .map_err(|error| jni_error(env, "cannot reach the decoder's picture", error))?;
        let result = self.handle_output(env, media, buffer, native, flags_read, &buffer_info);
        release_local(env, &buffer_info);
        result
    }

    /// Handles one dequeued index. The `BufferInfo` stays valid for this call
    /// only; its owner releases it once this returns.
    fn handle_output(
        &mut self,
        env: &mut Env<'_>,
        media: &Media,
        buffer: &mut Vec<u8>,
        native: bool,
        flags_read: CodecFlags,
        buffer_info: &JObject<'_>,
    ) -> Result<Dequeued, VideoError> {
        let index = env
            .call_method(
                media.codec.as_obj(),
                jni_str!("dequeueOutputBuffer"),
                jni_sig!("(Landroid/media/MediaCodec$BufferInfo;J)I"),
                &[JValue::Object(buffer_info), JValue::Long(TIMEOUT_US)],
            )
            .and_then(jni::JValueOwned::i)
            .map_err(|error| jni_error(env, "the decoder stopped", error))?;

        if index >= 0 {
            // `BufferInfo` exposes public fields, not `getInteger` methods.
            // Reading them as method calls silently yielded zeros: no frame
            // was ever copied and the loop spun until the empty deadline.
            let flags = int_value(env, buffer_info, jni_str!("flags"))
                .map_err(|error| jni_error(env, "cannot read the picture's flags", error))?;
            let size = int_value(env, buffer_info, jni_str!("size"))
                .map_err(|error| jni_error(env, "cannot read the picture's size", error))?;
            let micros = long_value(env, buffer_info, jni_str!("presentationTimeUs"))
                .map_err(|error| jni_error(env, "cannot read the picture's time", error))?;
            // Always return the codec buffer, even when conversion fails.
            // An unreleased output index exhausts the codec and stalls every
            // later dequeue until the deadline fires.
            let picture = if size > 0 && flags & flags_read.codec_config == 0 {
                self.copy(env, media, index, micros, buffer, native)
            } else {
                Ok(None)
            };
            env.call_method(
                media.codec.as_obj(),
                jni_str!("releaseOutputBuffer"),
                jni_sig!("(IZ)V"),
                &[JValue::Int(index), JValue::Bool(false)],
            )
            .map_err(|error| jni_error(env, "the decoder kept its picture", error))?;
            if let Some(frame) = picture? {
                return Ok(Dequeued::Frame(frame));
            }
            if flags & flags_read.end_of_stream != 0 {
                return Ok(Dequeued::End);
            }
            return Ok(Dequeued::Next);
        }
        if index == flags_read.format_changed {
            Ok(Dequeued::FormatChanged)
        } else if index == flags_read.buffers_changed {
            // The codec replaced its output buffers. Nothing is held across a
            // dequeue, so the next one reads from the new set.
            Ok(Dequeued::BuffersChanged)
        } else if index == flags_read.try_again {
            Ok(Dequeued::TryAgain)
        } else {
            Err(VideoError::Decode(format!(
                "the decoder reported {index} instead of a picture"
            )))
        }
    }

    /// Allocates the sample buffer on the decode thread, not while the player
    /// constructs the decoder on the composition thread.
    fn ensure_sample_buffer(&mut self) {
        if self.sample.is_empty() {
            self.sample.resize(SAMPLE_CAPACITY, 0);
        }
    }

    /// Reads the next compressed sample into the reusable buffer, returning its
    /// size, or a negative value at the end of the stream. The extractor only
    /// moves on when [`Reader::feed`] calls `advance`, so a read that does not
    /// fit can be repeated with a larger buffer without losing bytes.
    fn read_sample(&mut self, env: &mut Env<'_>, media: &Media) -> Result<i32, VideoError> {
        // SAFETY: `self.sample` outlives this call and is not touched again
        // until the local ByteBuffer is dropped below. `readSampleData` writes
        // into the buffer during the call and does not retain it afterwards.
        let buffer =
            unsafe { env.new_direct_byte_buffer(self.sample.as_mut_ptr(), self.sample.len()) }
                .map_err(|error| jni_error(env, "cannot offer the demuxer a buffer", error))?;
        let read = env.call_method(
            media.extractor.as_obj(),
            jni_str!("readSampleData"),
            jni_sig!("(Ljava/nio/ByteBuffer;I)I"),
            &[JValue::Object(&buffer), JValue::Int(0)],
        );
        release_local(env, &buffer);
        read.and_then(jni::JValueOwned::i)
            .map_err(|error| jni_error(env, "the demuxer cannot read a sample", error))
    }

    /// Hands one sample from the demuxer to the codec.
    fn feed(&mut self, env: &mut Env<'_>, media: &Media, eos_flag: i32) -> Result<(), VideoError> {
        // Read and size the sample before taking a codec input buffer. If the
        // demuxer fails or the sample needs more room than the ceiling allows,
        // no buffer is held and there is nothing to give back.
        self.ensure_sample_buffer();
        let mut size = self.read_sample(env, media)?;
        // A full buffer holds only the head of a sample that is larger. Grow
        // and read the same sample again; the extractor has not advanced, so
        // no bytes are lost.
        while size >= 0 && size as usize >= self.sample.len() {
            let needed = size as usize;
            if self.sample.len() >= MAX_SAMPLE {
                return Err(VideoError::Decode(format!(
                    "one sample is at least {needed} bytes, larger than this decoder accepts"
                )));
            }
            let next = self
                .sample
                .len()
                .saturating_mul(2)
                .max(needed.saturating_add(1))
                .min(MAX_SAMPLE);
            self.sample.resize(next, 0);
            size = self.read_sample(env, media)?;
        }

        let index = env
            .call_method(
                media.codec.as_obj(),
                jni_str!("dequeueInputBuffer"),
                jni_sig!("(J)I"),
                &[JValue::Long(TIMEOUT_US)],
            )
            .and_then(jni::JValueOwned::i)
            .map_err(|error| jni_error(env, "the decoder takes no sample", error))?;
        if index < 0 {
            return Ok(());
        }
        // The codec holds `index` until it is queued back exactly once. Prepare
        // the sample first; on any failure return the index empty, or the codec
        // would run out of input buffers and every later feed would stall.
        let prepared = self.prepare_input(env, media, index, size, eos_flag);
        let (offset, length, micros, flags) = match prepared {
            Ok(parameters) => parameters,
            Err(error) => {
                self.return_input_buffer(env, media, index);
                return Err(error);
            }
        };
        env.call_method(
            media.codec.as_obj(),
            jni_str!("queueInputBuffer"),
            jni_sig!("(IIIJI)V"),
            &[
                JValue::Int(index),
                JValue::Int(offset),
                JValue::Int(length),
                JValue::Long(micros),
                JValue::Int(flags),
            ],
        )
        .map(|_| ())
        .map_err(|error| jni_error(env, "the decoder refused the sample", error))
    }

    /// Fills a dequeued input buffer and returns the arguments to queue it with:
    /// offset, byte count, presentation time and flags. Does not queue, so the
    /// caller can hand the index back empty if anything here fails.
    fn prepare_input(
        &mut self,
        env: &mut Env<'_>,
        media: &Media,
        index: i32,
        size: i32,
        eos_flag: i32,
    ) -> Result<(i32, i32, i64, i32), VideoError> {
        if size < 0 {
            self.finished = true;
            return Ok((0, 0, 0, eos_flag));
        }
        let micros = match env
            .call_method(
                media.extractor.as_obj(),
                jni_str!("getSampleTime"),
                jni_sig!("()J"),
                &[],
            )
            .and_then(jni::JValueOwned::j)
        {
            Ok(value) => value,
            Err(error) => {
                // A failed call leaves a pending Java exception. Clearing it is
                // mandatory: the next getInputBuffer JNI call would otherwise run
                // with an exception pending, which is undefined behaviour.
                crate::diag(
                    "WARN",
                    "android.sample",
                    &format!("getSampleTime failed: {error}"),
                );
                clear(env);
                0
            }
        };
        let input = env
            .call_method(
                media.codec.as_obj(),
                jni_str!("getInputBuffer"),
                jni_sig!("(I)Ljava/nio/ByteBuffer;"),
                &[JValue::Int(index)],
            )
            .and_then(jni::JValueOwned::l)
            .map_err(|error| jni_error(env, "the decoder has no input buffer", error))?;
        let addressed: Result<(*mut u8, usize), VideoError> = (|| {
            let typed = env
                .as_cast::<JByteBuffer>(&input)
                .map_err(|error| jni_error(env, "the decoder handed out no byte buffer", error))?;
            let destination = env
                .get_direct_buffer_address(&typed)
                .map_err(|error| jni_error(env, "cannot write the sample", error))?;
            let capacity = env
                .get_direct_buffer_capacity(&typed)
                .map_err(|error| jni_error(env, "cannot size the sample's destination", error))?;
            Ok((destination, capacity))
        })();
        release_local(env, &input);
        let (destination, destination_capacity) = addressed?;
        let length = size as usize;
        let address = self.sample.as_ptr();
        if destination.is_null() || length > destination_capacity || length > self.sample.len() {
            return Err(VideoError::Decode(
                "the decoder asked for more than one sample holds".to_owned(),
            ));
        }
        // SAFETY: `destination` is the codec's direct input buffer and holds at
        // least `length` bytes; `self.sample` holds the sample's `length` bytes
        // at `address`, and neither region overlaps the other.
        unsafe { std::ptr::copy_nonoverlapping(address, destination, length) };
        env.call_method(
            media.extractor.as_obj(),
            jni_str!("advance"),
            jni_sig!("()Z"),
            &[],
        )
        .map_err(|error| jni_error(env, "the demuxer cannot advance", error))?;
        Ok((0, size, micros, 0))
    }

    /// Returns a dequeued input buffer to the codec as an empty sample. Best
    /// effort: a second failure must not hide the error that caused this.
    fn return_input_buffer(&self, env: &mut Env<'_>, media: &Media, index: i32) {
        let empty = [
            JValue::Int(index),
            JValue::Int(0),
            JValue::Int(0),
            JValue::Long(0),
            JValue::Int(0),
        ];
        let queued = env.call_method(
            media.codec.as_obj(),
            jni_str!("queueInputBuffer"),
            jni_sig!("(IIIJI)V"),
            &empty,
        );
        if queued.is_err() {
            dismiss(env);
        }
    }

    /// Converts one picture to RGBA and closes it, so the codec gets its
    /// buffer back.
    fn copy(
        &mut self,
        env: &mut Env<'_>,
        media: &Media,
        index: i32,
        micros: i64,
        buffer: &mut Vec<u8>,
        native: bool,
    ) -> Result<Option<DecodedFrame>, VideoError> {
        let image = env
            .call_method(
                media.codec.as_obj(),
                jni_str!("getOutputImage"),
                jni_sig!("(I)Landroid/media/Image;"),
                &[JValue::Int(index)],
            )
            .and_then(jni::JValueOwned::l)
            .map_err(|error| jni_error(env, "cannot reach the decoded picture", error))?;
        if image.as_raw().is_null() {
            return Ok(None);
        }
        let converted = if native {
            self.planes_of(env, &image, micros, buffer)
        } else {
            self.convert(env, &image, micros, buffer)
        };
        let closed = env
            .call_method(&image, jni_str!("close"), jni_sig!("()V"), &[])
            .map(|_| ())
            .map_err(|error| jni_error(env, "cannot release the decoded picture", error));
        release_local(env, &image);
        closed?;
        converted
    }

    /// Copies one open picture's planes into unpadded NV12: luma rows, then one
    /// interleaved chroma row per two luma rows.
    ///
    /// The framework hands out either three separate planes or one luma plane
    /// and one interleaved chroma plane, and both are joined into the layout a
    /// frame carries. The colour conversion then happens once, across every
    /// core, when the screen asks for RGB, instead of in a scalar loop here on
    /// the decode thread.
    fn planes_of(
        &mut self,
        env: &mut Env<'_>,
        image: &JObject<'_>,
        micros: i64,
        nv12: &mut Vec<u8>,
    ) -> Result<Option<DecodedFrame>, VideoError> {
        let width = env
            .call_method(image, jni_str!("getWidth"), jni_sig!("()I"), &[])
            .and_then(jni::JValueOwned::i)
            .map_err(|error| jni_error(env, "the picture states no size", error))?;
        let height = env
            .call_method(image, jni_str!("getHeight"), jni_sig!("()I"), &[])
            .and_then(jni::JValueOwned::i)
            .map_err(|error| jni_error(env, "the picture states no size", error))?;
        if width <= 0 || height <= 0 {
            return Ok(None);
        }
        let (width, height) = (width as usize, height as usize);
        if width % 2 != 0 || height % 2 != 0 {
            return Err(VideoError::Decode(
                "the decoder handed out a 4:2:0 picture with an odd side".to_owned(),
            ));
        }
        let planes = env
            .call_method(
                image,
                jni_str!("getPlanes"),
                jni_sig!("()[Landroid/media/Image$Plane;"),
                &[],
            )
            .and_then(jni::JValueOwned::l)
            .map_err(|error| jni_error(env, "the picture holds no planes", error))?;
        let planes = JObjectArray::<JObject<'_>>::cast_local(env, planes)
            .map_err(|error| jni_error(env, "the picture holds no planes", error))?;
        let result = (|| -> Result<Option<DecodedFrame>, VideoError> {
            let count = planes
                .len(&*env)
                .map_err(|error| jni_error(env, "the picture holds no planes", error))?;
            if count < 2 {
                return Err(VideoError::Decode(
                    "the decoder handed out a picture without colour planes".to_owned(),
                ));
            }

            let luma = plane_data(env, &planes, 0)?;
            if !luma.holds(height, width) {
                return Err(VideoError::Decode(
                    "the decoder stated a picture smaller than its planes hold".to_owned(),
                ));
            }
            if nv12.len() != width * height * 3 / 2 {
                nv12.resize(width * height * 3 / 2, 0);
            }
            // SAFETY: the plane holds the rows read below, checked just above,
            // and the image stays open for the whole copy.
            let luma_pixels = unsafe { std::slice::from_raw_parts(luma.address, luma.capacity) };
            for y in 0..height {
                let source = &luma_pixels[y * luma.row_stride..];
                let out = &mut nv12[y * width..][..width];
                if luma.pixel_stride == 1 {
                    out.copy_from_slice(&source[..width]);
                } else {
                    for (x, sample) in out.iter_mut().enumerate() {
                        *sample = source[x * luma.pixel_stride];
                    }
                }
            }

            let base = width * height;
            let rows = height / 2;
            let columns = width / 2;
            if count >= 3 {
                let blue = plane_data(env, &planes, 1)?;
                let red = plane_data(env, &planes, 2)?;
                if !blue.holds(rows, columns) || !red.holds(rows, columns) {
                    return Err(VideoError::Decode(
                        "the decoder stated a picture smaller than its planes hold".to_owned(),
                    ));
                }
                // SAFETY: each plane holds the rows read below, checked above.
                let blue_pixels =
                    unsafe { std::slice::from_raw_parts(blue.address, blue.capacity) };
                // SAFETY: as above, for the plane that holds the red difference.
                let red_pixels = unsafe { std::slice::from_raw_parts(red.address, red.capacity) };
                for y in 0..rows {
                    let blue_row = &blue_pixels[y * blue.row_stride..];
                    let red_row = &red_pixels[y * red.row_stride..];
                    let out = &mut nv12[base + y * width..][..width];
                    for x in 0..columns {
                        out[2 * x] = blue_row[x * blue.pixel_stride];
                        out[2 * x + 1] = red_row[x * red.pixel_stride];
                    }
                }
            } else {
                let chroma = plane_data(env, &planes, 1)?;
                // SAFETY: the plane holds the rows read below, checked per shape.
                let pixels = unsafe { std::slice::from_raw_parts(chroma.address, chroma.capacity) };
                for y in 0..rows {
                    let out = &mut nv12[base + y * width..][..width];
                    if chroma.pixel_stride >= 2 {
                        // The chroma plane is already interleaved, which is NV12
                        // itself, so a row copies across as it lies.
                        if !chroma.holds(rows, width / chroma.pixel_stride) {
                            return Err(VideoError::Decode(
                                "the decoder stated a picture smaller than its planes hold"
                                    .to_owned(),
                            ));
                        }
                        out.copy_from_slice(&pixels[y * chroma.row_stride..][..width]);
                    } else {
                        // One plane holding every blue row and then every red one.
                        if !chroma.holds(rows * 2, columns) {
                            return Err(VideoError::Decode(
                                "the decoder stated a picture smaller than its planes hold"
                                    .to_owned(),
                            ));
                        }
                        let blue_row = &pixels[y * chroma.row_stride..];
                        let red_row = &pixels[(rows + y) * chroma.row_stride..];
                        for x in 0..columns {
                            out[2 * x] = blue_row[x * chroma.pixel_stride];
                            out[2 * x + 1] = red_row[x * chroma.pixel_stride];
                        }
                    }
                }
            }

            Ok(Some(DecodedFrame {
                pts: Duration::from_micros(u64::try_from(micros.max(0)).unwrap_or(0)),
                width: width as u32,
                height: height as u32,
            }))
        })();
        release_local(env, &planes);
        result
    }

    fn convert(
        &mut self,
        env: &mut Env<'_>,
        image: &JObject<'_>,
        micros: i64,
        rgba: &mut Vec<u8>,
    ) -> Result<Option<DecodedFrame>, VideoError> {
        let width = env
            .call_method(image, jni_str!("getWidth"), jni_sig!("()I"), &[])
            .and_then(jni::JValueOwned::i)
            .map_err(|error| jni_error(env, "the picture states no size", error))?;
        let height = env
            .call_method(image, jni_str!("getHeight"), jni_sig!("()I"), &[])
            .and_then(jni::JValueOwned::i)
            .map_err(|error| jni_error(env, "the picture states no size", error))?;
        if width <= 0 || height <= 0 {
            return Ok(None);
        }
        let width = width as u32;
        let height = height as u32;
        let planes = env
            .call_method(
                image,
                jni_str!("getPlanes"),
                jni_sig!("()[Landroid/media/Image$Plane;"),
                &[],
            )
            .and_then(jni::JValueOwned::l)
            .map_err(|error| jni_error(env, "the picture holds no planes", error))?;
        let planes = JObjectArray::<JObject<'_>>::cast_local(env, planes)
            .map_err(|error| jni_error(env, "the picture holds no planes", error))?;
        let count = planes
            .len(&*env)
            .map_err(|error| jni_error(env, "the picture holds no planes", error))?;
        if count < 3 {
            return Err(VideoError::Decode(
                "the decoder handed out a picture without colour planes".to_owned(),
            ));
        }

        let luma = plane_data(env, &planes, 0)?;
        let blue = plane_data(env, &planes, 1)?;
        let red = plane_data(env, &planes, 2)?;

        let row = width as usize * 4;
        let needed = row * height as usize;
        if rgba.len() != needed {
            rgba.resize(needed, 0);
        }
        let colour = Colour {
            matrix: self.matrix,
            range: self.range,
        };
        colour.convert(&luma, &blue, &red, width, height, rgba)?;

        Ok(Some(DecodedFrame {
            pts: Duration::from_micros(u64::try_from(micros.max(0)).unwrap_or(0)),
            width,
            height,
        }))
    }

    /// Reads the size, colour matrix and range the decoder states after it has
    /// seen the stream's first frames.
    fn adopt_output_format(&mut self, env: &mut Env<'_>, media: &Media) -> Result<(), VideoError> {
        let format = env
            .call_method(
                media.codec.as_obj(),
                jni_str!("getOutputFormat"),
                jni_sig!("()Landroid/media/MediaFormat;"),
                &[],
            )
            .and_then(jni::JValueOwned::l)
            .map_err(|error| jni_error(env, "the decoder states no output format", error))?;
        let result = (|| -> Result<(), VideoError> {
            if let Some(width) = int_field(env, &format, WIDTH).filter(|width| *width > 0) {
                self.info.width = width as u32;
            }
            if let Some(height) = int_field(env, &format, HEIGHT).filter(|height| *height > 0) {
                self.info.height = height as u32;
            }
            if let Some(rotation) = int_field(env, &format, ROTATION) {
                self.info.rotation = Rotation::from_degrees(f64::from(rotation));
            }
            if let Some(rate) = float_field(env, &format, FRAME_RATE) {
                self.info.frame_rate = FrameRate::approx(rate);
            }
            self.matrix = match int_field(env, &format, COLOR_STANDARD) {
                // 1 is BT.709, 2 and 4 are the two BT.601 variants.
                Some(1) => Matrix::Bt709,
                _ => Matrix::Bt601,
            };
            self.range = match int_field(env, &format, COLOR_RANGE) {
                // 1 is the full range, 2 the studio range every other decoder uses.
                Some(1) => Range::Full,
                _ => Range::Limited,
            };
            Ok(())
        })();
        release_local(env, &format);
        result
    }
}

/// The text MediaExtractor takes, and the temporary file that must outlive it
/// when the caller supplied bytes. No JNI is involved; this runs on whichever
/// thread performs lazy setup.
fn source_address(source: &VideoSource) -> Result<(String, Option<TempFile>), VideoError> {
    if let VideoSource::Bytes(bytes) = source {
        let spill = TempFile::write(bytes)?;
        let address = spill.path().to_string_lossy().into_owned();
        return Ok((address, Some(spill)));
    }
    let text = source
        .as_text()
        .ok_or_else(|| VideoError::Source("the clip has no address".to_owned()))?;
    Ok((text, None))
}

fn cooldown_elapsed(failed_at: Option<Instant>) -> bool {
    failed_at.map_or(true, |at| at.elapsed() >= SETUP_COOLDOWN)
}

fn stopped(gate: &StopFlag) -> Result<(), VideoError> {
    if gate.stopped() {
        Err(VideoError::Source("the player is closing".to_owned()))
    } else {
        Ok(())
    }
}

/// Attaches the calling decode thread once for one bounded JNI operation. No
/// caller on the composition thread ever reaches this.
fn with_vm<T>(
    gate: &StopFlag,
    action: impl FnOnce(&mut Env<'_>) -> Result<T, VideoError>,
) -> Result<T, VideoError> {
    stopped(gate)?;
    let vm = vm()?;
    vm.attach_current_thread(|env| -> jni::errors::Result<Result<T, VideoError>> {
        Ok(action(env))
    })
    .map_err(|error| VideoError::Source(format!("cannot reach the Android runtime: {error}")))?
}

/// Builds the extractor and codec on whichever thread calls this.
///
/// The public `open` never calls it. Every call here is bounded JNI work, and
/// the gate is checked between the major steps so a close can stop a rebuild
/// before it creates more platform objects.
fn setup_media(
    env: &mut Env<'_>,
    address: &str,
    gate: &StopFlag,
) -> Result<(Media, VideoInfo), VideoError> {
    stopped(gate)?;
    let extractor = env
        .new_object(
            jni_str!("android/media/MediaExtractor"),
            jni_sig!("()V"),
            &[],
        )
        .map_err(|error| source_error(env, "cannot create the demuxer", error))?;
    let result = setup_with_extractor(env, &extractor, address, gate);
    release_local(env, &extractor);
    stopped(gate)?;
    result
}

fn setup_with_extractor<'local>(
    env: &mut Env<'local>,
    extractor: &JObject<'local>,
    address: &str,
    gate: &StopFlag,
) -> Result<(Media, VideoInfo), VideoError> {
    stopped(gate)?;
    let text = env
        .new_string(address)
        .map_err(|error| source_error(env, "cannot state the clip's address", error))?;
    let opened = env.call_method(
        extractor,
        jni_str!("setDataSource"),
        jni_sig!("(Ljava/lang/String;)V"),
        &[JValue::Object(&text)],
    );
    release_local(env, &text);
    opened.map_err(|error| source_error(env, "the demuxer cannot open the clip", error))?;
    stopped(gate)?;

    let track = video_track(env, extractor, gate)?;
    stopped(gate)?;
    let format = env
        .call_method(
            extractor,
            jni_str!("getTrackFormat"),
            jni_sig!("(I)Landroid/media/MediaFormat;"),
            &[JValue::Int(track)],
        )
        .and_then(jni::JValueOwned::l)
        .map_err(|error| source_error(env, "cannot read the video track", error))?;
    let result = (|| -> Result<(Media, VideoInfo), VideoError> {
        let mime = string_field(env, &format, MIME).ok_or(VideoError::NoVideoTrack)?;
        let info = VideoInfo {
            width: int_field(env, &format, WIDTH).unwrap_or(0).max(0) as u32,
            height: int_field(env, &format, HEIGHT).unwrap_or(0).max(0) as u32,
            duration: duration_from(int_field_long(env, &format, DURATION)),
            frame_rate: FrameRate::approx(float_field(env, &format, FRAME_RATE).unwrap_or(0.0)),
            rotation: Rotation::from_degrees(f64::from(
                int_field(env, &format, ROTATION).unwrap_or(0),
            )),
        };
        stopped(gate)?;

        let selected = env.call_method(
            extractor,
            jni_str!("selectTrack"),
            jni_sig!("(I)V"),
            &[JValue::Int(track)],
        );
        selected.map_err(|error| source_error(env, "cannot select the video track", error))?;
        stopped(gate)?;

        let codec = create_codec(env, &mime, gate)?;
        stopped(gate)?;
        let result = setup_with_codec(env, extractor, &codec, &format, info, gate);
        // `codec` is a Global; it deletes its own JNI reference on drop. Passing
        // it to release_local would call DeleteLocalRef on a global handle,
        // which is undefined behaviour (CheckJNI can abort on some devices).
        result
    })();
    release_local(env, &format);
    result
}

fn setup_with_codec<'local>(
    env: &mut Env<'local>,
    extractor: &JObject<'local>,
    codec: &JObject<'local>,
    format: &JObject<'_>,
    info: VideoInfo,
    gate: &StopFlag,
) -> Result<(Media, VideoInfo), VideoError> {
    stopped(gate)?;
    configure(env, codec, format)?;
    stopped(gate)?;

    let media = Media {
        extractor: global(env, extractor)?,
        codec: global(env, codec)?,
        flags: CodecFlags::read(env),
    };
    if gate.stopped() {
        release_media_refs(env, &media);
        return Err(VideoError::Source("the player is closing".to_owned()));
    }
    if let Err(error) = media.start(env, gate) {
        release_media_refs(env, &media);
        return Err(error);
    }
    stopped(gate)?;
    Ok((media, info))
}

fn video_track(
    env: &mut Env<'_>,
    extractor: &JObject<'_>,
    gate: &StopFlag,
) -> Result<i32, VideoError> {
    stopped(gate)?;
    let count = env
        .call_method(extractor, jni_str!("getTrackCount"), jni_sig!("()I"), &[])
        .and_then(jni::JValueOwned::i)
        .map_err(|error| source_error(env, "cannot count the clip's tracks", error))?;
    stopped(gate)?;
    let mime_key = env
        .new_string(MIME)
        .map_err(|error| source_error(env, "cannot read the tracks", error))?;
    let result = (|| -> Result<i32, VideoError> {
        for index in 0..count {
            stopped(gate)?;
            let format = env
                .call_method(
                    extractor,
                    jni_str!("getTrackFormat"),
                    jni_sig!("(I)Landroid/media/MediaFormat;"),
                    &[JValue::Int(index)],
                )
                .and_then(jni::JValueOwned::l)
                .map_err(|error| source_error(env, "cannot read a track", error))?;
            let found = {
                let mime = env.call_method(
                    &format,
                    jni_str!("getString"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                    &[JValue::Object(&mime_key)],
                );
                let text = match optional(env, mime.and_then(jni::JValueOwned::l)) {
                    Some(mime) => {
                        let cast = env.as_cast::<JString>(&mime);
                        let cast = optional(env, cast);
                        let text = match cast {
                            Some(cast) => {
                                let converted = cast.try_to_string(&*env);
                                optional(env, converted)
                            }
                            None => None,
                        };
                        release_local(env, &mime);
                        text
                    }
                    None => None,
                };
                text.is_some_and(|text| text.starts_with("video/"))
            };
            release_local(env, &format);
            if found {
                return Ok(index);
            }
        }
        Err(VideoError::NoVideoTrack)
    })();
    release_local(env, &mime_key);
    result
}

fn create_codec(
    env: &mut Env<'_>,
    mime: &str,
    gate: &StopFlag,
) -> Result<Global<JObject<'static>>, VideoError> {
    stopped(gate)?;
    let text = env
        .new_string(mime)
        .map_err(|error| jni_error(env, "cannot read the track's format", error))?;
    let result = (|| -> Result<Global<JObject<'static>>, VideoError> {
        let codec = env
            .call_static_method(
                jni_str!("android/media/MediaCodec"),
                jni_str!("createDecoderByType"),
                jni_sig!("(Ljava/lang/String;)Landroid/media/MediaCodec;"),
                &[JValue::Object(&text)],
            )
            .and_then(jni::JValueOwned::l)
            .map_err(|error| jni_error(env, "the device has no decoder for this video", error))?;
        stopped(gate)?;
        let created = global(env, &codec);
        release_local(env, &codec);
        created
    })();
    release_local(env, &text);
    result
}

/// Points the codec at the track with no output surface, so its pictures arrive
/// as byte buffers this file converts. Starting it belongs to `Media::start`.
fn configure(
    env: &mut Env<'_>,
    codec: &JObject<'_>,
    format: &JObject<'_>,
) -> Result<(), VideoError> {
    let nothing = JObject::null();
    env.call_method(
        codec,
        jni_str!("configure"),
        jni_sig!(
            "(Landroid/media/MediaFormat;Landroid/view/Surface;Landroid/media/MediaCrypto;I)V"
        ),
        &[
            JValue::Object(format),
            JValue::Object(&nothing),
            JValue::Object(&nothing),
            JValue::Int(0),
        ],
    )
    .map(|_| ())
    .map_err(|error| jni_error(env, "the decoder refused the video track", error))
}

fn global<'local>(
    env: &mut Env<'local>,
    object: &JObject<'local>,
) -> Result<Global<JObject<'static>>, VideoError> {
    env.new_global_ref(object)
        .map_err(|error| jni_error(env, "cannot hold the Android decoder", error))
}

/// Drops one local reference before the attached scope ends. Best effort: a
/// cleanup failure must not hide the error that brought us here.
fn release_local<'object, O: AsRef<JObject<'object>>>(env: &mut Env<'_>, object: O) {
    // A fresh wrapper around the same raw local reference has no `Drop` side
    // effects; `delete_local_ref` consumes it once and deletes the reference.
    // SAFETY: every caller passes a local reference created on this attached
    // thread and never uses it again after this point.
    let object = unsafe { JObject::from_raw(env, object.as_ref().as_raw()) };
    env.delete_local_ref(object);
}

/// Releases the platform objects and their global references on a failure path
/// or during close. Called only from an attached decode thread.
fn release_media_refs(env: &mut Env<'_>, media: &Media) {
    let _ = env.call_method(
        media.codec.as_obj(),
        jni_str!("release"),
        jni_sig!("()V"),
        &[],
    );
    clear(env);
    let _ = env.call_method(
        media.extractor.as_obj(),
        jni_str!("release"),
        jni_sig!("()V"),
        &[],
    );
    clear(env);
}

impl VideoDecoder for Reader {
    fn info(&self) -> VideoInfo {
        self.info
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        self.with_media(|reader, env, media| reader.seek_to(env, media, position))
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        self.with_media(|reader, env, media| reader.read_picture(env, media, rgba, false))
    }

    fn read_nv12(&mut self, nv12: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        self.with_media(|reader, env, media| reader.read_picture(env, media, nv12, true))
    }

    fn output_format(&self) -> PixelFormat {
        PixelFormat::Nv12
    }

    fn nv12_color(&self) -> YuvColor {
        let matrix = match self.matrix {
            Matrix::Bt709 => ColorMatrix::Bt709,
            Matrix::Bt601 => ColorMatrix::Bt601,
        };
        let range = match self.range {
            Range::Limited => ColorRange::Limited,
            Range::Full => ColorRange::Full,
        };
        YuvColor::new(matrix, range)
    }

    fn observe_shutdown(&mut self, flag: Arc<AtomicBool>) {
        self.gate.observe(flag);
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn attach_session(&mut self, session: PlaybackSession) {
        self.session = Some(session);
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn recover(&mut self) -> bool {
        // The worker calls this on the decode thread. The next read owns setup,
        // so clearing the cooldown is enough to retry on that same thread.
        self.state.lock().unwrap().failed_at = None;
        true
    }

    fn close(&mut self) {
        self.gate.close();
        let media = {
            let mut state = self.state.lock().unwrap();
            state.media.take()
        };
        if let Some(media) = media {
            // Cleanup attaches its own thread; the shutdown gate must not stop
            // it from releasing objects an earlier read created. Release the
            // native codec/extractor through a BORROWED reference first, so the
            // release() calls run even if VM lookup, attach, or try_unwrap fails
            // (previously those paths skipped release() and leaked the decoder).
            let released = vm().and_then(|vm| {
                vm.attach_current_thread(|env| -> jni::errors::Result<()> {
                    release_media_refs(env, &media);
                    Ok(())
                })
                .map_err(|error| {
                    VideoError::Source(format!("cannot reach the Android runtime: {error}"))
                })
            });
            if let Err(error) = &released {
                crate::diag(
                    "WARN",
                    "android.close",
                    &format!("could not attach to release the decoder: {error}"),
                );
            }
            drop(media);
        }
        let mut state = self.state.lock().unwrap();
        state.spill = None;
        state.failed_at = None;
        self.failure_reported_for = None;
    }
}

/// One plane's pixels, as the decoder laid them out.
struct Plane {
    address: *mut u8,
    capacity: usize,
    row_stride: usize,
    pixel_stride: usize,
}

fn plane_data(
    env: &mut Env<'_>,
    planes: &JObjectArray<'_, JObject<'_>>,
    index: usize,
) -> Result<Plane, VideoError> {
    let plane = planes
        .get_element(env, index)
        .map_err(|error| jni_error(env, "the picture holds no planes", error))?;
    let row_stride = env
        .call_method(&plane, jni_str!("getRowStride"), jni_sig!("()I"), &[])
        .and_then(jni::JValueOwned::i)
        .map_err(|error| jni_error(env, "the picture states no row stride", error))?;
    let pixel_stride = env
        .call_method(&plane, jni_str!("getPixelStride"), jni_sig!("()I"), &[])
        .and_then(jni::JValueOwned::i)
        .map_err(|error| jni_error(env, "the picture states no pixel stride", error))?;
    let buffer = env
        .call_method(
            &plane,
            jni_str!("getBuffer"),
            jni_sig!("()Ljava/nio/ByteBuffer;"),
            &[],
        )
        .and_then(jni::JValueOwned::l)
        .map_err(|error| jni_error(env, "the picture holds no pixels", error))?;
    let buffer = JByteBuffer::cast_local(env, buffer)
        .map_err(|error| jni_error(env, "the picture holds no pixels", error))?;
    let address = env
        .get_direct_buffer_address(&buffer)
        .map_err(|error| jni_error(env, "the picture holds no pixels", error))?;
    let capacity = env
        .get_direct_buffer_capacity(&buffer)
        .map_err(|error| jni_error(env, "the picture holds no pixels", error))?;
    if address.is_null() || row_stride <= 0 || pixel_stride <= 0 {
        return Err(VideoError::Decode(
            "the decoder laid its picture out in no readable form".to_owned(),
        ));
    }
    Ok(Plane {
        address,
        capacity,
        row_stride: row_stride as usize,
        pixel_stride: pixel_stride as usize,
    })
}

/// The coefficients that turn one picture's luma and chroma into red, green
/// and blue.
struct Colour {
    matrix: Matrix,
    range: Range,
}

impl Colour {
    fn convert(
        &self,
        luma: &Plane,
        blue: &Plane,
        red: &Plane,
        width: u32,
        height: u32,
        rgba: &mut [u8],
    ) -> Result<(), VideoError> {
        let luma_rows = height as usize;
        let luma_columns = width as usize;
        let chroma_rows = luma_rows.div_ceil(2);
        let chroma_columns = luma_columns.div_ceil(2);
        if !luma.holds(luma_rows, luma_columns)
            || !blue.holds(chroma_rows, chroma_columns)
            || !red.holds(chroma_rows, chroma_columns)
        {
            return Err(VideoError::Decode(
                "the decoder stated a picture smaller than its planes hold".to_owned(),
            ));
        }
        // SAFETY: each plane holds the rows read below, checked just above, and
        // the image stays open for the whole conversion.
        let luma_pixels = unsafe { std::slice::from_raw_parts(luma.address, luma.capacity) };
        // SAFETY: as above, for the plane that holds the blue difference.
        let blue_pixels = unsafe { std::slice::from_raw_parts(blue.address, blue.capacity) };
        // SAFETY: as above, for the plane that holds the red difference.
        let red_pixels = unsafe { std::slice::from_raw_parts(red.address, red.capacity) };
        let (red_green, green_red, green_blue, blue_green) = match self.matrix {
            // The studio range scales luma by 1.164, the full range by 1.
            Matrix::Bt709 => (459, 55, 136, 541),
            Matrix::Bt601 => (409, 100, 208, 516),
        };
        let base = match self.range {
            Range::Limited => 16,
            Range::Full => 0,
        };
        let scale = match self.range {
            Range::Limited => 298,
            Range::Full => 256,
        };
        let width_usize = width as usize;
        let row = width_usize * 4;
        for y in 0..luma_rows {
            let luma_row = &luma_pixels[y * luma.row_stride..];
            let blue_row = &blue_pixels[(y / 2) * blue.row_stride..];
            let red_row = &red_pixels[(y / 2) * red.row_stride..];
            let mut out = &mut rgba[y * row..][..row];
            for x in 0..width_usize {
                let tone = i32::from(luma_row[x * luma.pixel_stride]) - base;
                let cb = i32::from(blue_row[(x / 2) * blue.pixel_stride]) - 128;
                let cr = i32::from(red_row[(x / 2) * red.pixel_stride]) - 128;
                let green = (scale * tone - green_red * cb - green_blue * cr + 128) >> 8;
                let colour_red = (scale * tone + red_green * cr + 128) >> 8;
                let colour_blue = (scale * tone + blue_green * cb + 128) >> 8;
                out[0] = clamp(colour_red);
                out[1] = clamp(green);
                out[2] = clamp(colour_blue);
                out[3] = u8::MAX;
                out = &mut out[4..];
            }
        }
        Ok(())
    }
}

fn clamp(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

impl Plane {
    /// Whether the buffer holds `rows` rows of `columns` samples, which is the
    /// check the conversion reads through.
    fn holds(&self, rows: usize, columns: usize) -> bool {
        rows > 0
            && columns > 0
            && (rows - 1) * self.row_stride + columns * self.pixel_stride <= self.capacity
    }
}

/// Reads an `int` field straight off an object. `MediaCodec.BufferInfo` has
/// public fields such as `flags` and `size`, not `getInteger` methods.
fn int_value(
    env: &mut Env<'_>,
    object: &JObject<'_>,
    name: &'static jni::strings::JNIStr,
) -> Result<i32, jni::errors::Error> {
    env.get_field(object, name, jni_sig!("I"))
        .and_then(jni::JValueOwned::i)
}

/// Reads a `long` field straight off an object, for `BufferInfo.pts`.
fn long_value(
    env: &mut Env<'_>,
    object: &JObject<'_>,
    name: &'static jni::strings::JNIStr,
) -> Result<i64, jni::errors::Error> {
    env.get_field(object, name, jni_sig!("J"))
        .and_then(jni::JValueOwned::j)
}

/// An optional `MediaFormat` lookup. Absent keys throw a Java exception as a
/// matter of course; clear it before the next JNI call so it is not poisoned.
fn optional<T>(env: &mut Env<'_>, value: Result<T, jni::errors::Error>) -> Option<T> {
    match value {
        Ok(value) => Some(value),
        Err(error) => {
            let _ = error;
            dismiss(env);
            None
        }
    }
}

fn int_field(env: &mut Env<'_>, format: &JObject<'_>, name: &str) -> Option<i32> {
    let key = env.new_string(name);
    let key = optional(env, key)?;
    let value = env.call_method(
        format,
        jni_str!("getInteger"),
        jni_sig!("(Ljava/lang/String;)I"),
        &[JValue::Object(&key)],
    );
    let value = optional(env, value.and_then(jni::JValueOwned::i));
    release_local(env, &key);
    value
}

fn int_field_long(env: &mut Env<'_>, format: &JObject<'_>, name: &str) -> Option<i64> {
    let key = env.new_string(name);
    let key = optional(env, key)?;
    let value = env.call_method(
        format,
        jni_str!("getLong"),
        jni_sig!("(Ljava/lang/String;)J"),
        &[JValue::Object(&key)],
    );
    let value = optional(env, value.and_then(jni::JValueOwned::j));
    release_local(env, &key);
    value
}

fn float_field(env: &mut Env<'_>, format: &JObject<'_>, name: &str) -> Option<f32> {
    let key = env.new_string(name);
    let key = optional(env, key)?;
    let value = env.call_method(
        format,
        jni_str!("getFloat"),
        jni_sig!("(Ljava/lang/String;)F"),
        &[JValue::Object(&key)],
    );
    let value = optional(env, value.and_then(jni::JValueOwned::f));
    release_local(env, &key);
    value
}

fn string_field(env: &mut Env<'_>, format: &JObject<'_>, name: &str) -> Option<String> {
    let key = env.new_string(name);
    let key = optional(env, key)?;
    let value = env.call_method(
        format,
        jni_str!("getString"),
        jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
        &[JValue::Object(&key)],
    );
    let value = optional(env, value.and_then(jni::JValueOwned::l));
    let text = match value {
        Some(value) => {
            let cast = env.as_cast::<JString>(&value);
            let cast = optional(env, cast);
            let text = match cast {
                Some(cast) => {
                    let converted = cast.try_to_string(&*env);
                    optional(env, converted)
                }
                None => None,
            };
            release_local(env, &value);
            text
        }
        None => None,
    };
    release_local(env, &key);
    text
}

/// The codec's own constant values, read from its class once when the decoder
/// opens instead of on every picture.
#[derive(Clone, Copy)]
struct CodecFlags {
    end_of_stream: i32,
    codec_config: i32,
    format_changed: i32,
    buffers_changed: i32,
    try_again: i32,
}

impl CodecFlags {
    fn read(env: &mut Env<'_>) -> Self {
        // The values below are the ones the framework documents, and the class
        // supplies them again whenever it can be reached.
        let mut flags = Self {
            end_of_stream: 4,
            codec_config: 2,
            format_changed: -2,
            buffers_changed: -3,
            try_again: -1,
        };
        let Ok(class) = env.find_class(jni_str!("android/media/MediaCodec")) else {
            dismiss(env);
            return flags;
        };
        flags.end_of_stream = static_int(env, &class, jni_str!("BUFFER_FLAG_END_OF_STREAM"))
            .unwrap_or(flags.end_of_stream);
        flags.codec_config = static_int(env, &class, jni_str!("BUFFER_FLAG_CODEC_CONFIG"))
            .unwrap_or(flags.codec_config);
        flags.format_changed = static_int(env, &class, jni_str!("INFO_OUTPUT_FORMAT_CHANGED"))
            .unwrap_or(flags.format_changed);
        flags.buffers_changed = static_int(env, &class, jni_str!("INFO_OUTPUT_BUFFERS_CHANGED"))
            .unwrap_or(flags.buffers_changed);
        flags.try_again =
            static_int(env, &class, jni_str!("INFO_TRY_AGAIN_LATER")).unwrap_or(flags.try_again);
        release_local(env, &class);
        flags
    }
}

fn static_int(
    env: &mut Env<'_>,
    class: &jni::objects::JClass<'_>,
    name: &'static jni::strings::JNIStr,
) -> Option<i32> {
    let value = env.get_static_field(class, name, jni_sig!("I"));
    optional(env, value.and_then(jni::JValueOwned::i))
}

fn duration_from(micros: Option<i64>) -> Option<Duration> {
    micros
        .filter(|micros| *micros > 0)
        .map(|micros| Duration::from_micros(micros as u64))
}

/// Clears the Java exception this call left and names the failure as one the
/// source caused.
fn source_error(env: &mut Env<'_>, what: &str, error: jni::errors::Error) -> VideoError {
    clear(env);
    VideoError::Source(format!("{what}: {error}"))
}

/// As [`source_error`], for a failure the decoder reported.
fn jni_error(env: &mut Env<'_>, what: &str, error: jni::errors::Error) -> VideoError {
    clear(env);
    VideoError::Decode(format!("{what}: {error}"))
}

fn clear(env: &mut Env<'_>) {
    if env.exception_check() {
        env.exception_describe();
        env.exception_clear();
    }
}

/// Quietly clears a pending Java exception left by an optional lookup. Unlike
/// [`clear`], it does not describe the exception to the log: a missing optional
/// `MediaFormat` key is normal, not a fault worth reporting.
fn dismiss(env: &mut Env<'_>) {
    if env.exception_check() {
        env.exception_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_flag_starts_open_and_closes() {
        let mut gate = StopFlag::new();
        assert!(!gate.stopped());
        gate.close();
        assert!(gate.stopped());
    }

    #[test]
    fn stop_flag_follows_the_player_shutdown_signal() {
        let mut gate = StopFlag::new();
        let shutdown = Arc::new(AtomicBool::new(false));
        gate.observe(Arc::clone(&shutdown));
        assert!(!gate.stopped());
        shutdown.store(true, Ordering::Release);
        assert!(gate.stopped());
    }

    #[test]
    fn cooldown_allows_a_never_failed_reader_and_expired_failures() {
        assert!(cooldown_elapsed(None));
        assert!(!cooldown_elapsed(Some(Instant::now())));
        assert!(cooldown_elapsed(Some(
            Instant::now()
                .checked_sub(SETUP_COOLDOWN + Duration::from_millis(1))
                .unwrap()
        )));
    }

    #[test]
    fn duration_from_ignores_missing_and_non_positive_values() {
        assert_eq!(duration_from(None), None);
        assert_eq!(duration_from(Some(0)), None);
        assert_eq!(duration_from(Some(-1)), None);
        assert_eq!(duration_from(Some(2_000_000)), Some(Duration::from_secs(2)));
    }

    #[test]
    fn clamp_keeps_colours_in_byte_range() {
        assert_eq!(clamp(-7), 0);
        assert_eq!(clamp(19), 19);
        assert_eq!(clamp(300), u8::MAX);
    }

    #[test]
    fn plane_holds_checks_rows_columns_strides_and_capacity() {
        let plane = Plane {
            address: std::ptr::null_mut(),
            capacity: 16,
            row_stride: 4,
            pixel_stride: 1,
        };
        assert!(plane.holds(4, 4));
        assert!(!plane.holds(5, 4));
        assert!(!plane.holds(0, 4));
        assert!(!plane.holds(4, 0));

        let strided = Plane {
            address: std::ptr::null_mut(),
            capacity: 20,
            row_stride: 6,
            pixel_stride: 2,
        };
        // (3 - 1) * 6 + 4 * 2 == 20 exactly; one more row no longer fits.
        assert!(strided.holds(3, 4));
        assert!(!strided.holds(4, 4));
    }
}
