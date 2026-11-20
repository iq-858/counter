//! Video decoding through the platform's native media stack.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread::{self, JoinHandle},
};

pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub struct Decoder {
    receiver: Option<Receiver<Result<VideoFrame, String>>>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Decoder {
    pub fn open(name: &'static str) -> anyhow::Result<Self> {
        let path =
            resolve(name).ok_or_else(|| anyhow::anyhow!("video file for {name} was not found"))?;
        let (sender, receiver) = sync_channel(1);
        let paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_paused = paused.clone();
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name(format!("video-{name}"))
            .spawn(move || {
                if let Err(error) =
                    decode_loop(path, sender.clone(), worker_paused, worker_stop.clone())
                    && !worker_stop.load(Ordering::Relaxed)
                {
                    let _ = sender.send(Err(error.to_string()));
                }
            })?;

        Ok(Self {
            receiver: Some(receiver),
            paused,
            stop,
            worker: Some(worker),
        })
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn try_frame(&self) -> Option<Result<VideoFrame, String>> {
        match self.receiver.as_ref()?.try_recv() {
            Ok(frame) => Some(frame),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.receiver.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
fn decode_loop(
    path: PathBuf,
    sender: SyncSender<Result<VideoFrame, String>>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Media::MediaFoundation::{
                MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
                MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
                MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION, MFCreateAttributes, MFCreateMediaType,
                MFCreateSourceReaderFromURL, MFMediaType_Video, MFSTARTUP_FULL, MFShutdown,
                MFStartup, MFVideoFormat_RGB32,
            },
            System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        },
        core::PCWSTR,
    };

    struct Runtime;
    impl Drop for Runtime {
        fn drop(&mut self) {
            unsafe {
                let _ = MFShutdown();
                CoUninitialize();
            }
        }
    }

    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|error| anyhow::anyhow!("COM initialization failed: {error}"))?;
        if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
            CoUninitialize();
            return Err(anyhow::anyhow!("Media Foundation startup failed: {error}"));
        }
    }
    let _runtime = Runtime;

    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let mut played_frames = 0_u64;

    while !stop.load(Ordering::Relaxed) {
        let mut attributes = None;
        unsafe { MFCreateAttributes(&mut attributes, 1) }?;
        let attributes = attributes
            .ok_or_else(|| anyhow::anyhow!("Media Foundation returned no reader attributes"))?;
        unsafe {
            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
        }
        let reader =
            unsafe { MFCreateSourceReaderFromURL(PCWSTR(wide_path.as_ptr()), &attributes) }
                .map_err(|error| anyhow::anyhow!("opening {} failed: {error}", path.display()))?;
        let media_type = unsafe { MFCreateMediaType() }
            .map_err(|error| anyhow::anyhow!("creating video format failed: {error}"))?;
        unsafe {
            media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            reader
                .SetCurrentMediaType(stream, None, &media_type)
                .map_err(|error| anyhow::anyhow!("selecting RGB video output failed: {error}"))?;
        }
        let output_type = unsafe { reader.GetCurrentMediaType(stream) }
            .map_err(|error| anyhow::anyhow!("reading the output video format failed: {error}"))?;
        let frame_size = unsafe { output_type.GetUINT64(&MF_MT_FRAME_SIZE) }
            .map_err(|error| anyhow::anyhow!("reading the video frame size failed: {error}"))?;
        let width = (frame_size >> 32) as u32;
        let height = frame_size as u32;
        if width == 0 || height == 0 {
            anyhow::bail!(
                "Media Foundation returned an invalid video size for {}",
                path.display()
            );
        }

        let start = std::time::Instant::now();
        let mut paused_total = std::time::Duration::ZERO;
        let mut first_timestamp = None;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }

            let mut flags = 0_u32;
            let mut timestamp = 0_i64;
            let mut sample = None;
            unsafe {
                reader.ReadSample(
                    stream,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )?;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(sample) = sample else {
                continue;
            };

            let buffer = unsafe { sample.ConvertToContiguousBuffer() }?;
            let mut pixels = std::ptr::null_mut();
            let mut length = 0_u32;
            unsafe {
                buffer.Lock(&mut pixels, None, Some(&mut length))?;
            }
            let expected = width as usize * height as usize * 4;
            if (length as usize) < expected {
                unsafe { buffer.Unlock()? };
                anyhow::bail!(
                    "decoded video frame was incomplete: expected {expected} bytes, got {length}"
                );
            }
            let mut rgba = unsafe { std::slice::from_raw_parts(pixels, expected).to_vec() };
            unsafe { buffer.Unlock()? };
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }

            let timestamp = timestamp.max(0) as u64;
            let origin = *first_timestamp.get_or_insert(timestamp);
            let frame_time =
                std::time::Duration::from_nanos(timestamp.saturating_sub(origin) * 100);
            loop {
                if stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                if paused.load(Ordering::Relaxed) {
                    let pause_start = std::time::Instant::now();
                    while paused.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                    paused_total += pause_start.elapsed();
                    continue;
                }

                let target = frame_time.saturating_sub(paused_total);
                let remaining = target.saturating_sub(start.elapsed());
                if remaining.is_zero() {
                    break;
                }
                thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
            }

            if sender
                .send(Ok(VideoFrame {
                    width,
                    height,
                    pixels: rgba,
                }))
                .is_err()
            {
                return Ok(());
            }
            played_frames += 1;
        }
    }

    if played_frames == 0 && !stop.load(Ordering::Relaxed) {
        anyhow::bail!("no video frames could be decoded from {}", path.display());
    }
    Ok(())
}

#[cfg(not(windows))]
fn decode_loop(
    _path: PathBuf,
    _sender: SyncSender<Result<VideoFrame, String>>,
    _paused: Arc<AtomicBool>,
    _stop: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    anyhow::bail!("in-app video playback currently requires Windows Media Foundation")
}

/// Resolve video file path
pub fn resolve(name: &'static str) -> Option<PathBuf> {
    let rel = format!("assets/videos/{name}.mp4");
    let mut candidates = vec![PathBuf::from(&rel)];
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(&rel));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(&rel));
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join(&rel));
            }
        }
    }
    if let Some(dir) = option_env!("CARGO_MANIFEST_DIR") {
        candidates.push(PathBuf::from(dir).join(&rel));
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Get media URI for the video
pub fn media_uri(name: &'static str) -> Option<String> {
    resolve(name).map(|path| {
        let abs = path.canonicalize().unwrap_or(path);
        cranpose_services::media::uri_for_path(&abs)
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::Decoder;
    use std::time::{Duration, Instant};

    #[test]
    fn bundled_videos_decode_to_frames() {
        for name in ["bbb10", "flower", "sintel", "bunny", "w3c"] {
            let decoder = Decoder::open(name).expect("video decoder should start");
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut decoded = 0;
            while decoded < 3 {
                let frame = loop {
                    if let Some(frame) = decoder.try_frame() {
                        break frame
                            .unwrap_or_else(|error| panic!("{name} failed to decode: {error}"));
                    }
                    assert!(Instant::now() < deadline, "timed out decoding {name}");
                    std::thread::sleep(Duration::from_millis(10));
                };
                assert!(frame.width > 0, "{name} has no video width");
                assert!(frame.height > 0, "{name} has no video height");
                assert_eq!(
                    frame.pixels.len(),
                    frame.width as usize * frame.height as usize * 4,
                    "{name} returned an incomplete frame"
                );
                decoded += 1;
            }
        }
    }
}
