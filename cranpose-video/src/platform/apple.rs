//! AVFoundation: the framework QuickTime and Safari decode with.
//!
//! An `AVAssetReader` demuxes the file and a track output decodes the video
//! stream. NV12 is requested first because it is the framework's native output;
//! BGRA is the fallback for tracks that cannot produce planes. The reader holds
//! no clock of its own, so a seek rebuilds it at the new position and decoding
//! resumes from the key frame before it.
#![expect(
    unsafe_code,
    reason = "AVFoundation is an Objective-C framework, so every call crosses an FFI boundary"
)]
#![allow(
    deprecated,
    reason = "the synchronous track and duration accessors are the only ones a decode thread can wait on without a run loop"
)]

use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use objc2::{AllocAnyThread, rc::Retained, runtime::AnyObject};
use objc2_av_foundation::{
    AVAssetReader, AVAssetReaderStatus, AVAssetReaderTrackOutput, AVAssetTrack, AVMediaTypeVideo,
    AVURLAsset,
};
use objc2_core_foundation::CGAffineTransform;
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeRange};
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeight, CVPixelBufferGetPlaneCount,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferPixelFormatTypeKey,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

use super::super::temp_file::TempFile;
use crate::{
    DecodedFrame, FrameRate, PixelFormat, Rotation, VideoDecoder, VideoError, VideoInfo,
    VideoSource, YuvColor, pixels::bgra_to_rgba,
};

pub(crate) const NAME: &str = "AVFoundation";
pub(crate) const SUPPORTED: bool = true;

/// The timescale a position is stated in, in ticks per second.
const TIMESCALE: i32 = 1_000_000;

/// How many consecutive samples may arrive without a usable picture before the
/// stream is declared stuck. This bounds malformed demux output, not decode
/// time: a slow but healthy reader is never timed out.
const MAX_UNUSABLE_SAMPLES: usize = 8;

pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn VideoDecoder>, VideoError> {
    let spill = match source {
        VideoSource::Bytes(bytes) => Some(TempFile::write(bytes)?),
        VideoSource::Path(_) | VideoSource::Url(_) => None,
    };
    let url = match &spill {
        Some(file) => file_url(file.path())?,
        None => source_url(source)?,
    };
    // SAFETY: the URL outlives the asset, which keeps its own reference to it.
    let asset = unsafe { AVURLAsset::initWithURL_options(AVURLAsset::alloc(), &url, None) };
    // SAFETY: the media type is the framework's own constant.
    let Some(media_type) = (unsafe { AVMediaTypeVideo }) else {
        return Err(VideoError::NoVideoTrack);
    };
    // SAFETY: the asset is open and the media type is the framework's constant.
    let video_track = unsafe { asset.tracksWithMediaType(media_type) }
        .firstObject()
        .ok_or(VideoError::NoVideoTrack)?;
    // SAFETY: the track belongs to the open asset.
    let size = unsafe { video_track.naturalSize() };
    // SAFETY: as above.
    let transform = unsafe { video_track.preferredTransform() };
    // SAFETY: as above.
    let rate = unsafe { video_track.nominalFrameRate() };
    // SAFETY: as above.
    let stated = unsafe { asset.duration() };

    let info = VideoInfo {
        width: size.width.round().max(0.0) as u32,
        height: size.height.round().max(0.0) as u32,
        duration: stated_seconds(stated).map(Duration::from_secs_f64),
        frame_rate: FrameRate::approx(rate),
        rotation: rotation(&transform),
    };

    let mut reader = Reader {
        asset,
        track: video_track,
        info,
        playback: None,
        start: Duration::ZERO,
        finished: false,
        spill,
        format: PixelFormat::Nv12,
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        session: None,
        shutdown: None,
    };
    reader.open_at(Duration::ZERO)?;
    Ok(Box::new(reader))
}

fn file_url(path: &std::path::Path) -> Result<Retained<NSURL>, VideoError> {
    NSURL::from_file_path(path)
        .ok_or_else(|| VideoError::Source("the file path is not valid".to_owned()))
}

fn source_url(source: &VideoSource) -> Result<Retained<NSURL>, VideoError> {
    match source {
        VideoSource::Path(path) => file_url(path),
        VideoSource::Url(url) => {
            let text = NSString::from_str(url);
            NSURL::URLWithString(&text)
                .ok_or_else(|| VideoError::Source(format!("{url} is not a valid URL")))
        }
        VideoSource::Bytes(_) => Err(VideoError::Source(
            "the clip's bytes have no URL".to_owned(),
        )),
    }
}

