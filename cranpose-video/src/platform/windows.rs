//! Media Foundation: the Windows media stack the system's own players use.
//!
//! A source reader demuxes the file and decodes the video stream with the
//! platform's decoder. Planar NV12 is preferred: the reader hands over the
//! decoder's own planes, which are repacked and converted here. RGB32 stays the
//! fallback for turned pictures and formats the planar path cannot serve.
//! H.264, HEVC, VP9, AV1 and MPEG-2 decode in hardware wherever the driver
//! offers it.
#![expect(
    unsafe_code,
    reason = "Media Foundation is a COM API, so every call crosses an FFI boundary"
)]

use std::{
    mem::{ManuallyDrop, size_of},
    ptr, slice,
    sync::OnceLock,
    time::Duration,
};

use windows::{
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        Media::MediaFoundation::{
            IMF2DBuffer, IMFAttributes, IMFMediaBuffer, IMFMediaType, IMFSample, IMFSourceReader,
            MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
            MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_SUBTYPE,
            MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_ROTATION, MF_MT_YUV_MATRIX, MF_PD_DURATION,
            MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READER_ALL_STREAMS,
            MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
            MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READER_MEDIASOURCE_CHARACTERISTICS,
            MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED, MF_SOURCE_READERF_ENDOFSTREAM,
            MF_SOURCE_READERF_ERROR, MF_SOURCE_READERF_STREAMTICK, MF_VERSION, MFCreateAttributes,
            MFCreateMFByteStreamOnStream, MFCreateMediaType, MFCreateSourceReaderFromByteStream,
            MFCreateSourceReaderFromURL, MFMEDIASOURCE_CAN_SEEK, MFMediaType_Video, MFNominalRange,
            MFNominalRange_0_255, MFNominalRange_16_235, MFSTARTUP_LITE, MFStartup, MFVideoArea,
            MFVideoFormat_NV12, MFVideoFormat_RGB32, MFVideoTransferMatrix,
            MFVideoTransferMatrix_BT601, MFVideoTransferMatrix_BT709,
            MFVideoTransferMatrix_BT2020_10, MFVideoTransferMatrix_BT2020_12,
            MFVideoTransferMatrix_SMPTE240M,
        },
        System::{
            Com::{
                COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
                StructuredStorage::{
                    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                },
            },
            Variant::{VT_I4, VT_I8, VT_UI4, VT_UI8},
        },
        UI::Shell::SHCreateMemStream,
    },
    core::{GUID, HSTRING, Interface},
};

use crate::{
    ColorMatrix, ColorRange, DecodedFrame, FrameRate, PixelFormat, Rotation, VideoDecoder,
    VideoError, VideoInfo, VideoSource, YuvColor,
    pixels::{bgra_to_rgba, nv12_to_rgba},
};

pub(crate) const NAME: &str = "Media Foundation";
pub(crate) const SUPPORTED: bool = true;

#[cfg(feature = "native-audio")]
mod mf_audio;

/// Opens a video with the source reader.
pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn VideoDecoder>, VideoError> {
    start()?;
    // The first WASAPI device lookup creates cpal's PROCESS-GLOBAL
    // IMMDeviceEnumerator (cpal device.rs:1145) inside the caller's COM
    // apartment. That object is cached forever and never rebuilt. It must
    // therefore live in an apartment that never goes away: `open` runs on the
    // UI thread here, which is alive for the whole process. If we let this
    // apartment end at `open` exit, every later player reuses a COM pointer
    // whose apartment is gone, so audio dies on all channels after the first
    // failed switch and no retry can bring it back. Media Foundation's own
    // COM needs are met separately by the decode thread's `THREAD` apartment.
    Apartment::join_for_process();
    let attributes = attributes()?;
    let reader = match source {
        VideoSource::Bytes(bytes) => open_bytes(bytes, &attributes),
        VideoSource::Path(path) => {
            let text = path.to_str().ok_or_else(|| {
                VideoError::Source("the file path is not valid Unicode".to_owned())
            })?;
            open_url(text, &attributes)
        }
        VideoSource::Url(url) => open_url(url, &attributes),
    }?;
    let (info, can_seek, layout) = prepare(&reader)?;
    // The same reader owns both tracks. Audio setup is deliberately deferred to
    // the decode thread: touching the output device on the UI thread both
    // violated the "no device work on the UI thread" boundary and made a busy
    // device a terminal open failure. It now runs on the first `read()` and is
    // retried by the session's recovery loop while the source really has audio.
    Ok(Box::new(Reader {
        reader: Some(SendReader(reader)),
        info,
        can_seek,
        origin: None,
        layout,
        // Reused by `read` when a planar decoder is reached through RGB-only
        // callers: the planes are copied once here, then converted in place.
        scratch: Vec::new(),
        #[cfg(feature = "native-audio")]
        audio: None,
        #[cfg(feature = "native-audio")]
        audio_pending: true,
        #[cfg(feature = "native-audio")]
        source_has_audio: false,
        #[cfg(feature = "native-audio")]
        audio_ever_present: false,
        // Nothing has asked for sound yet; the first `play` or setter records
        // it so a device rebuilt later resumes instead of starting silent.
        #[cfg(feature = "native-audio")]
        playing: false,
        #[cfg(feature = "native-audio")]
        speed: 1.0,
        #[cfg(feature = "native-audio")]
        muted: false,
        #[cfg(feature = "native-audio")]
        volume: 1.0,
        #[cfg(all(
            feature = "native-audio",
            not(target_os = "linux"),
            not(target_arch = "wasm32")
        ))]
        session: None,
    }))
}

fn open_url(text: &str, attributes: &IMFAttributes) -> Result<IMFSourceReader, VideoError> {
    let text = HSTRING::from(text);
    // SAFETY: the text stays alive for the call, and the attributes stay valid
    // for as long as the reader keeps them.
    unsafe { MFCreateSourceReaderFromURL(&text, attributes) }
        .map_err(|error| VideoError::Source(format!("cannot open the media: {error}")))
}

fn open_bytes(bytes: &[u8], attributes: &IMFAttributes) -> Result<IMFSourceReader, VideoError> {
    // SAFETY: the stream copies the bytes before the slice goes away, so no
    // borrowed memory reaches Media Foundation.
    let stream = unsafe { SHCreateMemStream(Some(bytes)) }
        .ok_or_else(|| VideoError::Source("cannot copy the clip's bytes into memory".to_owned()))?;
    // SAFETY: the stream owns a reference of its own once the byte stream is
    // built, so it outlives this scope.
    let stream = unsafe { MFCreateMFByteStreamOnStream(&stream) }
        .map_err(|error| VideoError::Source(format!("cannot read the clip's bytes: {error}")))?;
    // SAFETY: the byte stream and the attributes stay alive for the call.
    unsafe { MFCreateSourceReaderFromByteStream(&stream, attributes) }
        .map_err(|error| VideoError::Source(format!("cannot open the clip: {error}")))
}

