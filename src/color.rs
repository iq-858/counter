//! Unified cross-platform colour metadata.
//!
//! Replaces guessing colour from resolution with explicit stream metadata.
//! Every field has an `Unspecified` state: the player must never assume
//! BT.709 just because a frame is 1920x1080 when the stream says otherwise.

use serde::{Deserialize, Serialize};

/// Colour primaries (ITU-R BT.2020 applications).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ColorPrimaries {
    /// ITU-R BT.601 (SD definition).
    Bt601,
    /// ITU-R BT.709 (HD definition).
    #[default]
    Bt709,
    /// ITU-R BT.2020 (UHD / HDR).
    Bt2020,
    /// The stream did not state its primaries.
    Unspecified,
}

/// Transfer characteristic / electro-optical transfer function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TransferFunction {
    /// BT.709 electro-optical transfer function.
    Bt709,
    /// SMPTE ST 2084 (HDR10).
    Pq,
    /// ARIB STD-B67 (Hybrid Log-Gamma).
    Hlg,
    /// The stream did not state its transfer function.
    #[default]
    Unspecified,
}

/// Where chroma samples sit relative to luma (4:2:0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ChromaLocation {
    /// Chroma samples sit left of luma (MPEG-2 style).
    Left,
    /// Chroma samples are horizontally centred (common default).
    Center,
    /// Chroma samples sit at the top-left (H.264/HEVC style).
    TopLeft,
    /// The stream did not state chroma siting.
    #[default]
    Unspecified,
}

/// Static HDR10 metadata from the stream (SMPTE ST 2086 / MaxCLL/FALL).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HdrMetadata {
    /// Maximum display luminance in nits (SMPTE ST 2086).
    pub max_luminance: f32,
    /// Minimum display luminance in nits.
    pub min_luminance: f32,
    /// Maximum content light level (MaxCLL) in nits.
    pub max_content_light_level: u16,
    /// Maximum frame-average light level (MaxFALL) in nits.
    pub max_frame_average_light_level: u16,
}

/// Complete colour description of a decoded picture.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ColorInfo {
    /// Colour primaries from the stream.
    pub primaries: ColorPrimaries,
    /// Transfer characteristic from the stream.
    pub transfer: TransferFunction,
    /// Whether luma/chroma use full (0-255) rather than limited range.
    pub full_range: bool,
    /// Chroma sample siting for 4:2:0 pictures.
    pub chroma_location: ChromaLocation,
    /// Visible crop inside the coded picture (clean aperture). Values are in
    /// coded pixels; `(0,0,0,0)` means "use the whole picture".
    pub clean_aperture: (u32, u32, u32, u32),
    /// Static HDR metadata when the stream carries it.
    pub hdr: Option<HdrMetadata>,
    /// True when all fields came from authoritative stream metadata, false when
    /// the backend had to guess. Callers can decide to distrust guesses.
    pub authoritative: bool,
}

impl ColorInfo {
    /// A conservative description: BT.709, limited range, no HDR, not authoritative.
    pub fn conservative() -> Self {
        Self {
            primaries: ColorPrimaries::Bt709,
            transfer: TransferFunction::Bt709,
            full_range: false,
            chroma_location: ChromaLocation::Unspecified,
            clean_aperture: (0, 0, 0, 0),
            hdr: None,
            authoritative: false,
        }
    }

    /// Whether the stream is HDR (PQ or HLG transfer).
    pub fn is_hdr(self) -> bool {
        matches!(self.transfer, TransferFunction::Pq | TransferFunction::Hlg)
    }

    /// Validates a crop against a coded size. Returns `None` when invalid.
    pub fn visible_size(self, coded: (u32, u32)) -> Option<(u32, u32)> {
        let (left, top, right, bottom) = self.clean_aperture;
        let (w, h) = coded;
        if left.saturating_add(right) >= w || top.saturating_add(bottom) >= h {
            if self.clean_aperture == (0, 0, 0, 0) {
                return Some(coded);
            }
            return None;
        }
        Some((w - left - right, h - top - bottom))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conservative_is_not_authoritative() {
        let info = ColorInfo::conservative();
        assert!(!info.authoritative);
        assert!(!info.is_hdr());
    }

    #[test]
    fn hdr_detection() {
        let mut info = ColorInfo::conservative();
        info.transfer = TransferFunction::Pq;
        assert!(info.is_hdr());
        info.transfer = TransferFunction::Hlg;
        assert!(info.is_hdr());
    }

    #[test]
    fn clean_aperture_valid() {
        let mut info = ColorInfo::conservative();
        info.clean_aperture = (2, 2, 2, 2);
        assert_eq!(info.visible_size((1920, 1080)), Some((1916, 1076)));
    }

    #[test]
    fn clean_aperture_invalid() {
        let mut info = ColorInfo::conservative();
        info.clean_aperture = (1000, 0, 1000, 0);
        assert_eq!(info.visible_size((1920, 1080)), None);
    }

    #[test]
    fn zero_crop_means_full_picture() {
        let info = ColorInfo::conservative();
        assert_eq!(info.visible_size((1280, 720)), Some((1280, 720)));
    }
}