fn stated_seconds(time: CMTime) -> Option<f64> {
    // SAFETY: the value is a plain structure the framework filled.
    let seconds = unsafe { time.seconds() };
    (seconds.is_finite() && seconds > 0.0).then_some(seconds)
}

/// The quarter turn the track's preferred transform applies.
///
/// The transform's first two values are the cosine and sine of the rotation,
/// so the turn reads as their angle.
fn rotation(transform: &CGAffineTransform) -> Rotation {
    Rotation::from_degrees(transform.b.atan2(transform.a).to_degrees())
}

/// The reader and the output that decodes into it, from one position on.
struct Playback {
    reader: Retained<AVAssetReader>,
    output: Retained<AVAssetReaderTrackOutput>,
}

struct Reader {
    asset: Retained<AVURLAsset>,
    track: Retained<AVAssetTrack>,
    info: VideoInfo,
    playback: Option<Playback>,
    start: Duration,
    finished: bool,
    spill: Option<TempFile>,
    /// The layout the track output decodes into: the decoder's own NV12 planes
    /// when the framework offers them, BGRA when it does not.
    format: PixelFormat,
    /// The player's recovery handle, handed over before this decoder moves to
    /// the decode thread. A stream that fails while decoding reports here so
    /// the worker rebuilds the reader instead of leaving a dead channel.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    session: Option<crate::PlaybackSession>,
    /// The player's close signal. Checked between samples so a reader that
    /// keeps handing out pictures after a switch cannot outlive its player.
    shutdown: Option<Arc<AtomicBool>>,
}

// SAFETY: AVFoundation objects are not thread bound. The reader is created on
// the thread that opens the source and every later call comes from the decode
// thread, one at a time.
unsafe impl Send for Reader {}

impl Reader {
    /// Builds a reader that decodes from `position` on.
    fn open_at(&mut self, position: Duration) -> Result<(), VideoError> {
        // Build the replacement reader *before* disturbing the current one.
        // The old AVAssetReader keeps its place until the new one exists, so a
        // failed attach cannot leave the decoder with no playback at all: that
        // made `pull` return `Ok(None)` and the worker end a live feed.
        // A track output that decodes straight to NV12 hands over the decoder's
        // own planes, one and a half bytes a pixel, instead of the four the
        // BGRA conversion produces. `attach` builds a fresh reader each call,
        // so a failed NV12 attempt is safe to retry as BGRA.
        let attached = self
            .attach(position, PixelFormat::Nv12)
            .or_else(|_| self.attach(position, PixelFormat::Rgba8));
        let (reader, output, format) = attached?;
        // The new reader is ready: only now cancel the old one. Dropping a
        // running AVAssetReader without cancelling it leaves decode work queued
        // on internal queues, so a seek would briefly read two media positions.
        if let Some(playback) = self.playback.take() {
            // SAFETY: the reader is being replaced and this is its last use.
            unsafe { playback.reader.cancelReading() };
        }
        self.start = position;
        self.finished = false;
        self.format = format;
        self.playback = Some(Playback { reader, output });
        Ok(())
    }

