//! AVPlayer-backed decoder for media `AVAssetReader` cannot read (MPEG-TS).
//!
//! AVPlayer and AVPlayerItem are main-thread-only in objc2. Creation and frame
//! extraction are dispatched to the main queue with `dispatch2`, mirroring
//! Cranpose's own iOS media player. The objects are held in `SendRetained`
//! wrappers so the decoder can travel to the crate's decode thread; every
//! method call still hops back to the main thread.

#![expect(
    unsafe_code,
    reason = "AVFoundation is Objective-C; every call crosses an FFI boundary"
)]
#![allow(
    deprecated,
    reason = "the synchronous item/status accessors are the only ones usable without a run loop"
)]

use std::{
    path::Path,
    ptr,
    time::{Duration, Instant},
};

use dispatch2::DispatchQueue;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, rc::Retained, runtime::AnyObject};
use objc2_av_foundation::{AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVPlayerItemVideoOutput};
use objc2_core_media::{CMTime, CMTimeMake};
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferPixelFormatTypeKey,
};
use objc2_foundation::{NSDictionary, NSNotification, NSNumber, NSString, NSURL};

use crate::{DecodedFrame, PixelFormat, VideoError, VideoInfo, pixels::bgra_to_rgba};

const BGRA: u32 = 0x4247_5241;
const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// A `Retained` that is safe to move to the decode thread. All its uses go
/// through `on_main`, so no ObjC message is sent from the wrong thread.
struct SendRetained<T: ?Sized>(Retained<T>);
unsafe impl<T: ?Sized> Send for SendRetained<T> {}
unsafe impl<T: ?Sized> Sync for SendRetained<T> {}

fn on_main<R: Send>(action: impl FnOnce(MainThreadMarker) -> R + Send) -> R {
    if let Some(mtm) = MainThreadMarker::new() {
        return action(mtm);
    }
    let mut landed = None;
    DispatchQueue::main().exec_sync(|| {
        let mtm = MainThreadMarker::new().expect("the main queue is the main thread");
        landed = Some(action(mtm));
    });
    landed.expect("the main queue ran the action")
}

/// A decoder that plays a joined HLS window through AVPlayer and reads BGRA.
pub(crate) struct AvPlayerDecoder {
    player: SendRetained<AVPlayer>,
    item: SendRetained<AVPlayerItem>,
    output: SendRetained<AVPlayerItemVideoOutput>,
    info: VideoInfo,
    started: Instant,
    first_pts: Option<Duration>,
}

impl AvPlayerDecoder {
    pub(crate) fn open(path: &Path) -> Result<Self, VideoError> {
        let path = path.to_path_buf();
        let setup: Result<
            (
                Retained<AVPlayer>,
                Retained<AVPlayerItem>,
                Retained<AVPlayerItemVideoOutput>,
            ),
            String,
        > = on_main(move |mtm| {
            let url = NSURL::from_file_path(&path)
                .ok_or_else(|| "the joined HLS window path is not a file URL".to_owned())?;
            let item = unsafe { AVPlayerItem::initWithURL(AVPlayerItem::alloc(mtm), &url) };
            let player = unsafe { AVPlayer::initWithPlayerItem(AVPlayer::alloc(mtm), Some(&item)) };
            let key: &NSString =
                unsafe { &*(kCVPixelBufferPixelFormatTypeKey as *const _ as *const NSString) };
            let value: &AnyObject = &NSNumber::new_u32(BGRA);
            let settings: Retained<NSDictionary<NSString, AnyObject>> =
                NSDictionary::from_slices(&[key], &[value]);
            let output = unsafe {
                AVPlayerItemVideoOutput::initWithPixelBufferAttributes(
                    AVPlayerItemVideoOutput::alloc(mtm),
                    Some(settings.as_ref()),
                )
            };
            unsafe { item.addOutput(&output) };
            unsafe { player.play() };
            Ok((player, item, output))
        });
        let (player, item, output) = setup.map_err(VideoError::Decode)?;

        // Wait for the item to become ready on the main thread.
        let began = Instant::now();
        while began.elapsed() < READY_TIMEOUT {
            let status = on_main(move |_| unsafe { item.status() });
            if status == AVPlayerItemStatus::ReadyToPlay {
                break;
            }
            if status == AVPlayerItemStatus::Failed {
                let msg = on_main(move |_| {
                    unsafe { item.error() }
                        .map(|e| e.localizedDescription().to_string())
                        .unwrap_or_else(|| "AVPlayer could not open the media".to_owned())
                });
                return Err(VideoError::Decode(msg));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let size = on_main(move |_| unsafe { item.asset().naturalSize() });
        let info = VideoInfo {
            width: size.width.round().max(0.0) as u32,
            height: size.height.round().max(0.0) as u32,
            duration: None,
            frame_rate: None,
            rotation: Default::default(),
        };
        Ok(Self {
            player: SendRetained(player),
            item: SendRetained(item),
            output: SendRetained(output),
            info,
            started: Instant::now(),
            first_pts: None,
        })
    }

    fn item_time(&self) -> CMTime {
        let elapsed = self.started.elapsed();
        unsafe { CMTimeMake(elapsed.as_secs() as i64 * 1_000_000, 1_000_000) }
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

    fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
        Err(VideoError::UnsupportedOperation {
            operation: "seeking a live HLS window",
        })
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        let time = self.item_time();
        let output_ptr = &self.output.0 as *const Retained<AVPlayerItemVideoOutput> as usize;
        let deadline = Instant::now() + Duration::from_millis(100);
        let mut got_frame = false;
        while Instant::now() < deadline {
            let has = on_main(move |_| unsafe {
                let out = &*(output_ptr as *const Retained<AVPlayerItemVideoOutput>);
                out.hasNewPixelBufferForItemTime(time)
            });
            if has {
                got_frame = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if !got_frame {
            return Ok(None);
        }
        let buffer = on_main(move |_| unsafe {
            let out = &*(output_ptr as *const Retained<AVPlayerItemVideoOutput>);
            out.copyPixelBufferForItemTime_itemTimeForDisplay(time, ptr::null_mut())
        });
        let Some(buffer) = buffer else {
            return Ok(None);
        };
        let lock_flags = CVPixelBufferLockFlags::ReadOnly;
        if unsafe { CVPixelBufferLockBaseAddress(&buffer, lock_flags) } != 0 {
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
                let src = unsafe { std::slice::from_raw_parts(base.add(y * stride), row) };
                bgra_to_rgba(src, &mut rgba[y * row..][..row]);
            }
            let pts = self.first_pts.unwrap_or(Duration::ZERO);
            self.first_pts = Some(pts + Duration::from_millis(33));
            Some(DecodedFrame { pts, width, height })
        } else {
            None
        };
        unsafe { CVPixelBufferUnlockBaseAddress(&buffer, lock_flags) };
        Ok(result)
    }
}
