//! Conservative adaptive-bitrate selection as pure data.
//!
//! Continuity is preferred over the highest resolution: a better
//! representation is adopted only after several good measurements, while a
//! bandwidth drop switches down immediately. Nothing here downloads or
//! decodes; the application applies the chosen index.

/// One selectable representation from a manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Representation {
    /// Manifest identifier.
    pub id: u64,
    /// Declared bandwidth in bits per second.
    pub bandwidth: u64,
    /// Encoded width in pixels.
    pub width: u32,
    /// Encoded height in pixels.
    pub height: u32,
}

/// How a quality selection is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbrMode {
    /// Lowest bandwidth.
    Lowest,
    /// Middle representation by bandwidth.
    Medium,
    /// Highest bandwidth available.
    Highest,
    /// Conservative adaptive controller.
    Auto,
}

/// Configuration for the adaptive controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbrConfig {
    /// `Representation` bandwidth is declared in bits per second.
    /// Good samples required before switching up.
    pub upgrade_after: u32,
    /// Fraction of estimated throughput reserved for safety.
    pub safety: u32,
    /// Minimum buffer occupancy required to consider an upgrade.
    pub upgrade_buffer_millis: u64,
}

impl Default for AbrConfig {
    fn default() -> Self {
        Self {
            upgrade_after: 3,
            safety: 80,
            upgrade_buffer_millis: 2_000,
        }
    }
}

/// State of the conservative selector.
#[derive(Debug, Clone)]
pub struct AbrController {
    config: AbrConfig,
    mode: AbrMode,
    good_samples: u32,
    current: usize,
}

impl AbrController {
    /// A controller starting at the lowest representation in `Auto` mode.
    pub fn new(config: AbrConfig) -> Self {
        Self {
            config,
            mode: AbrMode::Auto,
            good_samples: 0,
            current: 0,
        }
    }

    /// The current selection mode.
    pub fn mode(&self) -> AbrMode {
        self.mode
    }

    /// The currently selected representation index.
    pub fn current(&self) -> usize {
        self.current
    }

    /// Sets a fixed mode and selects its representation immediately.
    pub fn set_mode(&mut self, mode: AbrMode, representations: &[Representation]) -> bool {
        if representations.is_empty() {
            return false;
        }
        self.mode = mode;
        self.good_samples = 0;
        self.current = fixed_index(mode, representations);
        true
    }

    /// Feeds one throughput sample (bits per second) and buffer occupancy.
    ///
    /// Returns the selected index. Downgrades react at once; upgrades require
    /// `upgrade_after` consecutive samples with enough buffered media.
    pub fn sample(&mut self, throughput_bps: u64, buffer_millis: u64, reps: &[Representation]) -> usize {
        if reps.is_empty() {
            return 0;
        }
        if self.mode != AbrMode::Auto {
            self.current = fixed_index(self.mode, reps);
            return self.current;
        }
        let safe_bps = throughput_bps.saturating_mul(self.config.safety as u64) / 100;
        let mut wanted = 0;
        for (index, rep) in reps.iter().enumerate() {
            if rep.bandwidth <= safe_bps {
                wanted = index;
            }
        }
        if wanted < self.current {
            self.current = wanted;
            self.good_samples = 0;
        } else if wanted > self.current {
            if buffer_millis >= self.config.upgrade_buffer_millis {
                self.good_samples += 1;
                if self.good_samples >= self.config.upgrade_after {
                    self.current = wanted;
                    self.good_samples = 0;
                }
            } else {
                self.good_samples = 0;
            }
        } else {
            self.good_samples = 0;
        }
        self.current
    }
}

fn fixed_index(mode: AbrMode, reps: &[Representation]) -> usize {
    match mode {
        AbrMode::Lowest => 0,
        AbrMode::Highest => reps.len() - 1,
        AbrMode::Medium => (reps.len() - 1) / 2,
        AbrMode::Auto => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reps() -> Vec<Representation> {
        vec![
            Representation { id: 0, bandwidth: 500_000, width: 640, height: 360 },
            Representation { id: 1, bandwidth: 1_500_000, width: 1280, height: 720 },
            Representation { id: 2, bandwidth: 4_000_000, width: 1920, height: 1080 },
        ]
    }

    #[test]
    fn a_bandwidth_drop_downgrades_immediately() {
        let mut controller = AbrController::new(AbrConfig::default());
        let reps = reps();
        // An upgrade is conservative: three good samples are required.
        for _ in 0..3 {
            assert_eq!(controller.sample(2_000_000, 5_000, &reps), 0);
        }
        assert_eq!(controller.sample(2_000_000, 5_000, &reps), 1);
        // A bandwidth drop still reacts at once.
        assert_eq!(controller.sample(300_000, 5_000, &reps), 0);
    }

    #[test]
    fn an_upgrade_needs_several_good_samples_and_buffer() {
        let mut controller = AbrController::new(AbrConfig::default());
        let reps = reps();
        assert_eq!(controller.sample(3_000_000, 100, &reps), 0);
        for _ in 0..2 {
            assert_eq!(controller.sample(3_000_000, 3_000, &reps), 0);
        }
        assert_eq!(controller.sample(3_000_000, 3_000, &reps), 1);
    }

    #[test]
    fn fixed_modes_ignore_throughput() {
        let mut controller = AbrController::new(AbrConfig::default());
        let reps = reps();
        assert!(controller.set_mode(AbrMode::Highest, &reps));
        assert_eq!(controller.sample(1, 0, &reps), 2);
        assert!(controller.set_mode(AbrMode::Medium, &reps));
        assert_eq!(controller.current(), 1);
    }
}
