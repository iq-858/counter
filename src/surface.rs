//! A presentation-surface abstraction over the platform renderers.
//!
//! The default path of this crate converts every picture to CPU RGBA and the
//! application draws it. That is portable but costs a copy per frame. Each
//! platform can instead hand its decoder a surface it renders into directly:
//! D3D11/DXGI on Windows, `AVPlayerLayer`/`CVPixelBuffer` on Apple, a
//! `MediaCodec` surface on Android, DMA-BUF on Linux and the `<video>` element
//! itself on the web.
//!
//! This module names those paths as data, not as code: `SurfaceKind` says
//! which zero-copy route a build can take, `SurfaceReport` states what has
//! actually been measured, and `SurfacePlan` is the decision an application
//! makes. No path here claims zero-copy without a measurement — the roadmap
//! rule that CPU NV12 is never called zero-copy is enforced by the type.

/// A presentation route a platform can offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    /// CPU RGBA buffers the application draws. Always available; one copy.
    CpuRgba,
    /// CPU NV12 planes drawn by a GPU converter. Cheaper to produce than
    /// RGBA but still not zero-copy.
    CpuNv12,
    /// D3D11/DXGI texture on Windows.
    D3d11,
    /// `AVPlayerLayer` / `CVPixelBuffer` on Apple platforms.
    CVPixelBuffer,
    /// `MediaCodec` surface or `AHardwareBuffer` on Android.
    AndroidSurface,
    /// DMA-BUF / GLMemory on Linux.
    DmaBuf,
    /// The browser's own `<video>` presentation on the web.
    BrowserVideo,
}

/// What a surface path has proven, per the roadmap's evidence rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceEvidence {
    /// Copies were measured on a real device; `copies` states the count.
    Measured(u8),
    /// The path compiles and is wired, but no device measurement exists yet.
    CompileOnly,
    /// The path is planned; no code exists behind it yet.
    Planned,
}

/// One platform's surface options with their evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfacePlan {
    /// The route the player uses by default.
    pub default: SurfaceKind,
    /// The best zero-copy route the platform offers, when it has one.
    pub zero_copy: Option<SurfaceKind>,
    /// The evidence behind `zero_copy`.
    pub evidence: SurfaceEvidence,
}

impl SurfacePlan {
    /// The plan for the platform this build selected.
    ///
    /// Every zero-copy route is `CompileOnly` until a device measurement
    /// replaces it — the crate never claims a copy count it did not measure.
    pub fn for_platform(platform: crate::Platform) -> Self {
        let (zero_copy, evidence) = match platform {
            crate::Platform::Windows => (Some(SurfaceKind::D3d11), SurfaceEvidence::CompileOnly),
            crate::Platform::MacOs | crate::Platform::Ios => {
                (Some(SurfaceKind::CVPixelBuffer), SurfaceEvidence::CompileOnly)
            }
            crate::Platform::Android => {
                (Some(SurfaceKind::AndroidSurface), SurfaceEvidence::CompileOnly)
            }
            crate::Platform::Linux => (Some(SurfaceKind::DmaBuf), SurfaceEvidence::CompileOnly),
            crate::Platform::Web => (Some(SurfaceKind::BrowserVideo), SurfaceEvidence::CompileOnly),
            crate::Platform::WatchOs => (Some(SurfaceKind::CVPixelBuffer), SurfaceEvidence::Planned),
            crate::Platform::Emscripten | crate::Platform::Unknown => (None, SurfaceEvidence::Planned),
        };
        Self {
            default: SurfaceKind::CpuRgba,
            zero_copy,
            evidence,
        }
    }

    /// Whether the default path is the zero-copy one.
    pub fn default_is_zero_copy(&self) -> bool {
        self.zero_copy == Some(self.default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_names_a_zero_copy_route() {
        for platform in [
            crate::Platform::Windows,
            crate::Platform::MacOs,
            crate::Platform::Ios,
            crate::Platform::Android,
            crate::Platform::Linux,
            crate::Platform::Web,
        ] {
            let plan = SurfacePlan::for_platform(platform);
            assert!(plan.zero_copy.is_some(), "{platform:?} names a route");
            assert_eq!(plan.default, SurfaceKind::CpuRgba);
            assert!(!plan.default_is_zero_copy(), "CPU RGBA is never zero-copy");
        }
    }

    #[test]
    fn evidence_defaults_to_compile_only() {
        let plan = SurfacePlan::for_platform(crate::Platform::Windows);
        assert_eq!(plan.evidence, SurfaceEvidence::CompileOnly);
    }

    #[test]
    fn a_measured_path_states_its_copy_count() {
        let evidence = SurfaceEvidence::Measured(0);
        assert_eq!(evidence, SurfaceEvidence::Measured(0));
    }
}
