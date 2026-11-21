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
impl<T: ?Sized> std::ops::Deref for SendRetained<T> {
    type Target = Retained<T>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

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
                SendRetained<AVPlayer>,
                SendRetained<AVPlayerItem>,
                SendRetained<AVPlayerItemVideoOutput>,
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
                    AVPlayerItemVideoOutput::alloc(),
                    Some(settings.as_ref()),
                )
            };
            unsafe { item.addOutput(&output) };
            unsafe { player.play() };
            Ok((
                SendRetained(player),
                SendRetained(item),
                SendRetained(output),
            ))
        });
        let (player, item, output) = setup.map_err(VideoError::Decode)?;

        // Wait for the item to become ready on the main thread. The closures
        // borrow the item through a raw pointer so the SendRetained wrapper is
        // not consumed; every call still runs on the main queue.
        let item_ptr = &item as *const SendRetained<AVPlayerItem> as usize;
        let began = Instant::now();
        while began.elapsed() < READY_TIMEOUT {
            let status = on_main(move |_| unsafe {
                (&*(item_ptr as *const SendRetained<AVPlayerItem>)).status()
            });
            if status == AVPlayerItemStatus::ReadyToPlay {
                break;
            }
            if status == AVPlayerItemStatus::Failed {
                let msg = on_main(move |_| unsafe {
                    (&*(item_ptr as *const SendRetained<AVPlayerItem>))
                        .error()
                        .map(|e| e.localizedDescription().to_string())
                        .unwrap_or_else(|| "AVPlayer could not open the media".to_owned())
                });
                return Err(VideoError::Decode(msg));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let size = on_main(move |_| unsafe {
            (&*(item_ptr as *const SendRetained<AVPlayerItem>))
                .asset()
                .naturalSize()
        });
        let info = VideoInfo {
            width: size.width.round().max(0.0) as u32,
            height: size.height.round().max(0.0) as u32,
            duration: None,
            frame_rate: None,
            rotation: Default::default(),
        };
        Ok(Self {
            player,
            item,
            output,
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
        // Copy inside on_main: the CVPixelBuffer is not Send, so only the
        // resulting RGBA bytes and dimensions cross back to the decode thread.
        let copied: Option<(Vec<u8>, u32, u32)> = on_main(move |_| unsafe {
            let out = &*(output_ptr as *const Retained<AVPlayerItemVideoOutput>);
            let Some(buffer) =
                out.copyPixelBufferForItemTime_itemTimeForDisplay(time, ptr::null_mut())
            else {
                return None;
            };
            let lock_flags = CVPixelBufferLockFlags::ReadOnly;
            if CVPixelBufferLockBaseAddress(&buffer, lock_flags) != 0 {
                return None;
            }
            let width = CVPixelBufferGetWidth(&buffer) as u32;
            let height = CVPixelBufferGetHeight(&buffer) as u32;
            let stride = CVPixelBufferGetBytesPerRow(&buffer);
            let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
            let row = width as usize * 4;
            let pixels = if !base.is_null() && stride >= row && width > 0 && height > 0 {
                let mut pixels = vec![0u8; row * height as usize];
                for y in 0..height as usize {
                    let src = std::slice::from_raw_parts(base.add(y * stride), row);
                    bgra_to_rgba(src, &mut pixels[y * row..][..row]);
                }
                Some(pixels)
            } else {
                None
            };
            CVPixelBufferUnlockBaseAddress(&buffer, lock_flags);
            pixels.map(|pixels| (pixels, width, height))
        });
        let Some((pixels, width, height)) = copied else {
            return Ok(None);
        };
        *rgba = pixels;
        let pts = self.first_pts.unwrap_or(Duration::ZERO);
        self.first_pts = Some(pts + Duration::from_millis(33));
        Ok(Some(DecodedFrame { pts, width, height }))
    }
}