/// Builds the attributes the reader needs to decode video into a usable format.
fn attributes() -> Result<IMFAttributes, VideoError> {
    let mut attributes: Option<IMFAttributes> = None;
    // SAFETY: the out pointer refers to a local the call fills.
    unsafe { MFCreateAttributes(&mut attributes, 1) }.map_err(|error| {
        VideoError::Source(format!("cannot create the reader's attributes: {error}"))
    })?;
    let attributes = attributes
        .ok_or_else(|| VideoError::Source("Media Foundation created no attributes".to_owned()))?;
    // The source reader's video processor converts the stream to RGB32 when
    // the planar path cannot serve it, deinterlaces it, and applies the turn
    // the track asks for.
    // SAFETY: the keys are constants and the values are integers. Hardware
    // transforms let the platform's own decoder handle the formats it knows
    // best, including planar output. The advanced processor is deliberately
    // not requested: it conflicts with the basic processor and makes even the
    // tiny fixture clip fail to open.
    unsafe {
        attributes
            .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
            .map_err(|error| {
                VideoError::Source(format!("cannot enable video processing: {error}"))
            })?;
        attributes
            .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
            .map_err(|error| {
                VideoError::Source(format!("cannot enable hardware transforms: {error}"))
            })?;
    }
    Ok(attributes)
}

/// Selects the video stream, asks for RGB32, and reads what the source states.
fn prepare(reader: &IMFSourceReader) -> Result<(VideoInfo, bool, Layout), VideoError> {
    // SAFETY: the stream selectors are the constants the reader defines for
    // this call, and the reader is open.
    unsafe {
        reader
            .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
            .map_err(|error| {
                VideoError::Source(format!("cannot limit the reader to video: {error}"))
            })?;
        reader
            .SetStreamSelection(video(), true)
            .map_err(|_| VideoError::NoVideoTrack)?;
    }

    // SAFETY: the first media type of the video stream is what the container
    // states about the track.
    let native =
        unsafe { reader.GetNativeMediaType(video(), 0) }.map_err(|_| VideoError::NoVideoTrack)?;
    let rotation = rotation(&native);
    let native_size = frame_size(&native);

    // NV12 is the layout the decoder and the GPU both prefer: a picture is 1.5
    // bytes a pixel instead of 4, and the source reader's video processor need
    // not turn it into RGB. A track that asks for a turn keeps RGB32, because
    // the processor bakes the turn into the conversion it already does.
    let mut output = if rotation == Rotation::None {
        nv12(reader)
    } else {
        None
    };
    if output.is_none() {
        output = Some(rgb32(reader)?);
    }
    let mut output = output.expect("one of the two types was set");
    let (width, height) = display_size(&output)
        .or(native_size)
        .ok_or_else(|| VideoError::Source("the video states no picture size".to_owned()))?;
    // A 4:2:0 picture needs even sides. A stream that states an odd one keeps
    // the RGB path, which has no such rule.
    if width % 2 != 0 || height % 2 != 0 {
        output = rgb32(reader)?;
    }
    let layout = layout_from(&output, (width as usize, height as usize));

    // The video processor turns the picture upright and reports the turned
    // size. A quarter turn shows in that size. A half turn keeps the size, so
    // it reads as applied, as a turned picture must.
    let native_size = native_size.unwrap_or((width, height));
    let upright = rotation.display_size(native_size.0, native_size.1);
    let baked = match rotation {
        Rotation::None => false,
        Rotation::Clockwise180 => true,
        Rotation::Clockwise90 | Rotation::Clockwise270 => upright == (width, height),
    };

    Ok((
        VideoInfo {
            width,
            height,
            duration: duration(reader),
            frame_rate: frame_rate(&native),
            rotation: if baked { Rotation::None } else { rotation },
        },
        can_seek(reader),
        layout,
    ))
}

/// Where a decoded picture's rows sit in the buffer the decoder hands over.
///
/// The size the player publishes is the *display* size. The decoder may write a
/// wider, padded surface and ask the caller to crop it, so the coded size and
/// the crop are kept beside the stride. Nothing here is trusted blindly: the
/// buffer's own pitch and length are checked again at copy time.
#[derive(Clone, Copy)]
struct Layout {
    format: PixelFormat,
    /// The stride the media type declares for one luma row. A hint only.
    stride: usize,
    /// The surface the decoder actually writes, which may be padded.
    coded: (usize, usize),
    /// Where the display picture starts inside that surface, in luma pixels.
    crop: (usize, usize),
    /// The colour space the decoder states, or the size-based fallback.
    color: YuvColor,
}

/// Builds the geometry from the type the decoder currently reports.
fn layout_from(media_type: &IMFMediaType, display: (usize, usize)) -> Layout {
    let format = if is_nv12(media_type) {
        PixelFormat::Nv12
    } else {
        PixelFormat::Rgba8
    };
    // A changed type can report a padded coded size. Keep the display size the
    // player published and record the surface around it separately.
    let coded = frame_size(media_type)
        .map(|(width, height)| (width as usize, height as usize))
        .filter(|&(width, height)| width >= display.0 && height >= display.1)
        .unwrap_or(display);
    // A crop that would leave no room for the picture is not a crop: ignore it
    // rather than reading rows outside the surface.
    let crop = display_crop(media_type)
        .filter(|&(x, y)| x + display.0 <= coded.0 && y + display.1 <= coded.1 && y % 2 == 0)
        .unwrap_or((0, 0));
    let stride = default_stride(media_type)
        .filter(|&stride| stride >= display.0)
        .unwrap_or(display.0);
    let color = yuv_color(
        media_type,
        YuvColor::for_size(display.0 as u32, display.1 as u32),
    );
    Layout {
        format,
        stride,
        coded,
        crop,
        color,
    }
}

/// The display aperture's offset inside the coded surface, when one is stated.
fn display_crop(media_type: &IMFMediaType) -> Option<(usize, usize)> {
    let mut bytes = [0u8; size_of::<MFVideoArea>()];
    // SAFETY: the blob is copied into a buffer of exactly the type's size, and
    // the struct is read unaligned because a blob carries no alignment guarantee.
    unsafe {
        media_type
            .GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut bytes, None)
            .ok()?;
        let area: MFVideoArea = ptr::read_unaligned(bytes.as_ptr().cast::<MFVideoArea>());
        Some((
            usize::try_from(area.OffsetX.value).ok()?,
            usize::try_from(area.OffsetY.value).ok()?,
        ))
    }
}

