//! A runtime capability probe on top of the static tables.
//!
//! `VideoCapabilities::detect` answers from compile-time tables: which codecs
//! the platform's media stack is *supposed* to support. A real device can
//! disagree — a Windows machine without the AV1 extension, an Android phone
//! whose SoC lacks a HEVC encoder-side licence, a browser without AV1. This
//! module asks the actual decoder at open time and records what it accepted,
//! so an application can adapt instead of trusting the table.

use std::sync::Mutex;
use std::sync::OnceLock;

use crate::capabilities::{VideoCapabilities, VideoCodec};

/// What a runtime probe learned about one codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeResult {
    /// The codec that was probed.
    pub codec: VideoCodec,
    /// Whether the platform's decoder accepted it.
    pub accepted: bool,
}

/// A probe that remembers what it learned for the process lifetime.
#[derive(Debug, Default)]
pub struct RuntimeProbe {
    results: Mutex<Vec<ProbeResult>>,
}

impl RuntimeProbe {
    /// An empty probe.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one probe outcome.
    pub fn record(&self, codec: VideoCodec, accepted: bool) {
        let mut results = self.results.lock().unwrap_or_else(|poisoned| {
            poisoned.into_inner()
        });
        if let Some(existing) = results.iter_mut().find(|r| r.codec == codec) {
            existing.accepted = accepted;
        } else {
            results.push(ProbeResult { codec, accepted });
        }
    }

    /// The recorded outcome for `codec`, when it was probed.
    pub fn outcome(&self, codec: VideoCodec) -> Option<bool> {
        let results = self.results.lock().unwrap_or_else(|poisoned| {
            poisoned.into_inner()
        });
        results.iter().find(|r| r.codec == codec).map(|r| r.accepted)
    }

    /// The codecs the runtime accepted, intersected with the static table.
    ///
    /// A codec the table claims but the device refused is dropped; a codec
    /// the table does not list is never added, because the crate has no
    /// decoder path for it.
    pub fn effective_codecs(&self, capabilities: &VideoCapabilities) -> Vec<VideoCodec> {
        capabilities
            .supported_codecs()
            .iter()
            .copied()
            .filter(|codec| self.outcome(*codec) != Some(false))
            .collect()
    }

    /// Every recorded outcome.
    pub fn all(&self) -> Vec<ProbeResult> {
        self.results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// The process-wide probe.
pub fn shared() -> &'static RuntimeProbe {
    static PROBE: OnceLock<RuntimeProbe> = OnceLock::new();
    PROBE.get_or_init(RuntimeProbe::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_twice_replaces_the_outcome() {
        let probe = RuntimeProbe::new();
        probe.record(VideoCodec::H264, true);
        probe.record(VideoCodec::H264, false);
        assert_eq!(probe.outcome(VideoCodec::H264), Some(false));
        assert_eq!(probe.all().len(), 1);
    }

    #[test]
    fn an_unprobed_codec_falls_through_to_the_table() {
        let probe = RuntimeProbe::new();
        let capabilities = VideoCapabilities::detect();
        let before = capabilities.supported_codecs().to_vec();
        let effective = probe.effective_codecs(&capabilities);
        assert_eq!(effective, before, "no probe recorded, table stands");
    }

    #[test]
    fn a_refused_codec_is_dropped_from_the_effective_list() {
        let probe = RuntimeProbe::new();
        let capabilities = VideoCapabilities::detect();
        let table = capabilities.supported_codecs();
        if let Some(codec) = table.first().copied() {
            probe.record(codec, false);
            let effective = probe.effective_codecs(&capabilities);
            assert!(!effective.contains(&codec));
        }
    }

    #[test]
    fn the_shared_probe_is_stable() {
        let a = shared();
        let b = shared();
        assert!(std::ptr::eq(a, b));
    }
}
