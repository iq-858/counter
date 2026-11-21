//! AVPlayer-backed decoder for media that `AVAssetReader` cannot read.
//!
//! `AVAssetReader` opens QuickTime/MP4 files but not MPEG-TS, which is what
//! most live HLS channels serve. AVFoundation's playback stack (`AVPlayer`)
//! does read TS (and HLS) through `AVPlayerItemVideoOutput`. This module is the
//! narrow bridge: it plays one joined file and hands decoded BGRA frames to the
//! crate's RGBA path. The local-file decoder in `apple.rs` is unchanged, so
//! offline clips keep the proven reader path.
//!
//! Everything here runs on the decode thread. `AVPlayer` drives its own internal
//! machinery; this decoder only polls `hasNewPixelBufferForItemTime` and copies
//! the pixel buffer, so no run loop or UI-thread work is needed.

#![expect(
    unsafe_code,
    reason = "AVFoundation is an Objective-C framework; every call crosses an FFI boundary"
)]
#![allow(
    deprecated,
    reason = "the synchronous item/status accessors are the only ones usable without a run loop"
)]

use std::path::Path;
use std::time::{Duration, Instant};

use objc2::{AllocAnyThread, rc::Retained, runtime::AnyObject};
use objc2_av_foundation::{AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVPlayerItemVideoOutput};
use objc2_core_foundation::CMTime;
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferPixelFormatTypeKey,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

use crate::{DecodedFrame, PixelFormat, VideoError, VideoInfo, pixels::bgra_to_rgba};

/// The four-character code for 32-bit BGRA.
const BGRA: u32 = 0x4247_5241;
/// How long to wait for the item to become ready before reporting an error.
const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// A decoder that plays a file through AVPlayer and reads its frames.
pub(crate) struct AvPlayerDecoder {
    _player: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    output: Retained<AVPlayerItemVideoOutput>,
    info: VideoInfo,
    started: Instant,
    first_pts: Option<Duration>,
}

impl AvPlayerDecoder {
    /// Opens a joined HLS window file through the AVPlayer playback stack.
    pub(crate) fn open(path: &Path) -> Result<Self, VideoError> {
        // The caller's `TempFile` owns the path for at least as long as this
        // decoder, so holding only the URL is enough; macOS/iOS AVPlayer keeps
        // its own file handle while the item is live.
        let url = NSURL::from_file_path(path).map_err(|_| {
            VideoError::Decode("the joined HLS window path is not a file URL".to_owned())
        })?;
        let item = unsafe { AVPlayerItem::initWithURL(AVPlayerItem::alloc(), &url) };
        let player = unsafe { AVPlayer::initWithPlayerItem(AVPlayer::alloc(), &item) };

        // Ask for BGRA: one conversion, on this thread, into the crate's RGBA.
        let key: &NSString =
            unsafe { &*(kCVPixelBufferPixelFormatTypeKey as *const _ as *const NSString) };
        let value: &AnyObject = &NSNumber::new_u32(BGRA);
        let settings: Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_slices(&[key], &[value]);
        let output = unsafe {
            AVPlayerItemVideoOutput::initWithPixelBufferAttributes(
                AVPlayerItemVideoOutput::alloc(),
                Some(settings.as_ref()),
            )
        };
        unsafe { item.addOutput(&output) };
        unsafe { player.play() };

        // AVFoundation reports the natural size after the item becomes ready.
        // Until then the player shows Loading; a short wait keeps a bad file
        // from blocking the decode thread indefinitely.
        let began = Instant::now();
        while began.elapsed() < READY_TIMEOUT {
            let status = unsafe { item.status() };
            if status == AVPlayerItemStatus::ReadyToPlay {
                break;
            }
            if status == AVPlayerItemStatus::Failed {
                let reason = unsafe { item.error() }
                    .map(|error| error.localizedDescription().to_string())
                    .unwrap_or_else(|| "AVPlayer could not open the media".to_owned());
                return Err(VideoError::Decode(reason));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let size = unsafe { item.asset().naturalSize() };
        let info = VideoInfo {
            width: size.width.round().max(0.0) as u32,
            height: size.height.round().max(0.0) as u32,
            duration: None,
            frame_rate: None,
            rotation: Default::default(),
        };
        Ok(Self {
            _player: player,
            item,
            output,
            info,
            started: Instant::now(),
            first_pts: None,
        })
    }

    /// The item time for "now", which tracks wall time since playback began.
    fn item_time(&self) -> CMTime {
        use objc2_core_media::CMTimeMake;
        let elapsed = self.started.elapsed();
        CMTimeMake(elapsed.as_secs() as i64 * 1_000_000, 1_000_000)
    }
}

impl crate::VideoDecoder for AvPlayerDecoder {
    fn info(&self) -> VideoInfo {
        self.info
    }

    fn output_format(&self) -> PixelFormat {
        PixelFormat::Rgba8
    }

    fn is_live(&self) -> bool {
        true
    }

    fn is_seekable(&self) -> bool {
        false
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        let _ = position;
        Err(VideoError::UnsupportedOperation {
            operation: "seeking a live HLS window",
        })
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        let time = self.item_time();
        // Wait briefly for a frame to become available at the current time.
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            if unsafe { self.output.hasNewPixelBufferForItemTime(time) } {
                break;
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let Some(buffer) = (unsafe {
            self.output
                .copyPixelBufferForItemTime_itemTimeForDisplay(time, std::ptr::null_mut())
        }) else {
            return Ok(None);
        };
        let flags = CVPixelBufferLockFlags::ReadOnly;
        if unsafe { CVPixelBufferLockBaseAddress(&buffer, flags) } != 0 {
            return Ok(None);
        }
        let width = unsafe { CVPixelBufferGetWidth(&buffer) } as u32;
        let height = unsafe { CVPixelBufferGetHeight(&buffer) } as u32;
        let stride = unsafe { CVPixelBufferGetBytesPerRow(&buffer) };
        let base = unsafe { CVPixelBufferGetBaseAddress(&buffer) }.cast::<u8>();
        let row = width as usize * 4;
        let result = if !base.is_null() && stride >= row && width > 0 && height > 0 {
            let needed = row * height as usize;
            if rgba.len() != needed {
                rgba.resize(needed, 0);
            }
            for y in 0..height as usize {
                let source = unsafe { std::slice::from_raw_parts(base.add(y * stride), row) };
                bgra_to_rgba(source, &mut rgba[y * row..][..row]);
            }
            let pts = self.first_pts.unwrap_or(Duration::ZERO);
            self.first_pts = Some(pts + Duration::from_millis(33));
            Some(DecodedFrame { pts, width, height })
        } else {
            None
        };
        unsafe { CVPixelBufferUnlockBaseAddress(&buffer, flags) };
        Ok(result)
    }
}