/// The colour space the decoder states, falling back per component when it does
/// not state one. Old encoders often set the matrix but omit the range.
fn yuv_color(media_type: &IMFMediaType, fallback: YuvColor) -> YuvColor {
    // SAFETY: the keys are constants and both values are 32-bit enumerations.
    let matrix = unsafe { media_type.GetUINT32(&MF_MT_YUV_MATRIX) }
        .ok()
        .and_then(|value| {
            let value = MFVideoTransferMatrix(value as i32);
            if value == MFVideoTransferMatrix_BT709 {
                Some(ColorMatrix::Bt709)
            } else if value == MFVideoTransferMatrix_BT601
                || value == MFVideoTransferMatrix_SMPTE240M
            {
                Some(ColorMatrix::Bt601)
            } else if value == MFVideoTransferMatrix_BT2020_10
                || value == MFVideoTransferMatrix_BT2020_12
            {
                Some(ColorMatrix::Bt2020)
            } else {
                None
            }
        });
    let range = unsafe { media_type.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE) }
        .ok()
        .and_then(|value| {
            let value = MFNominalRange(value as i32);
            if value == MFNominalRange_0_255 {
                Some(ColorRange::Full)
            } else if value == MFNominalRange_16_235 {
                Some(ColorRange::Limited)
            } else {
                None
            }
        });
    YuvColor::new(
        matrix.unwrap_or(fallback.matrix()),
        range.unwrap_or(fallback.range()),
    )
}

/// Whether a display picture and its chroma fit a surface of this pitch.
///
/// Both planes are checked before a single row is copied, so a padded, cropped
/// or unexpectedly small surface is rejected instead of smearing pixels read
/// from outside the picture.
fn nv12_fits(
    coded: (usize, usize),
    crop: (usize, usize),
    row: usize,
    height: usize,
    pitch: usize,
) -> bool {
    let (coded_width, coded_height) = coded;
    let (crop_x, crop_y) = crop;
    row > 0
        && row % 2 == 0
        && height > 0
        && height % 2 == 0
        && coded_height % 2 == 0
        && pitch >= crop_x.saturating_add(row)
        && coded_width >= crop_x.saturating_add(row)
        && coded_height >= crop_y.saturating_add(height)
        // Chroma is subsampled: an odd horizontal offset still names a whole
        // 2x1 chroma sample at `crop_x / 2`, which covers the first two luma
        // pixels. An odd vertical offset would straddle two chroma rows.
        && crop_y % 2 == 0
}

/// Bytes an unpainted NV12 picture of this display size occupies.
fn plane_len(width: usize, height: usize) -> Option<usize> {
    width
        .checked_mul(height)?
        .checked_add(width.checked_mul(height / 2)?)
}

/// Bytes a padded NV12 surface of this pitch and coded height occupies.
fn plane_bytes(pitch: usize, coded_height: usize) -> Option<usize> {
    pitch
        .checked_mul(coded_height)?
        .checked_add(pitch.checked_mul(coded_height / 2)?)
}

/// The lowest and highest byte offsets a run of rows touches.
fn row_span(start: isize, step: isize, rows: usize, row: usize) -> Option<(isize, isize)> {
    let last = start.checked_add(
        isize::try_from(rows.checked_sub(1)?)
            .ok()?
            .checked_mul(step)?,
    )?;
    let end = start.max(last).checked_add(isize::try_from(row).ok()?)?;
    Some((start.min(last), end))
}

/// The valid byte-offset window of a pitched surface.
///
/// A top-down surface starts at byte zero. A bottom-up surface starts at its
/// last row, so the first row lies `needed` bytes below the pointer; a crop
/// shifts that first row right by `crop_x`, which widens the upper edge by the
/// same amount. Keeping this in one place makes the crop requirement testable
/// without a decoder.
fn surface_bounds(
    step: isize,
    needed: usize,
    crop_x: usize,
    row: usize,
) -> Result<(isize, isize), VideoError> {
    let bound = isize::try_from(needed)
        .map_err(|_| VideoError::Decode("the surface size overflows".to_owned()))?;
    if step < 0 {
        let upper = crop_x
            .checked_add(row)
            .and_then(|end| isize::try_from(end).ok())
            .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))?;
        Ok((-bound, upper))
    } else {
        Ok((0, bound))
    }
}

/// Whether an RGBA surface of `stride` by `height` fits `available` bytes.
///
/// `Lock2D` reports the pitch, but only `GetCurrentLength` proves the decoder
/// handed over every pitched row. Both must agree before any row is read.
fn rgba_fits(stride: usize, height: usize, available: usize) -> Result<(), VideoError> {
    let needed = stride
        .checked_mul(height)
        .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))?;
    if available < needed {
        return Err(VideoError::Decode(
            "the decoder handed over a picture shorter than its size".to_owned(),
        ));
    }
    Ok(())
}

/// Switches the video stream to NV12 and returns the type the decoder accepted.
///
/// `None` when this decoder or its processor will not produce planar output, in
/// which case the caller asks for RGB32 as it always did.
fn nv12(reader: &IMFSourceReader) -> Option<IMFMediaType> {
    // SAFETY: the media type is created here, filled with constant keys, and
    // handed to the reader, which copies it.
    unsafe {
        let output = MFCreateMediaType().ok()?;
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).ok()?;
        output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).ok()?;
        reader.SetCurrentMediaType(video(), None, &output).ok()?;
        let accepted = reader.GetCurrentMediaType(video()).ok()?;
        let subtype = accepted.GetGUID(&MF_MT_SUBTYPE).ok()?;
        (subtype == MFVideoFormat_NV12).then_some(accepted)
    }
}

/// The row stride the decoder states for its output picture.
///
/// Media Foundation allows a negative stride for bottom-up pictures. The
/// magnitude is the distance between rows; the sign only says which way they
/// lie in memory, and `Lock2D` reports that again when the buffer is locked.
fn default_stride(media_type: &IMFMediaType) -> Option<usize> {
    // SAFETY: the key is a constant and the value is a signed byte count.
    unsafe { media_type.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
        .ok()
        .map(|stride| (stride as i32).unsigned_abs() as usize)
}

/// Switches the video stream to RGB32 and reads the type the decoder accepted.
fn rgb32(reader: &IMFSourceReader) -> Result<IMFMediaType, VideoError> {
    // SAFETY: the media type is created here, filled with constant attribute
    // keys, and handed to the reader, which copies it.
    unsafe {
        let output = MFCreateMediaType().map_err(|error| {
            VideoError::Source(format!("cannot create an output media type: {error}"))
        })?;
        output
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|error| VideoError::Source(format!("cannot ask for video: {error}")))?;
        output
            .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
            .map_err(|error| VideoError::Source(format!("cannot ask for RGB32 output: {error}")))?;
        reader
            .SetCurrentMediaType(video(), None, &output)
            .map_err(|error| {
                VideoError::Decode(format!("cannot switch the decoder to RGB32: {error}"))
            })?;
        reader.GetCurrentMediaType(video()).map_err(|error| {
            VideoError::Decode(format!("the decoder states no output type: {error}"))
        })
    }
}

fn frame_size(media_type: &IMFMediaType) -> Option<(u32, u32)> {
    // SAFETY: the key is a constant and the value is a pair of integers.
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }.ok()?;
    Some(((packed >> 32) as u32, packed as u32))
}

/// Whether the type the decoder accepted carries unpacked luma and chroma.
fn is_nv12(media_type: &IMFMediaType) -> bool {
    // SAFETY: the key is a constant and the value is a media subtype GUID.
    let subtype = unsafe { media_type.GetGUID(&MF_MT_SUBTYPE) };
    subtype == Ok(MFVideoFormat_NV12)
}

