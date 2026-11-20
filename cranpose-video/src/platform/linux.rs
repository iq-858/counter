//! GStreamer: the media framework a Linux desktop installs.
//!
//! `uridecodebin` picks a demuxer and decoder from the plugins the system
//! carries, `videoconvert` turns each picture into NV12 (the planar layout the
//! decoder and the GPU both prefer), `videoflip` turns it upright from the
//! rotation the container states, and an `appsink` hands the pictures here. The
//! sink keeps two pictures queued, so the pipeline decodes no further ahead
//! than the player is behind.

use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use gstreamer_video::{VideoFormat, VideoInfo as Caps, VideoOrientation};

use super::super::temp_file::TempFile;
use crate::{
    DecodedFrame, FrameRate, PixelFormat, Rotation, VideoDecoder, VideoError, VideoInfo,
    VideoSource, YuvColor,
};

pub(crate) const NAME: &str = "GStreamer";
pub(crate) const SUPPORTED: bool = true;

/// How long each wait for the next picture lasts before the bus is read, so a
/// pipeline that stopped on an error reports it instead of waiting forever.
const SLICE: gst::ClockTime = gst::ClockTime::from_mseconds(100);

/// How long opening may take before the caller is told the pipeline never
/// became ready, which a network source that never answers causes.
const READY: gst::ClockTime = gst::ClockTime::from_seconds(10);

pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn VideoDecoder>, VideoError> {
    gst::init().map_err(|error| VideoError::Source(format!("GStreamer did not start: {error}")))?;
    let (uri, spill) = match source {
        VideoSource::Bytes(bytes) => {
            let file = TempFile::write(bytes)?;
            (file_uri(file.path()), Some(file))
        }
        VideoSource::Path(path) => (file_uri(path), None),
        VideoSource::Url(url) => (url.to_string(), None),
    };

    let pipeline = gst::Pipeline::new();
    let source = gst::ElementFactory::make("uridecodebin")
        .property("uri", &uri)
        .build()
        .map_err(|error| {
            VideoError::Source(format!(
                "the system has no uridecodebin, so it decodes no video: {error}"
            ))
        })?;
    let convert = gst::ElementFactory::make("videoconvert")
        .build()
        .map_err(|error| {
            VideoError::Source(format!(
                "the system has no videoconvert, so it decodes no video: {error}"
            ))
        })?;
    let flip = gst::ElementFactory::make("videoflip")
        .property("video-direction", VideoOrientation::Auto)
        .build()
        .map_err(|error| {
            VideoError::Source(format!(
                "the system has no videoflip, so it decodes no video: {error}"
            ))
        })?;
    let caps = gstreamer_video::VideoCapsBuilder::new()
        .format(VideoFormat::Nv12)
        .build();
    let sink = AppSink::builder()
        .caps(&caps)
        .sync(false)
        .max_buffers(2)
        .build();

    pipeline
        .add_many([&source, &convert, &flip, sink.upcast_ref()])
        .map_err(|error| VideoError::Source(format!("GStreamer built no pipeline: {error}")))?;
    gst::Element::link_many([&convert, &flip, sink.upcast_ref()])
        .map_err(|error| VideoError::Source(format!("GStreamer linked no pipeline: {error}")))?;

    // `uridecodebin` states its pads once it has read the container, and only
    // the video pad goes into the converter.
    let entry = convert
        .static_pad("sink")
        .ok_or_else(|| VideoError::Source("GStreamer's converter has no sink pad".to_owned()))?;
    source.connect_pad_added(move |_, pad| {
        let video = pad
            .current_caps()
            .and_then(|caps| {
                caps.structure(0)
                    .map(|structure| structure.name().starts_with("video/"))
            })
            .unwrap_or(true);
        if video && !entry.is_linked() {
            let _ = pad.link(&entry);
        }
    });

    pipeline.set_state(gst::State::Playing).map_err(|error| {
        VideoError::Source(format!("GStreamer did not start the pipeline: {error}"))
    })?;
    let (ready, _, _) = pipeline.state(READY);
    ready.map_err(|error| VideoError::Source(format!("GStreamer never became ready: {error}")))?;

    let mut info = sink_info(&sink);
    info.duration = pipeline
        .query_duration::<gst::ClockTime>()
        .map(|time| Duration::from_nanos(time.nseconds()));
    Ok(Box::new(Reader {
        pipeline,
        sink,
        info,
        spill,
        finished: false,
        planes: Vec::new(),
    }))
}

fn sink_info(sink: &AppSink) -> VideoInfo {
    let caps = sink
        .static_pad("sink")
        .and_then(|pad| pad.current_caps())
        .and_then(|caps| Caps::from_caps(&caps).ok());
    match caps {
        Some(caps) => VideoInfo {
            width: caps.width(),
            height: caps.height(),
            duration: None,
            frame_rate: rate(caps.fps()),
            rotation: Rotation::None,
        },
        None => VideoInfo::default(),
    }
}

fn rate(fps: gst::Fraction) -> Option<FrameRate> {
    let numerator = u32::try_from(fps.numer()).ok()?;
    let denominator = u32::try_from(fps.denom()).ok()?;
    (numerator > 0 && denominator > 0).then_some(FrameRate {
        numerator,
        denominator,
    })
}

