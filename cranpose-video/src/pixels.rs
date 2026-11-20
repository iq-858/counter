//! Pixel conversion.
//!
//! A platform decoder hands out its picture in whatever layout its hardware and
//! framework choose. Windows and Android decode to NV12 natively; AVFoundation
//! hands out BGRA. The screen wants RGBA8, so something has to convert, and at
//! 1080p60 that is 500 MB/s of writes: where the conversion runs decides
//! whether playback stutters.
//!
//! Two rules keep it off the frame's critical path:
//!
//! * every kernel writes into a caller-owned buffer, so no frame allocates;
//! * the 4:2:0 kernel splits its rows across the machine's cores, because the
//!   rows of one picture are independent by construction.
//!
//! The integer coefficients are Q16 fixed point, so the result is bit-identical
//! on every target and needs no floating point in the loop.

use std::{ops::Range, sync::OnceLock, thread};

use crate::frame::{ColorMatrix, ColorRange, YuvColor};

/// The smallest picture worth splitting across threads.
///
/// Measured on this 4-logical-core laptop with the optimized build:
/// 720p takes 1.54 ms on one core and 1.34 ms on four (1.15x);
/// 1080p takes 2.85 ms on one core and 2.93 ms on four (0.97x, slower);
/// 4K takes 10.82 ms on one core and 7.90 ms on four (1.37x).
///
/// The split only pays for itself at 4K. Below that it costs three extra
/// thread spawns on every picture and up to three cores of CPU for no wall-time
/// gain — CPU the decoder and the audio callback need. `CRANPOSE_VIDEO_WORKERS`
/// can still pin a different count for a diagnostic run.
const PARALLEL_MIN_PIXELS: usize = 3840 * 2160;

/// The most workers one conversion uses. Past this the memory bandwidth of the
/// machine is the limit, not the cores.
const MAX_WORKERS: usize = 8;

/// The environment variable that pins the converter to a given worker count.
///
/// It exists so a diagnostic build can compare one worker against all of them
/// on the machine it runs on; a normal build never sets it.
const WORKERS_ENV: &str = "CRANPOSE_VIDEO_WORKERS";

/// Turns a row of 32-bit BGRA pixels into RGBA8.
///
/// The alpha byte is set to full, because both platforms that hand out BGRA
/// state it as unused: RGB32 on Windows has no alpha of its own, and a video
/// track is shown opaque.
pub(crate) fn bgra_to_rgba(source: &[u8], dest: &mut [u8]) {
    for (pixel, out) in source
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dest.as_chunks_mut::<4>().0.iter_mut())
    {
        out[0] = pixel[2];
        out[1] = pixel[1];
        out[2] = pixel[0];
        out[3] = u8::MAX;
    }
}

/// One colour space as Q16 fixed-point gains from luma and chroma samples.
#[derive(Clone, Copy)]
struct Coefficients {
    /// Gain on luma once the range offset is removed.
    luma: i32,
    /// Gain on V for red.
    red_v: i32,
    /// Gain on U subtracted from green.
    green_u: i32,
    /// Gain on V subtracted from green.
    green_v: i32,
    /// Gain on U for blue.
    blue_u: i32,
    /// What a limited-range luma sample of black reads, 16 or 0.
    luma_bias: i32,
}

impl Coefficients {
    /// The gains for one matrix in one range.
    ///
    /// The limited-range figures are the studio-swing equations every
    /// broadcast codec uses; the full-range ones are the same matrices with
    /// luma spanning zero to 255. Both are multiplied by 65536, the shift the
    /// kernel applies after summing.
    const fn of(color: YuvColor) -> Self {
        const fn table(matrix: ColorMatrix, range: ColorRange) -> Coefficients {
            // (luma, red_v, green_u, green_v, blue_u)
            let (luma, red_v, green_u, green_v, blue_u) = match (matrix, range) {
                (ColorMatrix::Bt601, ColorRange::Limited) => (76284, 104595, 25690, 53281, 132185),
                (ColorMatrix::Bt601, ColorRange::Full) => (65536, 91881, 22554, 46802, 116134),
                (ColorMatrix::Bt709, ColorRange::Limited) => (76284, 117504, 13959, 34931, 138412),
                (ColorMatrix::Bt709, ColorRange::Full) => (65536, 103206, 12255, 30671, 121610),
                (ColorMatrix::Bt2020, ColorRange::Limited) => (76284, 109970, 12255, 39190, 123276),
                (ColorMatrix::Bt2020, ColorRange::Full) => (65536, 96638, 10788, 34523, 123305),
            };
            Coefficients {
                luma,
                red_v,
                green_u,
                green_v,
                blue_u,
                luma_bias: match range {
                    ColorRange::Limited => 16,
                    ColorRange::Full => 0,
                },
            }
        }
        table(color.matrix(), color.range())
    }
}