/// The picture the caller should see, not the padded surface the decoder writes.
///
/// The display aperture names the visible rectangle when a decoder pads its
/// output. Without one, the frame size is the whole picture. A zero or negative
/// aperture is discarded rather than becoming an out-of-range crop.
fn display_size(media_type: &IMFMediaType) -> Option<(u32, u32)> {
    let mut bytes = [0u8; size_of::<MFVideoArea>()];
    // SAFETY: the blob is copied into a buffer of exactly the type's size, and
    // the struct is read unaligned because a blob carries no alignment guarantee.
    let aperture = unsafe {
        media_type
            .GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut bytes, None)
            .ok()?;
        let area: MFVideoArea = ptr::read_unaligned(bytes.as_ptr().cast::<MFVideoArea>());
        let width = u32::try_from(area.Area.cx).ok()?;
        let height = u32::try_from(area.Area.cy).ok()?;
        (width > 0 && height > 0).then_some((width, height))
    };
    aperture.or_else(|| frame_size(media_type).filter(|&(width, height)| width > 0 && height > 0))
}

fn frame_rate(media_type: &IMFMediaType) -> Option<FrameRate> {
    // SAFETY: the key is a constant and the value is a ratio of integers.
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_RATE) }.ok()?;
    let numerator = (packed >> 32) as u32;
    let denominator = packed as u32;
    (numerator > 0 && denominator > 0).then_some(FrameRate {
        numerator,
        denominator,
    })
}

fn rotation(media_type: &IMFMediaType) -> Rotation {
    // SAFETY: the key is a constant and the value is a count of degrees.
    unsafe { media_type.GetUINT32(&MF_MT_VIDEO_ROTATION) }
        .map(|degrees| Rotation::from_degrees(f64::from(degrees)))
        .unwrap_or_default()
}

fn duration(reader: &IMFSourceReader) -> Option<Duration> {
    // SAFETY: the media source index and the key are the constants the reader
    // defines, and the value is a count of 100-nanosecond ticks.
    let prop = unsafe {
        reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
    }
    .ok()?;
    let ticks = prop_u64(&prop)?;
    Some(Duration::from_nanos(ticks.saturating_mul(100)))
}

fn can_seek(reader: &IMFSourceReader) -> bool {
    // SAFETY: the media source index and the key are the constants the reader
    // defines, and the value is a bit mask.
    let prop = unsafe {
        reader.GetPresentationAttribute(
            MF_SOURCE_READER_MEDIASOURCE.0 as u32,
            &MF_SOURCE_READER_MEDIASOURCE_CHARACTERISTICS,
        )
    }
    .ok();
    prop.and_then(|prop| prop_u32(&prop))
        .is_some_and(|flags| flags & (MFMEDIASOURCE_CAN_SEEK.0 as u32) != 0)
}

fn prop_u64(prop: &PROPVARIANT) -> Option<u64> {
    // SAFETY: `vt` states which arm of the union holds the value, and only that
    // arm is read.
    unsafe {
        let inner = &prop.Anonymous.Anonymous;
        match inner.vt {
            VT_UI8 => Some(inner.Anonymous.uhVal),
            VT_I8 => u64::try_from(inner.Anonymous.hVal).ok(),
            _ => None,
        }
    }
}

fn prop_u32(prop: &PROPVARIANT) -> Option<u32> {
    // SAFETY: `vt` states which arm of the union holds the value, and only that
    // arm is read.
    unsafe {
        let inner = &prop.Anonymous.Anonymous;
        match inner.vt {
            VT_UI4 => Some(inner.Anonymous.ulVal),
            VT_I4 => u32::try_from(inner.Anonymous.lVal).ok(),
            _ => None,
        }
    }
}

/// The seek position as the property value the reader takes.
fn ticks_prop(ticks: i64) -> PROPVARIANT {
    PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_I8,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 { hVal: ticks },
            }),
        },
    }
}

/// The stream index the reader's video selectors stand for.
const fn video() -> u32 {
    MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32
}

/// A source reader on its way to the decode thread.
///
/// Media Foundation objects live in the multithreaded apartment, which both the
/// thread that opens the file and the thread that decodes join. One thread uses
/// the reader at a time, so the move between them is the crossing that
/// apartment allows. The `windows` crate marks COM pointers as thread-bound
/// because it cannot know which apartment a caller joined.
struct SendReader(IMFSourceReader);

// SAFETY: the reader is created in the multithreaded apartment, only one thread
// touches it at a time, and the thread that drops it is that same thread.
unsafe impl Send for SendReader {}

struct Reader {
    reader: Option<SendReader>,
    info: VideoInfo,
    can_seek: bool,
    origin: Option<Duration>,
    layout: Layout,
    scratch: Vec<u8>,
    #[cfg(feature = "native-audio")]
    audio: Option<mf_audio::Audio>,
    /// True until audio has been set up at least once on the decode thread.
    #[cfg(feature = "native-audio")]
    audio_pending: bool,
    /// Whether the source itself owns an audio stream this decoder must keep
    /// trying to render. Distinguishes a busy device from a silent clip.
    #[cfg(feature = "native-audio")]
    source_has_audio: bool,
    /// Once a source has been seen to carry audio this stays true, even if a
    /// later device swap makes the reader refuse the stream for a moment. A
    /// transient device failure must never disable recovery forever.
    #[cfg(feature = "native-audio")]
    audio_ever_present: bool,
    /// Whether the player asked for sound. A device built after this point has
    /// to inherit it, or a recovery leaves a live stream that never plays.
    #[cfg(feature = "native-audio")]
    playing: bool,
    /// The playback rate the rebuilt device has to resume at.
    #[cfg(feature = "native-audio")]
    speed: f32,
    /// Whether the player silenced sound.
    #[cfg(feature = "native-audio")]
    muted: bool,
    /// The level the player asked for.
    #[cfg(feature = "native-audio")]
    volume: f32,
    /// The player's recovery handle, given before the decoder moves to the
    /// decode thread. Used to report a deferred setup failure so the worker
    /// schedules a retry instead of leaving a silent channel forever.
    #[cfg(all(
        feature = "native-audio",
        not(target_os = "linux"),
        not(target_arch = "wasm32")
    ))]
    session: Option<crate::PlaybackSession>,
}