    /// A reader and its track output, set up for `format`, ready to read from
    /// `position`.
    fn attach(
        &self,
        position: Duration,
        format: PixelFormat,
    ) -> Result<
        (
            Retained<AVAssetReader>,
            Retained<AVAssetReaderTrackOutput>,
            PixelFormat,
        ),
        VideoError,
    > {
        // SAFETY: the asset is open and an asset reader needs no other setup.
        let reader =
            unsafe { AVAssetReader::initWithAsset_error(AVAssetReader::alloc(), &self.asset) }
                .map_err(|error| VideoError::Source(describe(&error)))?;
        // `nil` output settings ask AVFoundation to vend the samples in the
        // track's own format — the one decoder output it is guaranteed to have.
        // Per Apple's AVAssetReaderTrackOutput docs, requesting a pixel format
        // the track cannot produce makes init/canAddOutput fail. Asking for the
        // original format is the universal fallback.
        let settings = settings(format);
        // SAFETY: the track belongs to the open asset, and the settings hold
        // only constant keys and an integer.
        let make_output = |settings: Option<Retained<NSDictionary<NSString, AnyObject>>>| unsafe {
            AVAssetReaderTrackOutput::initWithTrack_outputSettings(
                AVAssetReaderTrackOutput::alloc(),
                &self.track,
                settings.as_ref(),
            )
        };
        let output = make_output(Some(settings)).or_else(|| make_output(None));
        // SAFETY: the reader is open and the output is not attached yet.
        // `canAddOutput` must be consulted BEFORE `addOutput`: asking after the
        // output is attached reports the wrong result and made valid tracks
        // look refused.
        let output = output.ok_or_else(|| {
            VideoError::Decode("AVFoundation cannot create the video track output".to_owned())
        })?;
        let added = unsafe { reader.canAddOutput(&output) };
        if !added {
            return Err(VideoError::Decode(
                "AVFoundation refused the video track output".to_owned(),
            ));
        }
        unsafe { reader.addOutput(&output) };
        if position > Duration::ZERO {
            // SAFETY: the range states the position in the media's own time,
            // and the reader is not reading yet.
            unsafe { reader.setTimeRange(range(position)) };
        }
        // SAFETY: the output is attached and the reader is not reading yet.
        let reading = unsafe { reader.startReading() };
        if !reading {
            // SAFETY: the reader failed, so it holds the reason.
            let reason = unsafe { reader.error() };
            return Err(reason.map_or_else(
                || VideoError::Decode("AVFoundation cannot read the track".to_owned()),
                |error| VideoError::Source(describe(&error)),
            ));
        }
        Ok((reader, output, format))
    }

    /// Copies one sample's picture into `rgba` as top-down RGBA8 rows.
    fn copy(&self, sample: &CMSampleBuffer, rgba: &mut Vec<u8>) -> Option<DecodedFrame> {
        // SAFETY: the sample carries the picture the reader decoded.
        let image = unsafe { sample.image_buffer() }?;
        let buffer: &objc2_core_video::CVPixelBuffer = &image;
        let flags = CVPixelBufferLockFlags::ReadOnly;
        // SAFETY: the lock holds the pixels steady for the copy and is released
        // on every path.
        let locked = unsafe { CVPixelBufferLockBaseAddress(buffer, flags) } != 0;
        if locked {
            return None;
        }
        let width = CVPixelBufferGetWidth(buffer) as u32;
        let height = CVPixelBufferGetHeight(buffer) as u32;
        let stride = CVPixelBufferGetBytesPerRow(buffer);
        let base = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
        let row = width as usize * 4;
        let needed = row * height as usize;
        if base.is_null() || stride < row {
            // SAFETY: the lock was taken above and is released here.
            unsafe { CVPixelBufferUnlockBaseAddress(buffer, flags) };
            return None;
        }
        if rgba.len() != needed {
            rgba.resize(needed, 0);
        }
        for y in 0..height as usize {
            // SAFETY: the base address points at `height` rows of `stride`
            // bytes, each at least a row of pixels wide, and the lock holds
            // them until it is released below.
            let source = unsafe { std::slice::from_raw_parts(base.add(y * stride), row) };
            bgra_to_rgba(source, &mut rgba[y * row..][..row]);
        }
        // SAFETY: the lock was taken above on this same buffer.
        unsafe { CVPixelBufferUnlockBaseAddress(buffer, flags) };
        // SAFETY: the sample states when the picture is shown.
        let seconds = unsafe { sample.presentation_time_stamp().seconds() };
        let pts = if seconds.is_finite() && seconds > 0.0 {
            Duration::from_secs_f64(seconds)
        } else {
            self.start
        };
        Some(DecodedFrame { pts, width, height })
    }

