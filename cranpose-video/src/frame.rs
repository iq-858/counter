use std::{fmt, sync::OnceLock, time::Duration};

use crate::pixels;

/// How a frame's pixels are laid out in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PixelFormat {
    /// Four bytes per pixel in red, green, blue, alpha order, no padding.
    #[default]
    Rgba8,
    /// 8-bit 4:2:0 with a full-size luma plane followed by one interleaved
    /// chroma plane at half height, both unpadded — the layout every hardware
    /// decoder hands out and every GPU can sample as two textures.
    Nv12,
}

/// The matrix that turns luma and chroma into red, green and blue.
///
/// Only the conversion needs it: a frame that already carries RGBA ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorMatrix {
    /// Standard definition, and what H.264 streams before HD resolution use.
    Bt601,
    /// High definition and above, and what the platform decoders state for a
    /// modern track.
    #[default]
    Bt709,
    /// Ultra high definition and HDR tracks in their SDR-compatible form.
    Bt2020,
}

/// Whether samples use the studio swing or the full byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorRange {
    /// Luma runs from 16 to 235 and chroma from 16 to 240, as broadcast and
    /// most camera files state.
    #[default]
    Limited,
    /// Luma runs from 0 to 255, as a full-range flag states.
    Full,
}

/// The colour space a picture's luma and chroma samples are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct YuvColor {
    matrix: ColorMatrix,
    range: ColorRange,
}

impl YuvColor {
    /// BT.709 studio swing, which is what a high-definition track means when
    /// it states nothing else.
    pub const BT709_LIMITED: Self = Self {
        matrix: ColorMatrix::Bt709,
        range: ColorRange::Limited,
    };

    /// A colour space from its matrix and range.
    pub const fn new(matrix: ColorMatrix, range: ColorRange) -> Self {
        Self { matrix, range }
    }

    /// The matrix.
    pub const fn matrix(self) -> ColorMatrix {
        self.matrix
    }

    /// Whether the samples are studio swing or full range.
    pub const fn range(self) -> ColorRange {
        self.range
    }

    /// The colour space a track of this size conventionally uses.
    ///
    /// A container that states nothing leaves a decoder to infer the matrix
    /// from the resolution, which is what every system player does: 720 lines
    /// and up are BT.709, below that BT.601. A backend that knows the real
    /// answer overrides this.
    pub const fn for_size(width: u32, height: u32) -> Self {
        let matrix = if width >= 1280 || height >= 720 {
            ColorMatrix::Bt709
        } else {
            ColorMatrix::Bt601
        };
        Self::new(matrix, ColorRange::Limited)
    }
}

impl Default for YuvColor {
    fn default() -> Self {
        Self::BT709_LIMITED
    }
}

/// One decoded picture, as RGBA8 rows in top-down order or as the decoder's own
/// NV12 planes.
///
/// An RGBA frame's bytes are exactly `width * height * 4`, four bytes per pixel
/// in red, green, blue, alpha order, with no padding between rows. That is the
/// layout a GPU texture upload takes as it is, and the layout
/// [`ImageBitmap::from_rgba8`](https://docs.rs/cranpose-ui-graphics) reads.
///
/// A frame a decoder handed over as NV12 keeps its planes until something asks
/// for RGBA, so a screen that can sample two textures never pays for the
/// conversion. Reading [`rgba`](Self::rgba) or calling
/// [`into_rgba`](Self::into_rgba) converts once — across every core — and
/// caches the result, so the cost is paid at most once per picture.
pub struct VideoFrame {
    width: u32,
    height: u32,
    pts: Duration,
    native: Native,
    rgba: OnceLock<Vec<u8>>,
}

/// The pixels in the layout the decoder produced them in.
enum Native {
    Rgba,
    Nv12 { planes: Vec<u8>, color: YuvColor },
}