impl Reader {
    /// Sets up audio on the decode thread, once on first read and again whenever
    /// the worker's recovery loop asks.
    ///
    /// Nothing here touches the UI thread or fails the picture. A source with
    /// no audio is recorded once and never retried; a busy output device leaves
    /// `audio` absent and reports into the session so the worker schedules the
    /// next attempt on its cooldown.
    #[cfg(feature = "native-audio")]
    fn setup_audio(&mut self, report_failure: bool) -> bool {
        let Some(reader) = self.reader.as_ref() else {
            return false;
        };
        let setup = mf_audio::Audio::setup(&reader.0);
        let built = setup.audio.is_some();
        // Once the source has been seen to carry audio, a device swap that makes
        // the reader refuse it for a moment is not proof the source went silent.
        // Keep retrying: only a source never seen with audio is left alone.
        self.audio_ever_present |= setup.source_has_audio;
        self.source_has_audio = self.audio_ever_present;
        self.audio_pending = false;
        let (playing, speed, muted, volume) = (self.playing, self.speed, self.muted, self.volume);
        if let Some(audio) = setup.audio {
            // Swap in a stream that actually opened. On failure keep the previous
            // one instead of clearing it: dropping the live stream is what made a
            // headset switch go silent on both the old and the new device.
            // Retire the old endpoint off this thread, since tearing a WASAPI
            // client down can block and this thread paces the picture.
            if let Some(old) = self.audio.replace(audio) {
                let _ = std::thread::Builder::new()
                    .name("cranpose-video-retire".to_owned())
                    .spawn(move || drop(old));
            }
            // A rebuilt device starts paused and silent. Re-apply what the player
            // asked for so recovery brings sound back at the picture instead of
            // leaving a stream that was built and never started.
            if let Some(audio) = self.audio.as_mut() {
                audio.set_muted(muted);
                audio.set_volume(volume);
                audio.set_speed(speed);
                if playing {
                    audio.play();
                } else {
                    audio.pause();
                }
            }
        }
        if !built && self.source_has_audio {
            crate::diag(
                "WARN",
                "mf_audio.setup.deferred",
                "audio device unavailable; recovery will retry on the decode thread",
            );
            // Only the caller that owns the retry schedule may record a failure.
            // The worker already does this once when `recover()` returns false;
            // recording it here too doubled the count every cycle and drove the
            // cooldown to its 4 s ceiling after just two attempts. The user then
            // heard nothing for minutes even after the device came back.
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            if report_failure {
                if let Some(session) = &self.session {
                    session.failed("audio.setup", "audio device unavailable");
                }
            }
            let _ = report_failure;
        }
        built
    }

    /// Re-reads the output type after Media Foundation reports a stream change.
    ///
    /// The picture size the player publishes follows the new display aperture,
    /// while the padded surface, crop and colour come from the new type. A
    /// stream that changes to an odd NV12 size moves to RGB32, which has no
    /// chroma pairs to align. The origin stays fixed so timestamps keep their
    /// place in the media.
    fn refresh_layout(&mut self, reader: &IMFSourceReader) -> Result<(), VideoError> {
        let mut current = unsafe { reader.GetCurrentMediaType(video()) }.map_err(|error| {
            VideoError::Decode(format!("the decoder states no new output type: {error}"))
        })?;
        let mut display = display_size(&current)
            .map(|(width, height)| (width as usize, height as usize))
            .or_else(|| Some((self.info.width as usize, self.info.height as usize)));
        if is_nv12(&current)
            && display.is_some_and(|(width, height)| width % 2 != 0 || height % 2 != 0)
        {
            current = rgb32(reader)?;
            display = display_size(&current)
                .map(|(width, height)| (width as usize, height as usize))
                .or(display);
        }
        let (width, height) = display.expect("a display size was kept");
        self.layout = layout_from(&current, (width, height));
        self.info.width = u32::try_from(width).unwrap_or(u32::MAX);
        self.info.height = u32::try_from(height).unwrap_or(u32::MAX);
        Ok(())
    }

    /// Reads the next video sample, skipping the ticks that carry no picture.
    ///
    /// The time it returns is the picture's position in the media. The video
    /// processor delays the stream by the frames it holds for deinterlacing and
    /// states every time shifted by that delay, so the first picture defines the
    /// origin every later one counts from.
    fn next_sample(&mut self) -> Result<Option<(IMFSample, Duration)>, VideoError> {
        THREAD.with(|_| ());
        // Set up audio here, on the decode thread, the first time we read. The
        // same thread then pumps PCM a little ahead of every picture, from the
        // one source reader that owns both tracks.
        #[cfg(feature = "native-audio")]
        {
            if self.audio_pending {
                // This is the first setup, outside the worker's recovery loop,
                // so this is the only place that can arm the first retry.
                self.setup_audio(true);
            }
            if let (Some(audio), Some(reader)) = (self.audio.as_mut(), self.reader.as_ref()) {
                audio.pump(&reader.0);
            }
        }
        // A COM reference of its own keeps the reader alive while this method
        // updates `self.layout`: borrowing `self.reader` across that write would
        // keep the mutable borrow of `self` alive for the whole loop.
        let Some(reader) = self.reader.as_ref().map(|reader| reader.0.clone()) else {
            return Ok(None);
        };
        let mut stream_changed = false;
        loop {
            let mut flags = 0u32;
            let mut timestamp = 0i64;
            let mut sample: Option<IMFSample> = None;
            // SAFETY: the out pointers refer to locals the call fills, and the
            // sample it produces is released when the loop moves on.
            let read = unsafe {
                reader.ReadSample(
                    video(),
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )
            };
            match read {
                Ok(()) => {}
                Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE && !stream_changed => {
                    // The decoder cannot produce another sample until its new
                    // output type has been read. Refresh once, then retry.
                    self.refresh_layout(&reader)?;
                    stream_changed = true;
                    continue;
                }
                Err(error) => {
                    return Err(VideoError::Decode(format!("cannot read a sample: {error}")));
                }
            }

            if flags & (MF_SOURCE_READERF_ERROR.0 as u32) != 0 {
                return Err(VideoError::Decode(
                    "the source reader stopped on an error".to_owned(),
                ));
            }
            if flags & (MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32) != 0 {
                self.refresh_layout(&reader)?;
            }
            if flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
                return Ok(None);
            }
            if flags & (MF_SOURCE_READERF_STREAMTICK.0 as u32) != 0 {
                continue;
            }
            let Some(sample) = sample else {
                continue;
            };

            let ticks = unsafe { sample.GetSampleTime() }.ok().unwrap_or(timestamp);
            let stated =
                Duration::from_nanos(u64::try_from(ticks).unwrap_or(0).saturating_mul(100));
            let origin = *self.origin.get_or_insert(stated);
            return Ok(Some((sample, stated.saturating_sub(origin))));
        }
    }
}