/// The same gains as [`Coefficients`], indexed by sample value.
///
/// A sample is one byte, so a 256-entry table removes five integer multiplies
/// per pixel without changing a single result. The tables are shared by every
/// worker and every frame; at eight workers they stay hot in each core's cache.
#[derive(Clone, Copy)]
struct Tables {
    luma: [i32; 256],
    red_v: [i32; 256],
    green_u: [i32; 256],
    green_v: [i32; 256],
    blue_u: [i32; 256],
}

impl Tables {
    const fn of(color: YuvColor) -> Self {
        let c = Coefficients::of(color);
        let mut tables = Self {
            luma: [0; 256],
            red_v: [0; 256],
            green_u: [0; 256],
            green_v: [0; 256],
            blue_u: [0; 256],
        };
        let mut sample = 0usize;
        while sample < 256 {
            let centered = sample as i32 - 128;
            tables.luma[sample] = (sample as i32 - c.luma_bias) * c.luma;
            tables.red_v[sample] = c.red_v * centered;
            tables.green_u[sample] = c.green_u * centered;
            tables.green_v[sample] = c.green_v * centered;
            tables.blue_u[sample] = c.blue_u * centered;
            sample += 1;
        }
        tables
    }
}

/// The tables for the six colour spaces, built once on first use.
fn tables(color: YuvColor) -> &'static Tables {
    static BT601_LIMITED: OnceLock<Tables> = OnceLock::new();
    static BT601_FULL: OnceLock<Tables> = OnceLock::new();
    static BT709_LIMITED: OnceLock<Tables> = OnceLock::new();
    static BT709_FULL: OnceLock<Tables> = OnceLock::new();
    static BT2020_LIMITED: OnceLock<Tables> = OnceLock::new();
    static BT2020_FULL: OnceLock<Tables> = OnceLock::new();
    let (cell, color) = match (color.matrix(), color.range()) {
        (ColorMatrix::Bt601, ColorRange::Limited) => (&BT601_LIMITED, color),
        (ColorMatrix::Bt601, ColorRange::Full) => (&BT601_FULL, color),
        (ColorMatrix::Bt709, ColorRange::Limited) => (&BT709_LIMITED, color),
        (ColorMatrix::Bt709, ColorRange::Full) => (&BT709_FULL, color),
        (ColorMatrix::Bt2020, ColorRange::Limited) => (&BT2020_LIMITED, color),
        (ColorMatrix::Bt2020, ColorRange::Full) => (&BT2020_FULL, color),
    };
    cell.get_or_init(|| Tables::of(color))
}

/// Converts NV12 into RGBA8, using every core the picture is worth.
///
/// `nv12` holds a full-size luma plane followed by one interleaved chroma
/// plane at half height, both unpadded. `rgba` receives `width * height * 4`
/// bytes in top-down order.
pub(crate) fn nv12_to_rgba(
    nv12: &[u8],
    width: usize,
    height: usize,
    color: YuvColor,
    rgba: &mut [u8],
) {
    debug_assert_eq!(nv12.len(), width * height * 3 / 2);
    debug_assert_eq!(rgba.len(), width * height * 4);

    let tables = tables(color);
    let coefficients = Coefficients::of(color);
    let workers = workers().min(height / 2).max(1);
    if workers == 1 || width * height < PARALLEL_MIN_PIXELS {
        convert_rows_tuned(nv12, width, height, tables, coefficients, rgba, 0..height);
        return;
    }

    // Each worker owns a whole number of rows, so two writers never touch the
    // same cache line and no lock is needed.
    let rows = height / workers;
    let mut rest = rgba;
    let mut first = 0;
    thread::scope(|scope| {
        for _ in 1..workers {
            let (head, tail) = rest.split_at_mut(rows * width * 4);
            let span = first..first + rows;
            scope.spawn(move || {
                convert_rows_tuned(nv12, width, height, tables, coefficients, head, span);
            });
            rest = tail;
            first += rows;
        }
        convert_rows_tuned(
            nv12,
            width,
            height,
            tables,
            coefficients,
            rest,
            first..height,
        );
    });
}