    /// Copies one sample's NV12 planes into unpadded rows, luma then interleaved
    /// chroma, which is the layout every GPU samples as two textures.
    fn copy_planes(&self, sample: &CMSampleBuffer, nv12: &mut Vec<u8>) -> Option<DecodedFrame> {
        // SAFETY: the sample carries the picture the reader decoded.
        let image = unsafe { sample.image_buffer() }?;
        let buffer: &objc2_core_video::CVPixelBuffer = &image;
        let flags = CVPixelBufferLockFlags::ReadOnly;
        // SAFETY: the lock holds the planes steady for the copy and is released
        // on every path.
        if unsafe { CVPixelBufferLockBaseAddress(buffer, flags) } != 0 {
            return None;
        }
        let width = CVPixelBufferGetWidth(buffer) as u32;
        let height = CVPixelBufferGetHeight(buffer) as u32;
        // SAFETY: the plane count is what the buffer states about itself.
        let planes = unsafe { CVPixelBufferGetPlaneCount(buffer) };
        if planes < 2 || width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            // SAFETY: the lock was taken above.
            unsafe { CVPixelBufferUnlockBaseAddress(buffer, flags) };
            return None;
        }
        // SAFETY: the plane indices are below the count the buffer states, and
        // the addresses stay valid until the lock is released.
        let (luma, chroma, luma_stride, chroma_stride) = unsafe {
            (
                CVPixelBufferGetBaseAddressOfPlane(buffer, 0).cast::<u8>(),
                CVPixelBufferGetBaseAddressOfPlane(buffer, 1).cast::<u8>(),
                CVPixelBufferGetBytesPerRowOfPlane(buffer, 0),
                CVPixelBufferGetBytesPerRowOfPlane(buffer, 1),
            )
        };
        let row = width as usize;
        let chroma_row = row;
        let needed = row * height as usize * 3 / 2;
        if luma.is_null() || chroma.is_null() || luma_stride < row || chroma_stride < chroma_row {
            // SAFETY: the lock was taken above.
            unsafe { CVPixelBufferUnlockBaseAddress(buffer, flags) };
            return None;
        }
        if nv12.len() != needed {
            nv12.resize(needed, 0);
        }
        let chroma_base = row * height as usize;
        for y in 0..height as usize {
            // SAFETY: the plane holds `height` rows of at least `row` bytes,
            // and the lock holds them until it is released below.
            let source = unsafe { std::slice::from_raw_parts(luma.add(y * luma_stride), row) };
            nv12[y * row..][..row].copy_from_slice(source);
        }
        for y in 0..(height / 2) as usize {
            // SAFETY: the chroma plane holds half as many rows of the same
            // width, each at least `chroma_row` bytes.
            let source =
                unsafe { std::slice::from_raw_parts(chroma.add(y * chroma_stride), chroma_row) };
            nv12[chroma_base + y * chroma_row..][..chroma_row].copy_from_slice(source);
        }
        // SAFETY: the lock was taken above on this same buffer.
        unsafe { CVPixelBufferUnlockBaseAddress(buffer, flags) };
        // SAFETY: the sample states when the picture is shown.
        let seconds = unsafe { sample.presentation_time_stamp().seconds() };
        let pts = if seconds.is_finite() && seconds > 0.0 {
            Duration::from_secs_f64(seconds)
        } else {
            self.start
        };
        Some(DecodedFrame { pts, width, height })
    }
}

impl VideoDecoder for Reader {
    fn info(&self) -> VideoInfo {
        self.info
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        self.open_at(position)
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        self.pull(false, rgba)
    }

    fn read_nv12(&mut self, nv12: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        self.pull(true, nv12)
    }

    fn output_format(&self) -> PixelFormat {
        self.format
    }

    fn nv12_color(&self) -> YuvColor {
        YuvColor::for_size(self.info.width, self.info.height)
    }

    fn close(&mut self) {
        if let Some(playback) = self.playback.take() {
            // SAFETY: the reader is reading and this is the only handle to it.
            unsafe { playback.reader.cancelReading() };
        }
        self.spill = None;
    }

    fn observe_shutdown(&mut self, flag: Arc<AtomicBool>) {
        self.shutdown = Some(flag);
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn attach_session(&mut self, session: crate::PlaybackSession) {
        self.session = Some(session);
    }

    /// Rebuilds the asset reader on the decode thread after a stream failure.
    ///
    /// AVFoundation readers do not resume once `copyNextSampleBuffer` reports a
    /// failure, so retrying must start a fresh reader at the last good position.
    /// Returning false keeps the worker's cooldown-driven retry alive; nothing
    /// here touches the composition thread or changes the source.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn recover(&mut self) -> bool {
        let position = self.start;
        match self.open_at(position) {
            Ok(()) => {
                crate::diag("INFO", "apple.recover", "rebuilt the asset reader");
                true
            }
            Err(error) => {
                let reason = format!("cannot rebuild the asset reader: {error:?}");
                crate::diag("WARN", "apple.recover", &reason);
                false
            }
        }
    }
}