impl VideoDecoder for Reader {
    fn info(&self) -> VideoInfo {
        self.info
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        let Some(reader) = self.reader.as_ref() else {
            return Err(VideoError::Closed);
        };
        if !self.can_seek {
            return Err(VideoError::UnsupportedOperation {
                operation: "seeking",
            });
        }
        let ticks = i64::try_from(position.as_nanos() / 100).unwrap_or(i64::MAX);
        let prop = ticks_prop(ticks);
        // SAFETY: the time format is the all-zero GUID, which states that the
        // position is in the media's own time, and the value is 100-nanosecond
        // ticks, the type its `vt` names.
        unsafe { reader.0.SetCurrentPosition(&GUID::zeroed(), &prop) }
            .map_err(|error| VideoError::Source(format!("cannot seek: {error}")))?;
        #[cfg(feature = "native-audio")]
        if let Some(audio) = self.audio.as_mut() {
            audio.seek(position);
        }
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        let Some((sample, pts)) = self.next_sample()? else {
            return Ok(None);
        };
        let width = self.info.width as usize;
        let height = self.info.height as usize;
        let layout = self.layout;

        // An RGB-only caller can reach a decoder that accepted NV12: the audio
        // wrapper forwards `read` but not `output_format`. The planes are still
        // NV12, so copy and convert them here instead of reading them as RGB.
        if layout.format == PixelFormat::Nv12 {
            let plane_len = plane_len(width, height).ok_or_else(|| {
                VideoError::Decode("the picture size overflows a buffer".to_owned())
            })?;
            let needed = row_bytes(PixelFormat::Rgba8, width)
                .checked_mul(height)
                .ok_or_else(|| {
                    VideoError::Decode("the picture size overflows a buffer".to_owned())
                })?;
            if self.scratch.len() != plane_len {
                self.scratch.resize(plane_len, 0);
            }
            if rgba.len() != needed {
                rgba.resize(needed, 0);
            }
            // SAFETY: the sample owns the buffer for as long as the copy runs,
            // and `copy_nv12` releases the lock it takes on every path.
            let buffer = unsafe { sample.GetBufferByIndex(0) }.map_err(|error| {
                VideoError::Decode(format!("cannot reach the decoded picture: {error}"))
            })?;
            copy_nv12(&buffer, layout, width, height, &mut self.scratch)?;
            nv12_to_rgba(&self.scratch, width, height, layout.color, rgba);
            return Ok(Some(DecodedFrame {
                pts,
                width: width as u32,
                height: height as u32,
            }));
        }

        let row = row_bytes(PixelFormat::Rgba8, width);
        let needed = row * height;
        if rgba.len() != needed {
            rgba.resize(needed, 0);
        }
        // SAFETY: the sample owns its buffers for as long as the copy runs, and
        // the copy releases the lock it takes on every path.
        let buffer = unsafe { sample.ConvertToContiguousBuffer() }.map_err(|error| {
            VideoError::Decode(format!("cannot reach the decoded picture: {error}"))
        })?;
        copy(&buffer, row, height, rgba)?;
        Ok(Some(DecodedFrame {
            pts,
            width: width as u32,
            height: height as u32,
        }))
    }

    fn read_nv12(&mut self, nv12: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        let Some((sample, pts)) = self.next_sample()? else {
            return Ok(None);
        };
        if self.layout.format != PixelFormat::Nv12 {
            return Err(VideoError::Decode(
                "the decoder does not produce NV12 planes".to_owned(),
            ));
        }
        let width = self.info.width as usize;
        let height = self.info.height as usize;
        let layout = self.layout;
        let needed = plane_len(width, height)
            .ok_or_else(|| VideoError::Decode("the picture size overflows a buffer".to_owned()))?;
        if nv12.len() != needed {
            nv12.resize(needed, 0);
        }
        // The planes are read where the decoder left them, so a picture is
        // copied once instead of being flattened and copied again.
        // SAFETY: the sample owns the buffer for as long as the copy runs.
        let buffer = unsafe { sample.GetBufferByIndex(0) }.map_err(|error| {
            VideoError::Decode(format!("cannot reach the decoded picture: {error}"))
        })?;
        copy_nv12(&buffer, layout, width, height, nv12)?;
        Ok(Some(DecodedFrame {
            pts,
            width: width as u32,
            height: height as u32,
        }))
    }

    fn output_format(&self) -> PixelFormat {
        self.layout.format
    }

    fn nv12_color(&self) -> YuvColor {
        self.layout.color
    }

    #[cfg(feature = "native-audio")]
    fn play(&mut self) {
        self.playing = true;
        if let Some(audio) = self.audio.as_mut() {
            audio.play();
        }
    }

    #[cfg(feature = "native-audio")]
    fn pause(&mut self) {
        self.playing = false;
        if let Some(audio) = self.audio.as_mut() {
            audio.pause();
        }
    }

    #[cfg(feature = "native-audio")]
    fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
        if let Some(audio) = self.audio.as_mut() {
            audio.set_speed(speed);
        }
    }

    #[cfg(feature = "native-audio")]
    fn sync_clock(&mut self, position: Duration) {
        if let Some(audio) = self.audio.as_mut() {
            audio.sync_to(position);
        }
    }

    #[cfg(feature = "native-audio")]
    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(audio) = self.audio.as_mut() {
            audio.set_muted(muted);
        }
    }

    #[cfg(feature = "native-audio")]
    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if let Some(audio) = self.audio.as_mut() {
            audio.set_volume(volume);
        }
    }

    #[cfg(feature = "native-audio")]
    fn carries_audio(&self) -> bool {
        // One source reader owns both tracks from the moment it opens, even
        // before deferred setup has produced an output stream. Step the paired
        // audio wrapper aside so it cannot open the same source a second time
        // and play duplicate sound. A silent clip is harmless: every audio
        // call on this decoder is then a no-op.
        self.audio.is_some() || self.audio_pending || self.source_has_audio
    }

    #[cfg(all(
        feature = "native-audio",
        not(target_os = "linux"),
        not(target_arch = "wasm32")
    ))]
    fn attach_session(&mut self, session: crate::PlaybackSession) {
        self.session = Some(session);
    }

    #[cfg(all(
        feature = "native-audio",
        not(target_os = "linux"),
        not(target_arch = "wasm32")
    ))]
    fn recover(&mut self) -> bool {
        // A silent source needs no recovery. Otherwise try the whole setup
        // again on this decode thread; returning false keeps the worker's retry
        // scheduled while the device is still unavailable.
        if !self.source_has_audio {
            return true;
        }
        // Report whether a fresh stream was actually built. A failed attempt
        // keeps the previous stream alive (so sound can resume on the old device
        // or through the route rebind) but must not tell the worker that recovery
        // succeeded — otherwise retries stop while both devices are still silent.
        // The worker records the failed attempt once, so do not report it here
        // as well; two reports per cycle hit the 4 s backoff ceiling immediately.
        self.setup_audio(false)
    }

    fn close(&mut self) {
        // Release the device before the reader: HLS keeps its temporary file
        // open until the reader drops, and Windows will not delete a file whose
        // audio stream still holds a handle.
        #[cfg(feature = "native-audio")]
        {
            self.audio = None;
        }
        self.reader = None;
    }
}

/// Meaningful bytes in one picture row, before any decoder padding.
fn row_bytes(format: PixelFormat, width: usize) -> usize {
    match format {
        PixelFormat::Nv12 => width,
        PixelFormat::Rgba8 => width * 4,
    }
}