/// Converts `rows` with the fastest kernel this CPU supports.
///
/// The scalar table kernel stays the reference: it is the fallback on every
/// target, and on x86_64 with SSE4.1 and SSSE3 the vector kernel below
/// produces exactly the same bytes. `CRANPOSE_VIDEO_SIMD=0` pins the scalar
/// kernel, so one build can be measured both ways.
fn convert_rows_tuned(
    nv12: &[u8],
    width: usize,
    height: usize,
    tables: &Tables,
    coefficients: Coefficients,
    rgba: &mut [u8],
    rows: Range<usize>,
) {
    #[cfg(target_arch = "x86_64")]
    if vector_rows() {
        // SAFETY: `vector_rows` proved that this CPU implements SSE4.1 and
        // SSSE3, and every row is written once into the range the caller owns.
        unsafe {
            convert_rows_sse41(nv12, width, height, coefficients, rgba, rows);
        }
        return;
    }
    let _ = coefficients;
    convert_rows(nv12, width, height, tables, rgba, rows);
}

/// Whether the vector kernel may run: an x86_64 CPU with SSE4.1 and SSSE3,
/// unless the environment pins the scalar kernel for a comparison.
#[cfg(target_arch = "x86_64")]
fn vector_rows() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let requested = std::env::var("CRANPOSE_VIDEO_SIMD")
            .map(|value| value.trim() != "0")
            .unwrap_or(true);
        requested
            && std::arch::is_x86_feature_detected!("sse4.1")
            && std::arch::is_x86_feature_detected!("ssse3")
    })
}

