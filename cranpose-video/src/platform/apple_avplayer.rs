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
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use dispatch2::DispatchQueue;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, rc::Retained, runtime::AnyObject};
use objc2_av_foundation::{AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVPlayerItemVideoOutput};
use objc2_core_media::CMTime;
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

/// A decoder that plays an HLS stream through AVPlayer and reads BGRA.
///
/// Apple documents HLS as an HTTP-delivered playlist + MPEG-TS segments; a raw
/// joined `.ts` file is not a supported standalone `AVPlayerItem(url:)` source
/// (it fails with "Cannot Open"). The original `.m3u8` URL is passed straight to
/// AVPlayer, which handles the playlist, segments and adaptation itself. Audio
/// is muted here because cranpose pairs the source's sound separately on iOS.
pub(crate) struct AvPlayerDecoder {
    player: SendRetained<AVPlayer>,
    item: SendRetained<AVPlayerItem>,
    output: SendRetained<AVPlayerItemVideoOutput>,
    info: VideoInfo,
    /// Set by the worker's observe_shutdown so a buffering read can exit.
    shutdown: Arc<AtomicBool>,
}

impl AvPlayerDecoder {
    /// Opens a local file URL. Kept for the older per-window HLS path; on iOS a
    /// raw MPEG-TS file is not a supported standalone AVPlayerItem source, so
    /// the main HLS path uses [`Self::open_url`] with the original playlist.
    pub(crate) fn open_file(path: &std::path::Path) -> Result<Self, VideoError> {
        let address = format!("file://{}", path.to_string_lossy());
        Self::open_url(&address)
    }

    /// Opens the HLS playlist URL directly. AVPlayer reads the index and its
    /// MPEG-TS segments over HTTP — the supported Apple path — instead of a
    /// joined local `.ts`, which AVFoundation cannot open as a standalone file.
    pub(crate) fn open_url(address: &str) -> Result<Self, VideoError> {
        let address = address.to_owned();
        let setup: Result<
            (
                SendRetained<AVPlayer>,
                SendRetained<AVPlayerItem>,
                SendRetained<AVPlayerItemVideoOutput>,
            ),
            String,
        > = on_main(move |mtm| {
            let text = NSString::from_str(&address);
            let url = NSURL::URLWithString(&text)
                .ok_or_else(|| format!("the HLS address is not a valid URL: {address}"))?;
            let item = unsafe { AVPlayerItem::initWithURL(AVPlayerItem::alloc(mtm), &url) };
            let player = unsafe { AVPlayer::initWithPlayerItem(AVPlayer::alloc(mtm), Some(&item)) };
            // HLS skips the generic audio wrapper (source.is_hls()), so AVPlayer
            // is the ONLY sound source here. Keep its volume: muting it produced
            // picture with no sound. `carries_audio()` returns true below so
            // nothing else pairs a second player.
            unsafe { player.setVolume(1.0) };
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
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    fn item_time(&self) -> CMTime {
        // Use the player's own clock: it has sub-second precision and stays
        // correct through rebuffering. Whole-second truncation used to query
        // the video output at the same instant for ~1 s (frames=0), and a local
        // Instant drifted whenever AVPlayer stalled to fill its buffer.
        let player_ptr = &self.player.0 as *const Retained<AVPlayer> as usize;
        on_main(move |_| unsafe {
            let player = &*(player_ptr as *const Retained<AVPlayer>);
            player.currentTime()
        })
    }
}

impl crate::VideoDecoder for AvPlayerDecoder {
    fn info(&self) -> VideoInfo {
        self.info
    }

    fn output_format(&self) -> PixelFormat {
        PixelFormat::Rgba8
    }

    fn carries_audio(&self) -> bool {
        // AVPlayer plays the HLS stream's own audio track. Declaring this keeps
        // the generic audio wrapper from pairing a second player (double sound).
        true
    }

    fn observe_shutdown(&mut self, flag: Arc<AtomicBool>) {
        self.shutdown = flag;
    }

    fn pause(&mut self) {
        let player_ptr = &self.player.0 as *const Retained<AVPlayer> as usize;
        on_main(move |_| unsafe {
            (&*(player_ptr as *const Retained<AVPlayer>)).pause();
        });
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
        let output_ptr = &self.output.0 as *const Retained<AVPlayerItemVideoOutput> as usize;
        // Block until a frame is ready or the worker signals shutdown. A live
        // HLS source buffers over HTTP; returning Ok(None) on a buffering gap
        // is read by the worker as end-of-media and triggers the recovery cycle
        // that made playback freeze and restart repeatedly. Keeping the read
        // pending through normal buffering removes that cycle.
        let mut found: Option<CMTime> = None;
        while !self.shutdown.load(Ordering::Acquire) {
            // Recompute the item time each poll: a single value held across a
            // long wait goes stale and keeps missing the moving play head.
            let time = self.item_time();
            let has = on_main(move |_| unsafe {
                let out = &*(output_ptr as *const Retained<AVPlayerItemVideoOutput>);
                out.hasNewPixelBufferForItemTime(time)
            });
            if has {
                found = Some(time);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let Some(time) = found else {
            return Ok(None);
        };
        // Copy inside on_main: the CVPixelBuffer is not Send, so only the
        // resulting RGBA bytes and dimensions cross back to the decode thread.
        let copied: Option<(Vec<u8>, u32, u32, f64)> = on_main(move |_| unsafe {
            let out = &*(output_ptr as *const Retained<AVPlayerItemVideoOutput>);
            let mut display_time = time;
            let Some(buffer) =
                out.copyPixelBufferForItemTime_itemTimeForDisplay(time, &mut display_time)
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
            // Use the buffer's true display deadline as the frame timestamp so
            // VFR and 25/50/60 fps streams are paced correctly instead of a
            // fabricated fixed 33 ms step.
            let seconds = display_time.seconds();
            pixels
                .filter(|_| seconds.is_finite() && seconds >= 0.0)
                .map(|pixels| (pixels, width, height, seconds))
        });
        let Some((pixels, width, height, seconds)) = copied else {
            return Ok(None);
        };
        *rgba = pixels;
        let pts = Duration::from_secs_f64(seconds);
        Ok(Some(DecodedFrame { pts, width, height }))
    }
}