impl VideoFrame {
    /// A frame from bytes that already carry the RGBA8 layout.
    ///
    /// Returns `None` when a dimension is zero or the byte count does not match
    /// the size, so a decoder cannot hand a short buffer or an empty picture to
    /// a texture upload. A zero-sized buffer passes the length check on its own;
    /// rejecting it here is what lets the player report a bad decode instead of
    /// publishing a picture no screen can show.
    pub fn from_rgba8(width: u32, height: u32, pts: Duration, rgba: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        // Checked arithmetic: hostile metadata with huge dimensions could wrap
        // the product to a small number and let a short buffer through.
        let Some(expected) = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            return None;
        };
        (rgba.len() == expected).then(|| Self {
            width,
            height,
            pts,
            native: Native::Rgba,
            rgba: OnceLock::from(rgba),
        })
    }

    /// A frame from the NV12 planes a hardware decoder produced.
    ///
    /// `planes` holds `width * height` luma bytes followed by `width * height / 2`
    /// interleaved chroma bytes, both unpadded, which is the layout Media
    /// Foundation, MediaCodec and VideoToolbox hand out once their stride is
    /// normalised. Returns `None` for an odd size or a byte count that does not
    /// match it.
    pub fn from_nv12(
        width: u32,
        height: u32,
        pts: Duration,
        planes: Vec<u8>,
        color: YuvColor,
    ) -> Option<Self> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return None;
        }
        let Some(expected) = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(3))
            .map(|bytes| bytes / 2)
        else {
            return None;
        };
        (planes.len() == expected).then(|| Self {
            width,
            height,
            pts,
            native: Native::Nv12 { planes, color },
            rgba: OnceLock::new(),
        })
    }

    /// The frame's width in pixels, before the track's rotation.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The frame's height in pixels, before the track's rotation.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The frame's position in the media.
    pub fn pts(&self) -> Duration {
        self.pts
    }

    /// The layout the frame arrived in.
    pub fn format(&self) -> PixelFormat {
        match self.native {
            Native::Rgba => PixelFormat::Rgba8,
            Native::Nv12 { .. } => PixelFormat::Nv12,
        }
    }

    /// The colour space of the native planes, for a screen that samples them.
    pub fn color(&self) -> Option<YuvColor> {
        match &self.native {
            Native::Rgba => None,
            Native::Nv12 { color, .. } => Some(*color),
        }
    }

    /// The luma plane of an NV12 frame, `width * height` bytes.
    pub fn luma(&self) -> Option<&[u8]> {
        match &self.native {
            Native::Rgba => None,
            Native::Nv12 { planes, .. } => {
                Some(&planes[..self.width as usize * self.height as usize])
            }
        }
    }

    /// The interleaved chroma plane of an NV12 frame, `width * height / 2` bytes.
    pub fn chroma(&self) -> Option<&[u8]> {
        match &self.native {
            Native::Rgba => None,
            Native::Nv12 { planes, .. } => {
                Some(&planes[self.width as usize * self.height as usize..])
            }
        }
    }

    /// The pixels, as RGBA8 rows, converting an NV12 frame the first time.
    pub fn rgba(&self) -> &[u8] {
        self.rgba.get_or_init(|| self.convert())
    }

    /// Converts an NV12 frame now, on the calling thread.
    ///
    /// The decode thread calls this so the screen thread only ever copies RGBA
    /// into a texture, never converts. A screen that samples the planes itself
    /// skips it and keeps the conversion off the CPU entirely.
    pub fn materialize(&self) {
        let _ = self.rgba();
    }

    /// Converts the frame into a buffer the caller owns and reuses.
    ///
    /// A reused buffer that is already the right length costs no allocation and
    /// no clearing, which is what a decode thread wants: one picture per frame
    /// out of a pool rather than eight megabytes from the allocator. An RGBA
    /// frame copies its pixels, so the same call works for both layouts.
    pub fn convert_into(&self, rgba: &mut Vec<u8>) {
        let width = self.width as usize;
        let height = self.height as usize;
        let needed = width * height * 4;
        if rgba.len() != needed {
            rgba.resize(needed, 0);
        }
        match &self.native {
            Native::Rgba => rgba.copy_from_slice(self.rgba()),
            Native::Nv12 { planes, color } => {
                pixels::nv12_to_rgba(planes, width, height, *color, rgba);
            }
        }
    }

    /// Caches pixels the caller converted, so a later read returns them.
    ///
    /// Returns the buffer when the frame already held its pixels, so the caller
    /// can put it back in its pool instead of dropping it.
    pub(crate) fn cache_rgba(&self, rgba: Vec<u8>) -> Option<Vec<u8>> {
        self.rgba.set(rgba).err()
    }

    /// Whether the RGBA pixels are already there.
    pub fn has_rgba(&self) -> bool {
        self.rgba.get().is_some()
    }

    /// The pixels, taken out of the frame.
    ///
    /// An NV12 frame is converted here, across every core, so this is the call
    /// that costs. Call [`materialize`](Self::materialize) on the decode thread
    /// to pay it there instead.
    pub fn into_rgba(mut self) -> Vec<u8> {
        match self.rgba.take() {
            Some(rgba) => rgba,
            None => self.convert(),
        }
    }

    /// The size of the pixels in bytes, in the layout the frame carries.
    pub fn byte_len(&self) -> usize {
        match &self.native {
            Native::Rgba => self.width as usize * self.height as usize * 4,
            Native::Nv12 { planes, .. } => planes.len(),
        }
    }

    fn convert(&self) -> Vec<u8> {
        let mut rgba = Vec::new();
        self.convert_into(&mut rgba);
        rgba
    }

    /// Gives the pixel buffers back so a player can hand them to the decoder
    /// again instead of allocating a picture per frame.
    ///
    /// The first value is the RGBA buffer, present when it was materialised; the
    /// second is the NV12 plane buffer, present when the frame still holds it.
    pub(crate) fn into_buffers(self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        let rgba = self.rgba.into_inner();
        let planes = match self.native {
            Native::Nv12 { planes, .. } => Some(planes),
            Native::Rgba => None,
        };
        (rgba, planes)
    }

    /// Takes the NV12 planes back once they have been converted, so the same
    /// buffer decodes the next picture.
    pub(crate) fn release_planes(&mut self) -> Option<Vec<u8>> {
        match std::mem::replace(&mut self.native, Native::Rgba) {
            Native::Nv12 { planes, .. } => Some(planes),
            Native::Rgba => None,
        }
    }
}