/// Converts rows with SSE4.1, four pixels at a time.
///
/// The arithmetic is the scalar kernel's Q16 fixed point in the same order, so
/// every output byte is identical: `(sample - bias) * gain`, sum, arithmetic
/// shift, then saturation. Four luma bytes and four chroma bytes become one
/// 16-byte RGBA store; a width that is not a multiple of four finishes with the
/// scalar kernel so the tail is never dropped.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1,ssse3")]
unsafe fn convert_rows_sse41(
    nv12: &[u8],
    width: usize,
    height: usize,
    c: Coefficients,
    rgba: &mut [u8],
    rows: Range<usize>,
) {
    // SAFETY: every load and store below stays inside the slices this caller
    // handed over, and the CPU supports the instructions the attribute enables.
    use std::arch::x86_64::*;

    let chroma_plane = width * height;
    let first = rows.start;
    let groups = width / 4;
    let tail = groups * 4;
    let luma_gain = _mm_set1_epi32(c.luma);
    let luma_bias = _mm_set1_epi32(c.luma_bias);
    let red_v = _mm_set1_epi32(c.red_v);
    let green_u = _mm_set1_epi32(c.green_u);
    let green_v = _mm_set1_epi32(c.green_v);
    let blue_u = _mm_set1_epi32(c.blue_u);
    let centre = _mm_set1_epi32(128);
    let zero = _mm_setzero_si128();
    // Four chroma bytes [u0,v0,u1,v1] become [u0,u0,u1,u1,v0,v0,v1,v1], so the
    // low half widens to one U per pixel and the high half to one V per pixel.
    let split_uv = _mm_setr_epi8(0, 0, 2, 2, 1, 1, 3, 3, -1, -1, -1, -1, -1, -1, -1, -1);
    let alpha = _mm_set1_epi8(-1);

    for y in rows {
        let luma_row = &nv12[y * width..][..width];
        let chroma_row = &nv12[chroma_plane + (y / 2) * width..][..width];
        let out_row = &mut rgba[(y - first) * width * 4..][..width * 4];

        let mut x = 0;
        while x < tail {
            // SAFETY: `x + 4 <= tail <= width`, so the four-byte loads and the
            // sixteen-byte store stay inside this row's luma and output slices.
            let luma_bytes = unsafe {
                _mm_cvtsi32_si128(std::ptr::read_unaligned(
                    luma_row.as_ptr().add(x) as *const i32
                ))
            };
            let luma16 = _mm_unpacklo_epi8(luma_bytes, zero);
            let luma32 = _mm_unpacklo_epi16(luma16, zero);
            let luma = _mm_mullo_epi32(_mm_sub_epi32(luma32, luma_bias), luma_gain);

            // SAFETY: the chroma row has the same width as the luma row, and
            // `x + 4 <= width`.
            let chroma_bytes = unsafe {
                _mm_cvtsi32_si128(std::ptr::read_unaligned(
                    chroma_row.as_ptr().add(x) as *const i32
                ))
            };
            let uv = _mm_shuffle_epi8(chroma_bytes, split_uv);
            // `split_uv` lays the four chroma bytes out as
            // [u0,u0,u1,u1, v0,v0,v1,v1] in the LOW eight bytes; bytes 8..16
            // are the mask's -1 zeroes. Widen that low half once, then take the
            // low four 16-bit lanes as U and the high four as V. Reading V with
            // `unpackhi_epi8(uv, ..)` would read the all-zero upper bytes and
            // make every V equal -128, which silently corrupts green.
            let wide = _mm_unpacklo_epi8(uv, zero);
            let u32v = _mm_unpacklo_epi16(wide, zero);
            let v32v = _mm_unpackhi_epi16(wide, zero);
            let u = _mm_sub_epi32(u32v, centre);
            let v = _mm_sub_epi32(v32v, centre);

            let red = _mm_add_epi32(luma, _mm_mullo_epi32(v, red_v));
            let green = _mm_sub_epi32(
                luma,
                _mm_add_epi32(_mm_mullo_epi32(u, green_u), _mm_mullo_epi32(v, green_v)),
            );
            let blue = _mm_add_epi32(luma, _mm_mullo_epi32(u, blue_u));

            // The scalar kernel clamps `(value >> 16)` to a byte. Shift before
            // packing: an unshifted Q16 sum saturates at i16::MAX in
            // `_mm_packs_epi32` and then at 255 in `_mm_packus_epi16`, which
            // rounds bright-but-not-clipped channels (e.g. 254) up to 255 and
            // breaks bit-exactness with the scalar oracle. After the shift the
            // values already sit in byte range, so the two packs only clamp.
            let red = _mm_srai_epi32(red, 16);
            let green = _mm_srai_epi32(green, 16);
            let blue = _mm_srai_epi32(blue, 16);
            let red = _mm_packus_epi16(_mm_packs_epi32(red, red), zero);
            let green = _mm_packus_epi16(_mm_packs_epi32(green, green), zero);
            let blue = _mm_packus_epi16(_mm_packs_epi32(blue, blue), zero);
            // [r0,r1,r2,r3] and [g0,g1,g2,g3] interleave to 16-bit RG pairs,
            // and [b0..b3] with opaque alpha to 16-bit BA pairs; one more
            // interleave is four finished RGBA pixels.
            let rg = _mm_unpacklo_epi8(red, green);
            let ba = _mm_unpacklo_epi8(blue, alpha);
            let pixels = _mm_unpacklo_epi16(rg, ba);
            // SAFETY: the output row holds `width * 4` bytes and
            // `x * 4 + 16 <= width * 4`.
            unsafe {
                _mm_storeu_si128(out_row.as_mut_ptr().add(x * 4) as *mut __m128i, pixels);
            }
            x += 4;
        }

        // A width that is not a multiple of four, or the last pixels of any
        // row the vector loop could not take. Same arithmetic, one pixel at a
        // time, so the result never depends on which kernel ran.
        while x < width {
            let brightness = (i32::from(luma_row[x]) - c.luma_bias) * c.luma;
            let u = i32::from(chroma_row[x & !1]) - 128;
            let v = i32::from(chroma_row[(x & !1) + 1]) - 128;
            let out = &mut out_row[x * 4..][..4];
            out[0] = clamp(brightness + c.red_v * v);
            out[1] = clamp(brightness - (c.green_u * u + c.green_v * v));
            out[2] = clamp(brightness + c.blue_u * u);
            out[3] = u8::MAX;
            x += 1;
        }
    }
}

