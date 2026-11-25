//! A decision layer that connects manifests, buffering limits and ABR.
//!
//! This module chooses *what* to fetch and how much media to keep. It does
//! not fetch bytes or drive a decoder: the HLS/DASH backends and the
//! application own that work.

use std::time::Duration;

use crate::{
    AbrConfig, AbrController, AbrMode, BufferSnapshot, DashRepresentation, LoadControl,
    Representation,
};

/// A representation the streaming layer can actually select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamChoice {
    /// Index into the manifest's representation list.
    pub index: usize,
    /// Declared bandwidth in bits per second.
    pub bandwidth: u64,
    /// Encoded width and height.
    pub size: (u32, u32),
}

/// Combines a buffer policy with a conservative ABR selector.
pub struct StreamPolicy {
    load: LoadControl,
    abr: AbrController,
    manual: Option<usize>,
}

impl StreamPolicy {
    /// A policy with default buffering and ABR settings.
    pub fn new(load: LoadControl, abr_config: AbrConfig) -> Self {
        Self {
            load,
            abr: AbrController::new(abr_config),
            manual: None,
        }
    }

    /// The buffering policy.
    pub fn load_control(&self) -> LoadControl {
        self.load
    }

    /// The ABR selector.
    pub fn abr(&self) -> &AbrController {
        &self.abr
    }

    /// Pins quality to a fixed mode.
    pub fn set_mode(&mut self, mode: AbrMode, reps: &[Representation]) -> bool {
        self.manual = None;
        self.abr.set_mode(mode, reps)
    }

    /// Pins quality to one representation index.
    pub fn set_manual_index(&mut self, index: usize, reps: &[Representation]) -> bool {
        if index >= reps.len() {
            return false;
        }
        self.manual = Some(index);
        true
    }

    /// Returns false while the buffer says playback must wait.
    pub fn can_start(&self, buffer: &BufferSnapshot) -> bool {
        self.load.should_start(buffer.buffered)
    }

    /// Whether the backend should keep reading ahead.
    pub fn should_continue(&self, buffer: &BufferSnapshot) -> bool {
        self.load.should_continue_loading(buffer.buffered)
    }

    /// Selects a representation from throughput and buffer measurements.
    pub fn select(
        &mut self,
        throughput_bps: u64,
        buffer: &BufferSnapshot,
        reps: &[Representation],
    ) -> Option<StreamChoice> {
        if reps.is_empty() {
            return None;
        }
        let index = match self.manual {
            Some(index) => index.min(reps.len() - 1),
            None => self
                .abr
                .sample(throughput_bps, buffer.buffered.as_millis() as u64, reps),
        };
        let rep = reps[index];
        Some(StreamChoice {
            index,
            bandwidth: rep.bandwidth,
            size: (rep.width, rep.height),
        })
    }

    /// Converts parsed DASH representations into the selector's type.
    pub fn dash_representations(reps: &[DashRepresentation]) -> Vec<Representation> {
        reps.iter()
            .map(|rep| Representation {
                id: rep.id.parse().unwrap_or(0),
                bandwidth: rep.bandwidth,
                width: rep.width,
                height: rep.height,
            })
            .collect()
    }
}

/// The minimal buffer state a streaming backend reports to the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferSnapshot {
    /// Media currently buffered ahead of the clock.
    pub buffered: Duration,
    /// How long the backend has been waiting for media.
    pub waiting: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reps() -> Vec<Representation> {
        vec![
            Representation { id: 0, bandwidth: 500_000, width: 640, height: 360 },
            Representation { id: 1, bandwidth: 2_000_000, width: 1280, height: 720 },
        ]
    }

    #[test]
    fn buffer_gates_playback() {
        let policy = StreamPolicy::new(LoadControl::default(), AbrConfig::default());
        assert!(!policy.can_start(&BufferSnapshot {
            buffered: Duration::from_millis(100),
            waiting: Duration::ZERO,
        }));
        assert!(policy.can_start(&BufferSnapshot {
            buffered: Duration::from_secs(1),
            waiting: Duration::ZERO,
        }));
    }

    #[test]
    fn selection_drops_immediately_and_upgrades_slowly() {
        let mut policy = StreamPolicy::new(LoadControl::default(), AbrConfig::default());
        let reps = reps();
        let good = BufferSnapshot {
            buffered: Duration::from_secs(3),
            waiting: Duration::ZERO,
        };
        assert_eq!(policy.select(3_000_000, &good, &reps).unwrap().index, 1);
        assert_eq!(policy.select(100_000, &good, &reps).unwrap().index, 0);
        for _ in 0..2 {
            assert_eq!(policy.select(3_000_000, &good, &reps).unwrap().index, 0);
        }
        assert_eq!(policy.select(3_000_000, &good, &reps).unwrap().index, 1);
    }

    #[test]
    fn manual_selection_is_respected() {
        let mut policy = StreamPolicy::new(LoadControl::default(), AbrConfig::default());
        let reps = reps();
        assert!(policy.set_manual_index(1, &reps));
        let buffer = BufferSnapshot {
            buffered: Duration::ZERO,
            waiting: Duration::ZERO,
        };
        assert_eq!(policy.select(1, &buffer, &reps).unwrap().size, (1280, 720));
        assert!(!policy.set_manual_index(9, &reps));
    }

    #[test]
    fn dash_representations_convert() {
        let dash = vec![DashRepresentation {
            id: "3".into(),
            bandwidth: 900_000,
            width: 800,
            height: 450,
            mime_type: "video/mp4".into(),
        }];
        let converted = StreamPolicy::dash_representations(&dash);
        assert_eq!(converted[0].id, 3);
        assert_eq!(converted[0].bandwidth, 900_000);
    }
}
