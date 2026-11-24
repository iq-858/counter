use std::time::Duration;

/// A frame rate as the container states it, kept as a ratio so 29.97 stays
/// exact instead of turning into a repeating decimal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRate {
    /// Frames per second's numerator, such as 30000.
    pub numerator: u32,
    /// Frames per second's denominator, such as 1001.
    pub denominator: u32,
}

impl FrameRate {
    /// The rate as a decimal number.
    pub fn frames_per_second(&self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator.max(1))
    }

    /// The time one frame lasts, or `None` when the rate is not stated.
    pub fn frame_duration(&self) -> Option<Duration> {
        let fps = self.frames_per_second();
        (fps > 0.0).then(|| Duration::from_secs_f64(1.0 / fps))
    }

    // A platform that states a rate as a floating point number keeps it at a
    // thousandth of a frame, which reads 29.97 as 29970/1000. Media Foundation
    // states a ratio and the browser states no rate, so only these platforms
    // ask for this.
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "android",
        all(target_os = "linux", feature = "linux-gstreamer")
    ))]
    pub(crate) fn approx(frames_per_second: f32) -> Option<Self> {
        let rate = f64::from(frames_per_second);
        (rate.is_finite() && rate > 0.0).then(|| Self {
            numerator: (rate * 1000.0).round() as u32,
            denominator: 1000,
        })
    }
}

impl Default for FrameRate {
    fn default() -> Self {
        Self {
            numerator: 0,
            denominator: 1,
        }
    }
}

/// How far a picture must turn clockwise before it is shown upright.
///
/// A phone records landscape frames and states the turn in the track, so a
/// player that ignores this shows a portrait clip on its side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Rotation {
    /// The frames are already upright.
    #[default]
    None,
    /// The frames are turned 90 degrees clockwise.
    Clockwise90,
    /// The frames are upside down.
    Clockwise180,
    /// The frames are turned 270 degrees clockwise.
    Clockwise270,
}

impl Rotation {
    /// The rotation a track states, from the degrees it reports. Values that
    /// are not a quarter turn read as [`Rotation::None`].
    pub fn from_degrees(degrees: f64) -> Self {
        let wrapped = degrees.rem_euclid(360.0);
        if (wrapped - 90.0).abs() < 1.0 {
            Self::Clockwise90
        } else if (wrapped - 180.0).abs() < 1.0 {
            Self::Clockwise180
        } else if (wrapped - 270.0).abs() < 1.0 {
            Self::Clockwise270
        } else {
            Self::None
        }
    }

    /// Whether the picture is shown on its side, so width and height swap.
    pub fn swaps_axes(&self) -> bool {
        matches!(self, Self::Clockwise90 | Self::Clockwise270)
    }

    /// The width and height as the viewer sees them, after the turn.
    pub fn display_size(&self, width: u32, height: u32) -> (u32, u32) {
        if self.swaps_axes() {
            (height, width)
        } else {
            (width, height)
        }
    }
}

/// What a decoder learns when it opens a source.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VideoInfo {
    /// The width of a decoded frame in pixels, before any rotation.
    pub width: u32,
    /// The height of a decoded frame in pixels, before any rotation.
    pub height: u32,
    /// How long the media runs, when the container states it.
    pub duration: Option<Duration>,
    /// The rate the frames were recorded at, when the container states it.
    pub frame_rate: Option<FrameRate>,
    /// The turn the track asks for before the picture is shown.
    pub rotation: Rotation,
}

impl VideoInfo {
    /// The width and height a viewer sees, after the track's rotation.
    pub fn display_size(&self) -> (u32, u32) {
        self.rotation.display_size(self.width, self.height)
    }

    /// The size of one RGBA8 frame in bytes.
    pub fn frame_bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}