/// Converts `rows` of an NV12 picture into the matching rows of an RGBA buffer.
fn convert_rows(
    nv12: &[u8],
    width: usize,
    height: usize,
    tables: &Tables,
    rgba: &mut [u8],
    rows: Range<usize>,
) {
    let Tables {
        luma,
        red_v,
        green_u,
        green_v,
        blue_u,
    } = tables;
    let chroma_plane = width * height;
    // The buffer a worker receives starts at its first row, so the rows it is
    // given are absolute and the writes are relative to that first one.
    let first = rows.start;
    for y in rows {
        let luma_row = &nv12[y * width..][..width];
        // Both rows of a 2x2 block share one chroma row, so the integer
        // division is exact and the slice is always in range.
        let chroma_row = &nv12[chroma_plane + (y / 2) * width..][..width];
        let out_row = &mut rgba[(y - first) * width * 4..][..width * 4];
        let (pairs, leftovers) = luma_row.as_chunks::<2>();
        let (chroma, _) = chroma_row.as_chunks::<2>();
        let (out, tail) = out_row.as_chunks_mut::<8>();
        for ((luma_pair, chroma_pair), pixel_pair) in
            pairs.iter().zip(chroma.iter()).zip(out.iter_mut())
        {
            // Both pixels of the pair share one chroma sample, so the two
            // chroma terms are looked up once and the two luma terms stay
            // independent. The arithmetic order is unchanged from the
            // multiply-based kernel, so every byte stays bit-identical.
            let u = usize::from(chroma_pair[0]);
            let v = usize::from(chroma_pair[1]);
            let red = red_v[v];
            let green = green_u[u] + green_v[v];
            let blue = blue_u[u];
            let first = luma[usize::from(luma_pair[0])];
            let second = luma[usize::from(luma_pair[1])];
            pixel_pair[0] = clamp(first + red);
            pixel_pair[1] = clamp(first - green);
            pixel_pair[2] = clamp(first + blue);
            pixel_pair[3] = u8::MAX;
            pixel_pair[4] = clamp(second + red);
            pixel_pair[5] = clamp(second - green);
            pixel_pair[6] = clamp(second + blue);
            pixel_pair[7] = u8::MAX;
        }
        // NV12 always has an even width, so this only runs for a picture a
        // container stated wrongly. Leaving the last pixel opaque black is
        // better than writing past the row.
        for (sample, pixel) in leftovers.iter().zip(tail.chunks_mut(4)) {
            let brightness = luma[usize::from(*sample)];
            pixel[0] = clamp(brightness);
            pixel[1] = clamp(brightness);
            pixel[2] = clamp(brightness);
            pixel[3] = u8::MAX;
        }
    }
}

/// A Q16 sum to a byte, saturating instead of wrapping.
fn clamp(value: i32) -> u8 {
    // Two conditional moves, no branch to mispredict on a noisy picture.
    (value >> 16).clamp(0, 255) as u8
}

/// Parses the optional worker-count override, keeping it within the ceiling.
///
/// The override chooses how many of the allowed workers to use, not a new
/// ceiling: an absurd value must not spawn hundreds of threads and thrash
/// memory bandwidth on a modest machine.
fn pinned_workers(value: Option<String>) -> Option<usize> {
    value
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .map(|v| v.min(MAX_WORKERS))
}