/// Copies a decoded picture into top-down RGBA8 rows.
fn copy(
    buffer: &IMFMediaBuffer,
    row: usize,
    height: usize,
    rgba: &mut [u8],
) -> Result<(), VideoError> {
    // SAFETY: the lock hands out the decoder's pixels for the length of the
    // copy, and it is released on every path.
    unsafe {
        if let Ok(buffer2d) = buffer.cast::<IMF2DBuffer>() {
            let mut scanline: *mut u8 = ptr::null_mut();
            let mut pitch = 0i32;
            buffer2d
                .Lock2D(&mut scanline, &mut pitch)
                .map_err(|error| {
                    VideoError::Decode(format!("cannot lock the decoded picture: {error}"))
                })?;
            let step = pitch as isize;
            let stride = step.unsigned_abs();
            if scanline.is_null() || stride < row {
                let _ = buffer2d.Unlock2D();
                return Err(VideoError::Decode(format!(
                    "the decoder states a stride ({stride}) narrower than a row ({row})"
                )));
            }
            // The 2-D branch trusts `Lock2D` for the pitch, but the pitch alone
            // does not prove the buffer is long enough: a decoder can report a
            // wide surface and hand over a short one. The NV12 branch below
            // already measures it; do the same before reading any RGBA row.
            let available = buffer.GetCurrentLength().map_err(|error| {
                let _ = buffer2d.Unlock2D();
                VideoError::Decode(format!("cannot measure the decoded picture: {error}"))
            })? as usize;
            if let Err(error) = rgba_fits(stride, height, available) {
                let _ = buffer2d.Unlock2D();
                return Err(error);
            }
            // A negative pitch means the rows lie the other way in memory, and
            // the first scanline still points at the top row.
            for y in 0..height {
                let source = slice::from_raw_parts(scanline.offset(y as isize * step), row);
                bgra_to_rgba(source, &mut rgba[y * row..][..row]);
            }
            let _ = buffer2d.Unlock2D();
            return Ok(());
        }

        let mut data: *mut u8 = ptr::null_mut();
        let mut length = 0u32;
        buffer
            .Lock(&mut data, None, Some(&mut length))
            .map_err(|error| {
                VideoError::Decode(format!("cannot lock the decoded picture: {error}"))
            })?;
        if data.is_null() || (length as usize) < row * height {
            let _ = buffer.Unlock();
            return Err(VideoError::Decode(
                "the decoder handed over a picture shorter than its size".to_owned(),
            ));
        }
        for y in 0..height {
            let source = slice::from_raw_parts(data.add(y * row), row);
            bgra_to_rgba(source, &mut rgba[y * row..][..row]);
        }
        let _ = buffer.Unlock();
    }
    Ok(())
}

/// Copies a decoded NV12 surface into two unpadded planes.
///
/// Nothing about the media type is trusted on its own. The pitch reported by
/// `Lock2D` is authoritative for the rows actually in memory; the declared
/// stride, coded size and display crop decide where the visible picture lies.
/// Every row's extent is checked against the buffer length before it is read.
fn copy_nv12(
    buffer: &IMFMediaBuffer,
    layout: Layout,
    width: usize,
    height: usize,
    nv12: &mut [u8],
) -> Result<(), VideoError> {
    let row = row_bytes(PixelFormat::Nv12, width);
    let plane = plane_len(width, height)
        .ok_or_else(|| VideoError::Decode("the picture size overflows a buffer".to_owned()))?;
    let (crop_x, crop_y) = layout.crop;
    let (coded_width, coded_height) = layout.coded;
    let stride_error = |pitch: usize| {
        VideoError::Decode(format!(
            "the decoder states a stride ({pitch}) narrower than a row ({row})"
        ))
    };
    let geometry_error = || {
        VideoError::Decode(format!(
            "the decoder's crop ({crop_x},{crop_y}) does not fit a {coded_width}x{coded_height} surface"
        ))
    };
    if nv12.len() < plane {
        return Err(VideoError::Decode(
            "the caller's NV12 buffer is shorter than its size".to_owned(),
        ));
    }

    // SAFETY: the lock hands out the decoder's pixels for the length of the
    // copy, and it is released on every path.
    unsafe {
        if let Ok(buffer2d) = buffer.cast::<IMF2DBuffer>() {
            let mut scanline: *mut u8 = ptr::null_mut();
            let mut pitch = 0i32;
            buffer2d
                .Lock2D(&mut scanline, &mut pitch)
                .map_err(|error| {
                    VideoError::Decode(format!("cannot lock the decoded picture: {error}"))
                })?;
            let step = pitch as isize;
            let stride = step.unsigned_abs();
            let copy = (|| -> Result<(), VideoError> {
                if scanline.is_null() {
                    return Err(VideoError::Decode(
                        "the decoder handed over no picture".to_owned(),
                    ));
                }
                if stride < row {
                    return Err(stride_error(stride));
                }
                if !nv12_fits(layout.coded, layout.crop, row, height, stride) {
                    return Err(geometry_error());
                }
                let needed = plane_bytes(stride, coded_height)
                    .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))?;
                let available = buffer.GetCurrentLength().map_err(|error| {
                    VideoError::Decode(format!("cannot measure the decoded picture: {error}"))
                })? as usize;
                if available < needed {
                    return Err(VideoError::Decode(
                        "the decoder handed over a picture shorter than its size".to_owned(),
                    ));
                }

                // A bottom-up surface points at its last luma row, so valid
                // offsets can be negative. Check both ends of every plane.
                // The top row starts at `crop_x`, so its last byte is at
                // `crop_x + row`: a horizontal crop must widen the upper edge,
                // exactly as `nv12_fits` already requires of the pitch. Using
                // just `row` here rejected a cropped bottom-up picture whose
                // bytes were entirely inside the surface.
                let (lower, upper) = surface_bounds(step, needed, crop_x, row)?;
                let luma = row_offset(step, crop_y, crop_x)?;
                let chroma = row_offset(step, coded_height + crop_y / 2, crop_x / 2)?;
                let luma_span = row_span(luma, step, height, row)
                    .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))?;
                let chroma_span = row_span(chroma, step, height / 2, row)
                    .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))?;
                for span in [luma_span, chroma_span] {
                    if span.0 < lower || span.1 > upper {
                        return Err(geometry_error());
                    }
                }

                let chroma_plane = width * height;
                for y in 0..height {
                    let source =
                        slice::from_raw_parts(scanline.offset(luma).offset(y as isize * step), row);
                    nv12[y * row..][..row].copy_from_slice(source);
                }
                for y in 0..height / 2 {
                    let source = slice::from_raw_parts(
                        scanline.offset(chroma).offset(y as isize * step),
                        row,
                    );
                    nv12[chroma_plane + y * row..][..row].copy_from_slice(source);
                }
                Ok(())
            })();
            let _ = buffer2d.Unlock2D();
            return copy;
        }

        // A buffer without `IMF2DBuffer` is contiguous, so the media type's
        // stride is the only pitch available. It still has to describe a
        // surface that contains the cropped picture.
        let stride = layout.stride;
        if !nv12_fits(layout.coded, layout.crop, row, height, stride) {
            return Err(geometry_error());
        }
        let needed = plane_bytes(stride, coded_height)
            .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))?;
        let mut data: *mut u8 = ptr::null_mut();
        let mut length = 0u32;
        buffer
            .Lock(&mut data, None, Some(&mut length))
            .map_err(|error| {
                VideoError::Decode(format!("cannot lock the decoded picture: {error}"))
            })?;
        let copy = (|| -> Result<(), VideoError> {
            if data.is_null() {
                return Err(VideoError::Decode(
                    "the decoder handed over no picture".to_owned(),
                ));
            }
            if stride < row {
                return Err(stride_error(stride));
            }
            if (length as usize) < needed {
                return Err(VideoError::Decode(
                    "the decoder handed over a picture shorter than its size".to_owned(),
                ));
            }
            let luma = row_offset(stride as isize, crop_y, crop_x)?;
            let chroma = row_offset(stride as isize, coded_height + crop_y / 2, crop_x / 2)?;
            let bound = isize::try_from(needed)
                .map_err(|_| VideoError::Decode("the surface size overflows".to_owned()))?;
            for span in [
                row_span(luma, stride as isize, height, row)
                    .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))?,
                row_span(chroma, stride as isize, height / 2, row)
                    .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))?,
            ] {
                if span.0 < 0 || span.1 > bound {
                    return Err(geometry_error());
                }
            }
            let chroma_plane = width * height;
            for y in 0..height {
                let source =
                    slice::from_raw_parts(data.offset(luma + y as isize * stride as isize), row);
                nv12[y * row..][..row].copy_from_slice(source);
            }
            for y in 0..height / 2 {
                let source =
                    slice::from_raw_parts(data.offset(chroma + y as isize * stride as isize), row);
                nv12[chroma_plane + y * row..][..row].copy_from_slice(source);
            }
            Ok(())
        })();
        let _ = buffer.Unlock();
        copy
    }
}

