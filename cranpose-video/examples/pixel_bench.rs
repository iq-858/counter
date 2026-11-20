//! Measures what one picture costs to turn into RGBA, per layout.
//!
//! A decoder hands out what its platform produces, and the screen needs RGBA.
//! This example answers which arriving layout costs less work on this machine,
//! at the sizes a video comes in:
//!
//! ```text
//! cargo run --release --example pixel_bench
//! CRANPOSE_VIDEO_WORKERS=1 cargo run --release --example pixel_bench
//! ```
//!
//! Every subject runs over the same span, warm-up included. The interesting
//! number is the kernel with a pooled buffer, because that is what the decode
//! thread does; the last line is the same work with a fresh allocation, which
//! is what a player pays when it does not pool.
//!
//! Run it on the slowest device a build ships to. A desktop result does not
//! decide a phone.

use std::time::{Duration, Instant};

use cranpose_video::{ColorMatrix, ColorRange, VideoFrame, YuvColor};

/// The sizes a video comes in, widest last.
const SIZES: [(u32, u32); 3] = [(1280, 720), (1920, 1080), (3840, 2160)];

/// How long each subject runs: long enough that one scheduling hiccup does not
/// decide the answer, short enough to run on a phone.
const SPAN: Duration = Duration::from_millis(500);

fn planes(width: u32, height: u32) -> Vec<u8> {
    let mut planes = vec![0u8; width as usize * height as usize];
    for (index, sample) in planes.iter_mut().enumerate() {
        // A gradient rather than a flat plane, so the maths cannot be skipped.
        *sample = (index % 236 + 16) as u8;
    }
    planes.resize(width as usize * height as usize * 3 / 2, 128);
    planes
}

fn report(label: &str, width: u32, height: u32, frames: u64, elapsed: Duration) {
    let seconds = elapsed.as_secs_f64();
    let fps = frames as f64 / seconds;
    let written = f64::from(width) * f64::from(height) * 4.0 * fps / 1e6;
    println!(
        "  {label:<26} {fps:8.1}/s   {written:8.1} MB/s   {:6.2} ms",
        seconds * 1000.0 / frames as f64
    );
}

fn main() {
    println!(
        "cores: {} (set CRANPOSE_VIDEO_WORKERS to pin the converter)",
        std::thread::available_parallelism().map_or(1, |count| count.get())
    );
    for (width, height) in SIZES {
        println!("\n{width}x{height}");
        let color = YuvColor::new(ColorMatrix::Bt709, ColorRange::Limited);
        let planes =
            VideoFrame::from_nv12(width, height, Duration::ZERO, planes(width, height), color)
                .expect("the planes match the size");
        let rgba_frame = VideoFrame::from_rgba8(
            width,
            height,
            Duration::ZERO,
            vec![0x40; width as usize * height as usize * 4],
        )
        .expect("the buffer matches the size");

        let mut buffer = Vec::new();
        let started = Instant::now();
        let mut frames = 0u64;
        let mut sink = 0u64;
        while started.elapsed() < SPAN {
            planes.convert_into(&mut buffer);
            sink = sink.wrapping_add(buffer[0] as u64);
            frames += 1;
        }
        report("NV12 to RGBA", width, height, frames, started.elapsed());

        // The copy a screen makes when the decoder already produced RGBA: the
        // floor any path pays to hand the pixels over.
        let started = Instant::now();
        frames = 0;
        while started.elapsed() < SPAN {
            rgba_frame.convert_into(&mut buffer);
            sink = sink.wrapping_add(buffer[0] as u64);
            frames += 1;
        }
        report(
            "RGBA hand-over (copy)",
            width,
            height,
            frames,
            started.elapsed(),
        );

        let started = Instant::now();
        frames = 0;
        while started.elapsed() < SPAN {
            let mut fresh = Vec::new();
            planes.convert_into(&mut fresh);
            sink = sink.wrapping_add(fresh[0] as u64);
            frames += 1;
        }
        report(
            "NV12 to RGBA, new buffer",
            width,
            height,
            frames,
            started.elapsed(),
        );
        std::hint::black_box(sink);
    }
}