fn file_uri(path: &std::path::Path) -> String {
    // A URI states each byte the file name holds, so a space or a non-ASCII
    // character reaches the demuxer as it is written on disk.
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'/' | b'-' | b'_' | b'.' | b'~' => uri.push(char::from(byte)),
            _ if byte.is_ascii_alphanumeric() => uri.push(char::from(byte)),
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

struct Reader {
    pipeline: gst::Pipeline,
    sink: AppSink,
    info: VideoInfo,
    spill: Option<TempFile>,
    finished: bool,
    /// The planar buffer the sink fills, kept so a picture costs no allocation.
    planes: Vec<u8>,
}

impl Reader {
    /// The failure a pipeline reported on its bus, when it reported one.
    fn failure(&self) -> Option<VideoError> {
        let bus = self.pipeline.bus()?;
        while let Some(message) = bus.pop_filtered(&[gst::MessageType::Error]) {
            if let gst::MessageView::Error(error) = message.view() {
                return Some(VideoError::Decode(format!(
                    "GStreamer stopped: {}",
                    error.error()
                )));
            }
        }
        None
    }
}

impl VideoDecoder for Reader {
    fn info(&self) -> VideoInfo {
        self.info
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        let target =
            gst::ClockTime::from_nseconds(u64::try_from(position.as_nanos()).unwrap_or(u64::MAX));
        self.pipeline
            .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, target)
            .map_err(|error| VideoError::Source(format!("GStreamer cannot seek: {error}")))?;
        self.finished = false;
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        let mut planes = std::mem::take(&mut self.planes);
        let frame = match self.pull(&mut planes) {
            Ok(Some(frame)) => frame,
            other => {
                self.planes = planes;
                return other;
            }
        };
        let width = frame.width as usize;
        let height = frame.height as usize;
        if rgba.len() != width * height * 4 {
            rgba.resize(width * height * 4, 0);
        }
        crate::pixels::nv12_to_rgba(&planes, width, height, self.color(), rgba);
        self.planes = planes;
        Ok(Some(frame))
    }

    fn read_nv12(&mut self, nv12: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        // The sink fills the caller's buffer directly, so a planar picture is
        // copied once and never held here.
        self.pull(nv12)
    }

    fn output_format(&self) -> PixelFormat {
        PixelFormat::Nv12
    }

    fn nv12_color(&self) -> YuvColor {
        self.color()
    }

    fn close(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        self.spill = None;
    }
}

impl Reader {
    /// The colour space a picture of this size conventionally uses.
    fn color(&self) -> YuvColor {
        YuvColor::for_size(self.info.width, self.info.height)
    }

    /// The next picture's planes, unpadded, which is the layout the frame type
    /// carries.
    fn pull(&mut self, nv12: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        if self.finished {
            return Ok(None);
        }
        loop {
            let Some(sample) = self.sink.try_pull_sample(SLICE) else {
                if let Some(error) = self.failure() {
                    return Err(error);
                }
                if self.sink.is_eos() {
                    self.finished = true;
                    return Ok(None);
                }
                continue;
            };
            let Some(buffer) = sample.buffer() else {
                continue;
            };
            let caps = sample.caps().and_then(|caps| Caps::from_caps(caps).ok());
            let (width, height, luma_stride, chroma_stride, luma_offset, chroma_offset) =
                match &caps {
                    Some(caps) => {
                        let stride = caps.stride();
                        let offset = caps.offset();
                        let luma_stride = stride[0].max(0) as usize;
                        // A second plane states where it starts. A buffer whose
                        // caps leave it out carries the chroma straight after the
                        // luma plane, which is where NV12 puts it.
                        let chroma_stride = if stride[1] > 0 {
                            stride[1] as usize
                        } else {
                            luma_stride
                        };
                        let chroma_offset = if offset[1] > offset[0] {
                            offset[1]
                        } else {
                            offset[0] + luma_stride * caps.height() as usize
                        };
                        (
                            caps.width(),
                            caps.height(),
                            luma_stride,
                            chroma_stride,
                            offset[0],
                            chroma_offset,
                        )
                    }
                    None => continue,
                };
            if width == 0 || height == 0 {
                continue;
            }
            let row = width as usize;
            let height = height as usize;
            let needed = row * height * 3 / 2;
            let map = buffer.map_readable().map_err(|error| {
                VideoError::Decode(format!("GStreamer holds no pixels: {error}"))
            })?;
            let pixels = map.as_slice();
            if luma_stride < row
                || luma_offset + luma_stride * (height - 1) + row > pixels.len()
                || chroma_offset + chroma_stride * (height / 2 - 1) + row > pixels.len()
            {
                return Err(VideoError::Decode(
                    "GStreamer stated a picture larger than its buffer".to_owned(),
                ));
            }
            if nv12.len() != needed {
                nv12.resize(needed, 0);
            }
            for y in 0..height {
                let start = luma_offset + y * luma_stride;
                nv12[y * row..][..row].copy_from_slice(&pixels[start..start + row]);
            }
            let chroma_base = row * height;
            for y in 0..height / 2 {
                let start = chroma_offset + y * chroma_stride;
                nv12[chroma_base + y * row..][..row].copy_from_slice(&pixels[start..start + row]);
            }
            let pts = buffer
                .pts()
                .map(|time| Duration::from_nanos(time.nseconds()))
                .unwrap_or_default();
            if let Some(caps) = caps {
                self.info.width = width;
                self.info.height = height as u32;
                if self.info.frame_rate.is_none() {
                    self.info.frame_rate = rate(caps.fps());
                }
            }
            return Ok(Some(DecodedFrame {
                pts,
                width,
                height: height as u32,
            }));
        }
    }
}