impl Clone for VideoFrame {
    fn clone(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            pts: self.pts,
            native: match &self.native {
                Native::Rgba => Native::Rgba,
                Native::Nv12 { planes, color } => Native::Nv12 {
                    planes: planes.clone(),
                    color: *color,
                },
            },
            rgba: match self.rgba.get() {
                Some(rgba) => OnceLock::from(rgba.clone()),
                None => OnceLock::new(),
            },
        }
    }
}

impl PartialEq for VideoFrame {
    fn eq(&self, other: &Self) -> bool {
        if self.width != other.width || self.height != other.height || self.pts != other.pts {
            return false;
        }
        match (&self.native, &other.native) {
            (Native::Rgba, Native::Rgba) => self.rgba() == other.rgba(),
            (
                Native::Nv12 {
                    planes,
                    color: self_color,
                },
                Native::Nv12 {
                    planes: other_planes,
                    color: other_color,
                },
            ) => planes == other_planes && self_color == other_color,
            _ => false,
        }
    }
}

impl Eq for VideoFrame {}

impl fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pts", &self.pts)
            .field("format", &self.format())
            .field("byte_len", &self.byte_len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nv12(width: usize, height: usize) -> Vec<u8> {
        let mut planes = vec![128u8; width * height];
        planes.resize(width * height * 3 / 2, 128);
        planes
    }

    #[test]
    fn an_nv12_frame_converts_once_and_caches() {
        let frame =
            VideoFrame::from_nv12(4, 4, Duration::ZERO, nv12(4, 4), YuvColor::default()).unwrap();
        assert_eq!(frame.format(), PixelFormat::Nv12);
        assert!(!frame.has_rgba());
        assert_eq!(frame.byte_len(), 4 * 4 * 3 / 2);
        assert_eq!(frame.rgba().len(), 4 * 4 * 4);
        assert!(frame.has_rgba(), "the first read caches the pixels");
        assert_eq!(frame.luma().unwrap().len(), 16);
        assert_eq!(frame.chroma().unwrap().len(), 8);
    }

    #[test]
    fn an_nv12_frame_of_the_wrong_length_is_refused() {
        assert!(
            VideoFrame::from_nv12(4, 4, Duration::ZERO, vec![0; 10], YuvColor::default()).is_none()
        );
        // An odd size has no 4:2:0 layout to fill.
        assert!(
            VideoFrame::from_nv12(5, 4, Duration::ZERO, vec![0; 30], YuvColor::default()).is_none()
        );
    }

    #[test]
    fn planes_come_back_for_the_next_decode() {
        let mut frame =
            VideoFrame::from_nv12(4, 4, Duration::ZERO, nv12(4, 4), YuvColor::default()).unwrap();
        frame.materialize();
        let planes = frame.release_planes().expect("an NV12 frame holds planes");
        assert_eq!(planes.len(), 4 * 4 * 3 / 2);
        assert_eq!(frame.format(), PixelFormat::Rgba8);
        assert_eq!(frame.byte_len(), 4 * 4 * 4);
    }

    #[test]
    fn an_rgba_frame_is_unchanged_by_a_round_trip() {
        let bytes: Vec<u8> = (0..64).collect();
        let frame = VideoFrame::from_rgba8(4, 4, Duration::from_millis(40), bytes.clone())
            .expect("the byte count matches the size");
        assert_eq!(frame.format(), PixelFormat::Rgba8);
        assert!(frame.has_rgba());
        let (rgba, planes) = frame.into_buffers();
        assert_eq!(rgba.unwrap(), bytes);
        assert!(planes.is_none());
    }

    #[test]
    fn a_zero_sized_rgba_frame_is_refused_even_with_a_matching_buffer() {
        // An empty buffer has the expected length for 0x0, so the byte-count
        // check alone cannot catch this picture. Zero dimensions must be
        // rejected so the player can report a decode failure for it.
        assert!(VideoFrame::from_rgba8(0, 0, Duration::ZERO, Vec::new()).is_none());
        assert!(VideoFrame::from_rgba8(0, 4, Duration::ZERO, Vec::new()).is_none());
        assert!(VideoFrame::from_rgba8(4, 0, Duration::ZERO, Vec::new()).is_none());
    }

    #[test]
    fn a_normal_rgba_frame_is_still_accepted() {
        let frame = VideoFrame::from_rgba8(2, 3, Duration::ZERO, vec![0; 2 * 3 * 4])
            .expect("a positive size with a matching buffer is valid");
        assert_eq!(frame.width(), 2);
        assert_eq!(frame.height(), 3);
        assert_eq!(frame.byte_len(), 2 * 3 * 4);
    }
}
