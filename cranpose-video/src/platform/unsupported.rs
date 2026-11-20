//! The build that carries no decoder: Linux without the GStreamer feature,
//! Emscripten, and any target this crate knows nothing about.

use std::boxed::Box;

use super::current;
use crate::{VideoDecoder, VideoError, VideoSource};

pub(crate) const NAME: &str = "none";
pub(crate) const SUPPORTED: bool = false;

pub(crate) fn open(_source: &VideoSource) -> Result<Box<dyn VideoDecoder>, VideoError> {
    Err(VideoError::Unsupported {
        platform: current().name(),
    })
}