/// The byte offset of a row in a pitched surface, with its crop applied.
fn row_offset(step: isize, row: usize, column: usize) -> Result<isize, VideoError> {
    let rows = isize::try_from(row)
        .ok()
        .and_then(|row| row.checked_mul(step))
        .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))?;
    let column = isize::try_from(column)
        .ok()
        .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))?;
    rows.checked_add(column)
        .ok_or_else(|| VideoError::Decode("the surface size overflows".to_owned()))
}

/// Starts Media Foundation once for the whole process.
///
/// The library stays started: a player that opens and closes clips would
/// otherwise pay its start-up on every open, and no caller can know when the
/// last player is gone.
fn start() -> Result<(), VideoError> {
    static START: OnceLock<Result<(), String>> = OnceLock::new();
    let started = START.get_or_init(|| {
        // SAFETY: both arguments are plain integers, and the call has no
        // pointer arguments to keep valid.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) }
            .map_err(|error| format!("Media Foundation did not start: {error}"))
    });
    match started {
        Ok(()) => Ok(()),
        Err(reason) => Err(VideoError::Source(reason.clone())),
    }
}

/// The COM apartment a thread joined, left when that thread ends.
struct Apartment {
    joined: bool,
}

impl Apartment {
    fn join() -> Self {
        // SAFETY: the reserved pointer is absent, as the call requires, and the
        // apartment it joins belongs to this thread.
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // `RPC_E_CHANGED_MODE` means the host already put this thread in an
        // apartment of its own choosing. The thread is initialized either way,
        // and only a call this thread made may be undone here.
        Self {
            joined: result.is_ok() && result != RPC_E_CHANGED_MODE,
        }
    }

    /// Joins the caller's COM apartment for the life of the process.
    ///
    /// `open` calls this instead of [`Apartment::join`] because the WASAPI
    /// device enumerator cpal caches process-wide is created inside this
    /// apartment on the first audio lookup; it must never be uninitialized.
    /// A leaked guard is intentional and mirrors the never-shut-down
    /// `MFStartup`: both own process-lifetime COM state. If the host already
    /// initialized COM on this thread, `CoInitializeEx` returns
    /// `RPC_E_CHANGED_MODE` and there is nothing to keep alive.
    fn join_for_process() {
        let joined = Self::join();
        if joined.joined {
            std::mem::forget(joined);
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.joined {
            // SAFETY: this balances the call `join` made on this same thread.
            unsafe { CoUninitialize() };
        }
    }
}

thread_local! {
    // The decode thread joins the apartment when it reads its first sample: the
    // thread that opened the file is not the one that decodes.
    static THREAD: Apartment = Apartment::join();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `odd854.mp4` selects NV12: its luma row is 854 bytes, not 854 * 4.
    /// Assuming an RGB row made `read` demand 3416 bytes from a buffer whose
    /// real pitch is 896, which is the exact reported failure.
    #[test]
    fn nv12_row_is_one_byte_per_pixel() {
        assert_eq!(row_bytes(PixelFormat::Nv12, 854), 854);
        assert_eq!(row_bytes(PixelFormat::Rgba8, 854), 3416);
    }

    #[test]
    fn nv12_geometry_accepts_a_padded_surface() {
        // The changed type for `odd854.mp4` reports 864x480, while `Lock2D`
        // reports a pitch of 896. The 854-byte display row still fits.
        assert!(nv12_fits((864, 480), (5, 0), 854, 480, 896));
        assert_eq!(plane_bytes(896, 480), Some(896 * 480 * 3 / 2));
    }

    #[test]
    fn nv12_geometry_rejects_bad_crops_and_planes() {
        assert!(nv12_fits((864, 480), (1, 0), 854, 480, 896)); // odd x uses chroma x / 2
        assert!(!nv12_fits((864, 480), (0, 1), 854, 480, 896)); // odd y straddles chroma rows
        assert!(!nv12_fits((864, 480), (12, 0), 854, 480, 896)); // outside width
        assert!(!nv12_fits((864, 480), (0, 2), 854, 480, 896)); // outside height
        assert!(!nv12_fits((854, 481), (0, 0), 854, 480, 854)); // odd coded height
        assert!(!nv12_fits((854, 480), (0, 0), 854, 480, 853)); // pitch < row
    }

    #[test]
    fn nv12_picture_byte_counts() {
        assert_eq!(plane_len(854, 480), Some(854 * 480 * 3 / 2));
        assert_eq!(plane_len(usize::MAX, 2), None);
    }

    #[test]
    fn row_span_follows_a_negative_pitch() {
        assert_eq!(row_span(0, -896, 480, 854), Some((-896 * 479, 854)));
    }

    #[test]
    fn bottom_up_bounds_include_a_horizontal_crop() {
        // A 896-byte pitch with a 5-byte crop and an 854-byte row must accept a
        // last visible byte at 859. The old upper bound of 854 rejected it.
        let needed = plane_bytes(896, 480).unwrap();
        assert_eq!(
            surface_bounds(-896, needed, 5, 854).unwrap(),
            (-(needed as isize), 859)
        );
        assert_eq!(
            surface_bounds(896, needed, 5, 854).unwrap(),
            (0, needed as isize)
        );
    }

    #[test]
    fn rgba_length_check_rejects_a_short_pitched_surface() {
        assert!(rgba_fits(896, 480, 896 * 480).is_ok());
        assert!(rgba_fits(896, 480, 896 * 480 - 1).is_err());
        assert!(rgba_fits(usize::MAX, 2, usize::MAX).is_err());
    }
}
