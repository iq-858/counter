//! A backend-independent controller trait and a surface abstraction.
//!
//! `PlaybackController` lets a feed or a UI layer drive any player without
//! knowing whether it holds a `VideoPlayer`, a `VideoPlayerHandle`, or a
//! custom implementation. `VideoSurface` is deliberately minimal: it names a
//! presentation target and its size. It does not claim GPU access.

use std::time::Duration;

use crate::{VideoError, VideoPlayer, VideoPlayerHandle, VideoState};

/// The transport operations every playback owner can perform.
pub trait PlaybackController {
    /// Start or resume playback.
    fn controller_play(&self) -> Result<(), VideoError>;
    /// Hold the picture.
    fn controller_pause(&self) -> Result<(), VideoError>;
    /// Play or pause.
    fn controller_toggle(&self) -> Result<(), VideoError>;
    /// Move the clock to an absolute position.
    fn controller_seek(&self, position: Duration) -> Result<(), VideoError>;
    /// The current state.
    fn controller_state(&self) -> VideoState;
    /// The current position.
    fn controller_position(&self) -> Duration;
}

impl PlaybackController for VideoPlayer {
    fn controller_play(&self) -> Result<(), VideoError> {
        self.play()
    }
    fn controller_pause(&self) -> Result<(), VideoError> {
        self.pause()
    }
    fn controller_toggle(&self) -> Result<(), VideoError> {
        self.toggle()
    }
    fn controller_seek(&self, position: Duration) -> Result<(), VideoError> {
        self.seek(position)
    }
    fn controller_state(&self) -> VideoState {
        self.state()
    }
    fn controller_position(&self) -> Duration {
        self.position()
    }
}

impl PlaybackController for VideoPlayerHandle {
    fn controller_play(&self) -> Result<(), VideoError> {
        self.play()
    }
    fn controller_pause(&self) -> Result<(), VideoError> {
        self.pause()
    }
    fn controller_toggle(&self) -> Result<(), VideoError> {
        self.toggle()
    }
    fn controller_seek(&self, position: Duration) -> Result<(), VideoError> {
        self.seek(position)
    }
    fn controller_state(&self) -> VideoState {
        self.state()
    }
    fn controller_position(&self) -> Duration {
        self.position()
    }
}

/// A named presentation target with a size.
pub trait VideoSurface {
    /// Stable identity for this surface.
    fn surface_id(&self) -> u64;
    /// Current width and height in pixels.
    fn surface_size(&self) -> (u32, u32);
}

/// A plain in-memory surface description used by the CPU path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuSurface {
    id: u64,
    width: u32,
    height: u32,
}

impl CpuSurface {
    /// A surface description with an id and a size.
    pub fn new(id: u64, width: u32, height: u32) -> Self {
        Self { id, width, height }
    }
}

impl VideoSurface for CpuSurface {
    fn surface_id(&self) -> u64 {
        self.id
    }
    fn surface_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_surface_reports_identity_and_size() {
        let surface = CpuSurface::new(7, 1920, 1080);
        assert_eq!(surface.surface_id(), 7);
        assert_eq!(surface.surface_size(), (1920, 1080));
    }

    #[test]
    fn a_generic_owner_drives_any_controller() {
        #[derive(Default)]
        struct Fake {
            played: bool,
        }
        impl PlaybackController for Fake {
            fn controller_play(&self) -> Result<(), VideoError> {
                Ok(())
            }
            fn controller_pause(&self) -> Result<(), VideoError> {
                Ok(())
            }
            fn controller_toggle(&self) -> Result<(), VideoError> {
                Ok(())
            }
            fn controller_seek(&self, _: Duration) -> Result<(), VideoError> {
                Ok(())
            }
            fn controller_state(&self) -> VideoState {
                VideoState::Idle
            }
            fn controller_position(&self) -> Duration {
                Duration::ZERO
            }
        }
        fn resume<P: PlaybackController>(player: &P) {
            let _ = player.controller_play();
        }
        let fake = Fake::default();
        resume(&fake);
        assert!(!fake.played);
        assert_eq!(fake.controller_state(), VideoState::Idle);
    }
}