/// How many workers one conversion may use.
fn workers() -> usize {
    static WORKERS: OnceLock<usize> = OnceLock::new();
    *WORKERS.get_or_init(|| {
        pinned_workers(std::env::var(WORKERS_ENV).ok()).unwrap_or_else(|| {
            thread::available_parallelism()
                .map_or(1, |count| count.get())
                .clamp(1, MAX_WORKERS)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worker_override_respects_the_ceiling() {
        assert_eq!(pinned_workers(Some("100".into())), Some(MAX_WORKERS));
        assert_eq!(pinned_workers(Some("3".into())), Some(3));
        assert_eq!(pinned_workers(Some("0".into())), None);
        assert_eq!(pinned_workers(Some(" 8 ".into())), Some(MAX_WORKERS));
        assert_eq!(pinned_workers(Some("nope".into())), None);
        assert_eq!(pinned_workers(None), None);
    }

    fn nv12(width: usize, height: usize, y: u8, u: u8, v: u8) -> Vec<u8> {
        let mut frame = vec![y; width * height];
        for _ in 0..(width * height / 4) {
            frame.push(u);
            frame.push(v);
        }
        frame
    }

    fn convert(nv12: &[u8], width: usize, height: usize, color: YuvColor) -> Vec<u8> {
        let mut rgba = vec![0; width * height * 4];
        nv12_to_rgba(nv12, width, height, color, &mut rgba);
        rgba
    }

    /// Regression: the SSE path must agree with the scalar oracle for BOTH
    /// chroma sign and bright-but-not-clipped channels. Two historical bugs
    /// lived here: (1) V was read from the all-zero upper shuffle bytes, and
    /// (2) the Q16 sum was packed before `>> 16`, turning 254 into 255.
    #[test]
    fn the_vector_kernel_matches_the_scalar_oracle_for_non_neutral_chroma() {
        for color in [
            YuvColor::default(),
            YuvColor::new(ColorMatrix::Bt709, ColorRange::Full),
        ] {
            // Non-neutral U/V exercises the green/red/blue chroma terms.
            let mut frame = vec![81u8; 8 * 8];
            for _ in 0..(8 * 8 / 4) {
                frame.push(90);
                frame.push(240);
            }
            let mut got = vec![0u8; 8 * 8 * 4];
            convert_rows_tuned(
                &frame,
                8,
                8,
                tables(color),
                Coefficients::of(color),
                &mut got,
                0..8,
            );
            let mut want = vec![0u8; 8 * 8 * 4];
            convert_rows(&frame, 8, 8, tables(color), &mut want, 0..8);
            assert_eq!(got, want, "vector/scalar mismatch for {color:?}");
        }
    }

    #[test]
    fn a_limited_range_picture_reaches_black_and_white() {
        // The studio swing puts black at luma 16 and white at 235.
        let black = convert(&nv12(8, 8, 16, 128, 128), 8, 8, YuvColor::default());
        assert!(black.chunks(4).all(|pixel| pixel == [0, 0, 0, 255]));
        let white = convert(&nv12(8, 8, 235, 128, 128), 8, 8, YuvColor::default());
        assert!(
            white
                .chunks(4)
                .all(|pixel| pixel[0] >= 254 && pixel[1] >= 254 && pixel[2] >= 254),
            "white must not come back grey: {:?}",
            &white[..4]
        );
    }

    #[test]
    fn a_full_range_picture_uses_the_whole_byte() {
        let color = YuvColor::new(ColorMatrix::Bt709, ColorRange::Full);
        let black = convert(&nv12(8, 8, 0, 128, 128), 8, 8, color);
        assert!(black.chunks(4).all(|pixel| pixel == [0, 0, 0, 255]));
        let white = convert(&nv12(8, 8, 255, 128, 128), 8, 8, color);
        assert!(white.chunks(4).all(|pixel| pixel == [255, 255, 255, 255]));
    }

    #[test]
    fn chroma_lands_in_the_channel_its_matrix_names() {
        // Luma 81 with U 90 and V 240 is crimson in BT.601, and the red channel
        // must dominate while blue and green stay near black.
        let rgba = convert(
            &nv12(8, 8, 81, 90, 240),
            8,
            8,
            YuvColor::new(ColorMatrix::Bt601, ColorRange::Limited),
        );
        let pixel = &rgba[..4];
        assert!(pixel[0] > 240, "red must be nearly full: {pixel:?}");
        assert!(pixel[1] < 20, "green must be nearly empty: {pixel:?}");
        assert!(pixel[2] < 40, "blue must be nearly empty: {pixel:?}");

        // The same samples under BT.709 stay red, with a hue the matrix moves.
        let hd = convert(
            &nv12(8, 8, 81, 90, 240),
            8,
            8,
            YuvColor::new(ColorMatrix::Bt709, ColorRange::Limited),
        );
        assert!(
            hd[0] > 230 && hd[1] < 30,
            "BT.709 stays red: {:?}",
            &hd[..4]
        );
    }

    #[test]
    fn every_picture_pixel_is_written_opaque() {
        // A 2x2 block with four distinct luma samples, so a pair that packed
        // the wrong channel would show.
        let width = 40;
        let height = 24;
        let mut frame = Vec::new();
        for y in 0..height {
            for x in 0..width {
                frame.push(((x * 7 + y * 13) % 256) as u8);
            }
        }
        for _ in 0..(width * height / 4) {
            frame.push(96);
            frame.push(160);
        }
        let rgba = convert(&frame, width, height, YuvColor::default());
        assert_eq!(rgba.len(), width * height * 4);
        assert!(rgba.chunks(4).all(|pixel| pixel[3] == 255));
        assert!(
            rgba.chunks(4)
                .any(|pixel| pixel[0] != pixel[1] || pixel[1] != pixel[2]),
            "a chroma pair must colour the picture"
        );
    }

    #[test]
    fn splitting_a_picture_across_workers_changes_nothing() {
        let width = 1920;
        let height = 1088;
        let mut frame: Vec<u8> = (0..width * height).map(|i| (i % 251) as u8).collect();
        for i in 0..(width * height / 4) {
            frame.push((i % 200) as u8);
            frame.push(((i * 3) % 256) as u8);
        }
        let color = YuvColor::new(ColorMatrix::Bt709, ColorRange::Limited);
        let mut whole = vec![0; width * height * 4];
        convert_rows(&frame, width, height, tables(color), &mut whole, 0..height);

        let mut split = vec![0; width * height * 4];
        nv12_to_rgba(&frame, width, height, color, &mut split);
        assert_eq!(whole, split, "a worker split must not change a byte");
    }

    #[test]
    fn bgra_rows_swizzle_in_place_order() {
        let source = [0u8, 0, 255, 0, 10, 20, 30, 0];
        let mut dest = [0u8; 8];
        bgra_to_rgba(&source, &mut dest);
        assert_eq!(dest, [255, 0, 0, 255, 30, 20, 10, 255]);
    }

    /// The original per-pixel multiply kernel, kept as a bit-exact oracle.
    fn reference(nv12: &[u8], width: usize, height: usize, color: YuvColor) -> Vec<u8> {
        let Coefficients {
            luma,
            red_v,
            green_u,
            green_v,
            blue_u,
            luma_bias,
        } = Coefficients::of(color);
        let mut rgba = vec![0; width * height * 4];
        let chroma_plane = width * height;
        for y in 0..height {
            let luma_row = &nv12[y * width..][..width];
            let chroma_row = &nv12[chroma_plane + (y / 2) * width..][..width];
            let out_row = &mut rgba[y * width * 4..][..width * 4];
            let (pairs, leftovers) = luma_row.as_chunks::<2>();
            let (chroma, _) = chroma_row.as_chunks::<2>();
            let (out, tail) = out_row.as_chunks_mut::<8>();
            for ((luma_pair, chroma_pair), pixel_pair) in
                pairs.iter().zip(chroma.iter()).zip(out.iter_mut())
            {
                let blue_difference = i32::from(chroma_pair[0]) - 128;
                let red_difference = i32::from(chroma_pair[1]) - 128;
                let red = red_v * red_difference;
                let green = green_u * blue_difference + green_v * red_difference;
                let blue = blue_u * blue_difference;
                let first = (i32::from(luma_pair[0]) - luma_bias) * luma;
                let second = (i32::from(luma_pair[1]) - luma_bias) * luma;
                pixel_pair[0] = clamp(first + red);
                pixel_pair[1] = clamp(first - green);
                pixel_pair[2] = clamp(first + blue);
                pixel_pair[3] = u8::MAX;
                pixel_pair[4] = clamp(second + red);
                pixel_pair[5] = clamp(second - green);
                pixel_pair[6] = clamp(second + blue);
                pixel_pair[7] = u8::MAX;
            }
            for (sample, pixel) in leftovers.iter().zip(tail.chunks_mut(4)) {
                let brightness = (i32::from(*sample) - luma_bias) * luma;
                pixel[0] = clamp(brightness);
                pixel[1] = clamp(brightness);
                pixel[2] = clamp(brightness);
                pixel[3] = u8::MAX;
            }
        }
        rgba
    }

    #[test]
    fn lookup_tables_are_bit_identical_to_the_multiply_kernel() {
        let (width, height) = (64, 64);
        let mut frame = Vec::new();
        for i in 0..width * height {
            frame.push(i as u8);
        }
        for i in 0..(width * height / 4) {
            frame.push((i * 7 + 3) as u8);
            frame.push((i * 11 + 5) as u8);
        }

        let colors = [
            YuvColor::new(ColorMatrix::Bt601, ColorRange::Limited),
            YuvColor::new(ColorMatrix::Bt601, ColorRange::Full),
            YuvColor::new(ColorMatrix::Bt709, ColorRange::Limited),
            YuvColor::new(ColorMatrix::Bt709, ColorRange::Full),
            YuvColor::new(ColorMatrix::Bt2020, ColorRange::Limited),
            YuvColor::new(ColorMatrix::Bt2020, ColorRange::Full),
        ];
        for color in colors {
            let mut actual = vec![0; width * height * 4];
            nv12_to_rgba(&frame, width, height, color, &mut actual);
            assert_eq!(actual, reference(&frame, width, height, color), "{color:?}");
        }
    }

    /// The vector kernel must agree with the scalar oracle byte for byte, on
    /// every colour space, on widths the four-pixel loop takes whole and on
    /// widths that leave a tail pixel. The output buffer starts dirty, so a
    /// byte the kernel forgets to write shows up as a mismatch instead of a
    /// lucky zero.
    #[test]
    fn the_vector_kernel_matches_the_scalar_oracle() {
        let colors = [
            YuvColor::new(ColorMatrix::Bt601, ColorRange::Limited),
            YuvColor::new(ColorMatrix::Bt601, ColorRange::Full),
            YuvColor::new(ColorMatrix::Bt709, ColorRange::Limited),
            YuvColor::new(ColorMatrix::Bt709, ColorRange::Full),
            YuvColor::new(ColorMatrix::Bt2020, ColorRange::Limited),
            YuvColor::new(ColorMatrix::Bt2020, ColorRange::Full),
        ];
        for width in [2usize, 6, 18, 36, 100, 1940] {
            let height = 8;
            let mut frame: Vec<u8> = (0..width * height)
                .map(|index| (index * 37 % 256) as u8)
                .collect();
            for index in 0..(width * height / 4) {
                frame.push((index * 13 + 11) as u8);
                frame.push((index * 29 + 7) as u8);
            }
            for color in colors {
                let expected = reference(&frame, width, height, color);
                let mut actual = vec![0xAA; width * height * 4];
                nv12_to_rgba(&frame, width, height, color, &mut actual);
                assert_eq!(
                    actual, expected,
                    "a {width}x{height} picture in {color:?} differs from the oracle"
                );
            }
        }
    }

    /// Measures one conversion kernel over `frames` pictures.
    ///
    /// Returns `(average_ms, min_ms, max_ms)`. The output buffer is reused, so
    /// the measurement covers the kernel and not the allocator.
    fn measure(
        nv12: &[u8],
        width: usize,
        height: usize,
        color: YuvColor,
        frames: usize,
        mut run: impl FnMut(&[u8], usize, usize, YuvColor, &mut [u8]),
    ) -> (f64, f64, f64) {
        use std::time::Instant;

        let mut rgba = vec![0u8; width * height * 4];
        for _ in 0..10 {
            run(nv12, width, height, color, &mut rgba);
        }
        let mut best = f64::INFINITY;
        let mut worst = 0.0f64;
        let mut total = 0.0f64;
        for _ in 0..frames {
            let started = Instant::now();
            run(nv12, width, height, color, &mut rgba);
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            total += elapsed;
            best = best.min(elapsed);
            worst = worst.max(elapsed);
        }
        (total / frames as f64, best, worst)
    }

    /// Splits the rows across `workers` threads exactly as the production
    /// conversion does, so the report can compare a forced fan-out with the
    /// threshold the production path actually applies.
    fn parallel_rows(
        nv12: &[u8],
        width: usize,
        height: usize,
        color: YuvColor,
        rgba: &mut [u8],
        workers: usize,
    ) {
        let tables = tables(color);
        let coefficients = Coefficients::of(color);
        let workers = workers.min(height / 2).max(1);
        if workers == 1 {
            convert_rows_tuned(nv12, width, height, tables, coefficients, rgba, 0..height);
            return;
        }
        let rows = height / workers;
        let mut rest = rgba;
        let mut first = 0;
        std::thread::scope(|scope| {
            for _ in 1..workers {
                let (head, tail) = rest.split_at_mut(rows * width * 4);
                let span = first..first + rows;
                scope.spawn(move || {
                    convert_rows_tuned(nv12, width, height, tables, coefficients, head, span);
                });
                rest = tail;
                first += rows;
            }
            convert_rows_tuned(
                nv12,
                width,
                height,
                tables,
                coefficients,
                rest,
                first..height,
            );
        });
    }

    /// Reports the cost of one picture, sequentially and across cores.
    ///
    /// Run explicitly; it is a measurement, not a gate:
    /// `cargo test --lib conversion_cost_report -- --ignored --nocapture`.
    #[test]
    #[ignore = "performance measurement; run explicitly with --ignored"]
    fn conversion_cost_report() {
        let color = YuvColor::new(ColorMatrix::Bt709, ColorRange::Limited);
        let force_parallel: usize = std::env::var(WORKERS_ENV)
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or_else(workers);
        for (width, height) in [(1280usize, 720usize), (1920, 1080), (3840, 2160)] {
            let frame = nv12(width, height, 128, 128, 128);
            let frames = if width >= 3840 { 30 } else { 120 };
            let tables = tables(color);
            let coefficients = Coefficients::of(color);
            let sequential = measure(
                &frame,
                width,
                height,
                color,
                frames,
                |source, w, h, _, out| {
                    convert_rows_tuned(source, w, h, tables, coefficients, out, 0..h)
                },
            );
            let parallel = measure(
                &frame,
                width,
                height,
                color,
                frames,
                |source, w, h, c, out| parallel_rows(source, w, h, c, out, force_parallel),
            );
            let production = measure(
                &frame,
                width,
                height,
                color,
                frames,
                |source, w, h, c, out| nv12_to_rgba(source, w, h, c, out),
            );
            println!(
                "conversion {width}x{height}: workers={force_parallel} \
                 sequential avg={:.3}ms min={:.3} max={:.3} | \
                 forced_parallel avg={:.3}ms min={:.3} max={:.3} speedup={:.2}x | \
                 production avg={:.3}ms fanout={}",
                sequential.0,
                sequential.1,
                sequential.2,
                parallel.0,
                parallel.1,
                parallel.2,
                sequential.0 / parallel.0,
                production.0,
                width * height >= PARALLEL_MIN_PIXELS,
            );
        }
    }
}