impl Reader {
    /// The next sample's picture, in the layout the output decodes into.
    fn pull(
        &mut self,
        native: bool,
        buffer: &mut Vec<u8>,
    ) -> Result<Option<DecodedFrame>, VideoError> {
        if self.finished {
            return Ok(None);
        }
        // A sample that carries no usable picture is not the end of the media:
        // some tracks interleave empty buffers. A short run is normal, an
        // unbounded run is a stuck stream, so it is bounded here rather than
        // waiting for a wall-clock timeout that would punish a slow decoder.
        let mut unusable = 0usize;
        loop {
            if self
                .shutdown
                .as_ref()
                .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
            {
                return Ok(None);
            }
            let Some(playback) = self.playback.as_ref() else {
                return Ok(None);
            };
            // SAFETY: the reader is reading and the output is attached to it.
            match unsafe { playback.output.copyNextSampleBuffer() } {
                Some(sample) => {
                    let frame = if native {
                        self.copy_planes(&sample, buffer)
                    } else {
                        self.copy(&sample, buffer)
                    };
                    if let Some(frame) = frame {
                        return Ok(Some(frame));
                    }
                    unusable += 1;
                    if unusable >= MAX_UNUSABLE_SAMPLES {
                        // SAFETY: the reader is still the one being read.
                        let status = unsafe { playback.reader.status() };
                        if status == AVAssetReaderStatus::Completed {
                            self.finished = true;
                            return Ok(None);
                        }
                        let reason = unsafe { playback.reader.error() }.map_or_else(
                            || {
                                format!(
                                    "AVFoundation produced {unusable} samples with no picture (status {status:?})"
                                )
                            },
                            |error| describe(&error),
                        );
                        self.report_stream_failure(&reason);
                        return Err(VideoError::Decode(reason));
                    }
                }
                None => {
                    // SAFETY: the reader stopped producing samples.
                    let status = unsafe { playback.reader.status() };
                    if status == AVAssetReaderStatus::Completed {
                        self.finished = true;
                        return Ok(None);
                    }
                    // SAFETY: a failed reader holds the reason it stopped.
                    let reason = unsafe { playback.reader.error() }.map_or_else(
                        || format!("AVFoundation stopped with status {status:?}"),
                        |error| describe(&error),
                    );
                    self.report_stream_failure(&reason);
                    return Err(VideoError::Decode(reason));
                }
            }
        }
    }

    /// Reports a stream failure to the shared recovery session before the
    /// worker marks the player failed, so a scheduled rebuild stays armed.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn report_stream_failure(&self, reason: &str) {
        crate::diag("WARN", "apple.stream", reason);
        if let Some(session) = &self.session {
            session.failed("apple.stream", reason.to_owned());
        }
    }

    #[cfg(not(all(not(target_os = "linux"), not(target_arch = "wasm32"))))]
    fn report_stream_failure(&self, reason: &str) {
        crate::diag("WARN", "apple.stream", reason);
    }
}

/// The output settings that ask for `format`, the conversion the framework does
/// fastest.
fn settings(format: PixelFormat) -> Retained<NSDictionary<NSString, AnyObject>> {
    // SAFETY: a `CFString` and an `NSString` are the same object, and the key
    // is the framework's own constant.
    let key: &NSString =
        unsafe { &*(kCVPixelBufferPixelFormatTypeKey as *const _ as *const NSString) };
    let value: &AnyObject = &NSNumber::new_u32(match format {
        PixelFormat::Nv12 => NV12,
        PixelFormat::Rgba8 => BGRA,
    });
    NSDictionary::from_slices(&[key], &[value])
}

/// The four character code of `kCVPixelFormatType_32BGRA`.
const BGRA: u32 = 0x4247_5241;

/// The four character code of `kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange`,
/// which is the planar format every Apple decoder produces natively.
const NV12: u32 = 0x3432_3076;

fn range(position: Duration) -> CMTimeRange {
    let start = CMTime {
        value: i64::try_from(position.as_micros()).unwrap_or(i64::MAX),
        timescale: TIMESCALE,
        flags: objc2_core_media::CMTimeFlags(1),
        epoch: 0,
    };
    CMTimeRange {
        start,
        duration: CMTime {
            value: 0,
            timescale: 0,
            flags: objc2_core_media::CMTimeFlags(0),
            epoch: 0,
        },
    }
}

fn describe(error: &objc2_foundation::NSError) -> String {
    error.localizedDescription().to_string()
}
