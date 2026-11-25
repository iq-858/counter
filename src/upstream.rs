//! A thin bridge to Cranpose 0.28's hardware decoder and event-driven frames.
//!
//! This module is active only with the `upstream-video` feature. It does NOT
//! replace the crate's CPU `VideoPlayer`: it gives applications that already
//! demux access units a way to drive Cranpose's own zero-copy decoder and feed
//! its `DecodedFrames` to `VideoHardwareSurface` (`cranpose::PlayerSurface`).
//! No frame is copied through this crate on that path.

use std::time::Duration;

use cranpose_services::{
    AccessUnit, DecodedFrames, DecoderConfig, DecoderError, DecoderOutput, VideoCodec, VideoDecoder,
    VideoFrame as UpstreamVideoFrame,
};

/// A hardware decoder together with the handle that receives its pictures.
pub struct HardwareVideo {
    decoder: VideoDecoder,
    frames: DecodedFrames,
}

impl HardwareVideo {
    /// Opens the hardware decoder for `config`.
    pub fn new(config: DecoderConfig) -> Result<Self, DecoderError> {
        let decoder = VideoDecoder::new(config)?;
        let frames = decoder.frames();
        Ok(Self { decoder, frames })
    }

    /// Opens the official fast decoder for an elementary Annex-B stream.
    ///
    /// `split_access_units` cuts H.264/HEVC into pictures; each is submitted
    /// with a rising timestamp and a frame index as metadata. This is the real
    /// hardware-decode path: no CPU RGBA conversion happens in this crate.
    /// Demuxing a container into an elementary stream is the caller's job.
    pub fn open_stream(
        codec: VideoCodec,
        width: u32,
        height: u32,
        stream: &[u8],
    ) -> Result<Self, DecoderError> {
        let config = DecoderConfig::new(codec, width, height, DecoderOutput::Frames { depth: 6 });
        let mut decoder = VideoDecoder::new(config)?;
        let units = cranpose_services::split_access_units(codec, stream);
        let mut timestamp = Duration::ZERO;
        for (index, data) in units.iter().enumerate() {
            decoder.submit(AccessUnit {
                data,
                timestamp,
                metadata: index as u64,
            })?;
            timestamp += Duration::from_millis(40);
        }
        let frames = decoder.frames();
        Ok(Self { decoder, frames })
    }

    /// What the hardware decoder can do for `codec` on this device.
    pub fn capabilities(codec: VideoCodec) -> Result<cranpose_services::DecoderCapabilities, DecoderError> {
        VideoDecoder::capabilities(codec)
    }

    /// Feeds one access unit. Call from the feeding thread, never a frame loop.
    pub fn submit(&mut self, unit: AccessUnit<'_>) -> Result<(), DecoderError> {
        self.decoder.submit(unit)
    }

    /// The handle passed to `VideoHardwareSurface`.
    pub fn frames(&self) -> DecodedFrames {
        self.frames.clone()
    }

    /// Waits for the decoder's own wake-up, with no per-frame polling.
    pub async fn wait_for_frame(&self) {
        self.frames.wait_for_frame().await;
    }

    /// Takes the oldest picture, when the decoder outputs frames.
    pub fn next_frame(&self) -> Option<UpstreamVideoFrame> {
        self.frames.next_frame()
    }

    /// Decoder statistics.
    pub fn stats(&self) -> cranpose_services::DecoderStats {
        self.frames.stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_query_is_callable_on_every_target() {
        // Unsupported targets return an error; they must not panic.
        let _ = HardwareVideo::capabilities(VideoCodec::H264);
    }
}
