//! Runs a real source and reports whether its published frame cadence stutters.
//!
//! Unlike the synthetic `tests/pacing.rs` gate, this uses the actual platform
//! decoder, so it exercises Media Foundation / AVFoundation / MediaCodec /
//! GStreamer, HLS joins, and the paired native audio path. It needs no changes
//! to the application: pass the same path or URL the application opens.
//!
//!```text
//! cargo run -p cranpose-video --example stutter_probe -- tests/fixtures/clip.mp4 10
//! cargo run -p cranpose-video --features native-audio --example stutter_probe -- https://host/index.m3u8 20
//!```
//!
//! The verdict is about *frame publication*, which is what video pacing and
//! the audio clock both follow. It cannot see the compositor or a GPU present;
//! a screen may still drop a published picture. Use it to tell a processing
//! problem from a display problem: `STUTTER` here means the player itself is
//! delivering pictures late, regardless of what the screen does afterwards.

use std::time::Duration;

use cranpose_video::{PlaybackEvent, VideoPlayer, VideoState};
use web_time::Instant;

/// Buckets are ratios of the observed gap to one expected frame period.
const BUCKETS: [(&str, f64, f64); 6] = [
    ("<0.75x", 0.0, 0.75),
    ("0.75-1.25x", 0.75, 1.25),
    ("1.25-1.5x", 1.25, 1.5),
    ("1.5-2x", 1.5, 2.0),
    ("2-4x", 2.0, 4.0),
    (">=4x", 4.0, f64::INFINITY),
];

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn main() {
    let source = std::env::args().nth(1).expect("pass a path or URL to open");
    let seconds: u64 = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(15);
    let fps_hint: Option<f64> = std::env::args().nth(3).and_then(|value| value.parse().ok());

    println!("platform: {}", cranpose_video::platform().name());
    println!("decoder:  {}", cranpose_video::decoder_name());
    println!(
        "native-audio: {} (opt-in; this probe only hears the source if the app enables it)",
        cfg!(feature = "native-audio")
    );
    if !cranpose_video::is_supported() {
        println!(
            "note: this build has no platform decoder; an application-supplied VideoDecoder is needed"
        );
    }

    let player = match VideoPlayer::open(source.as_str()) {
        Ok(player) => player,
        Err(error) => {
            println!("open failed: {error:?}");
            return;
        }
    };
    let info = player.info();
    println!("source:   {source}");
    println!("info:     {info:?}");
    println!("state:    {:?}", player.state());

    let period = info
        .frame_rate
        .and_then(|rate| rate.frame_duration())
        .or_else(|| {
            fps_hint
                .filter(|fps| *fps > 0.0 && fps.is_finite())
                .map(|fps| Duration::from_secs_f64(1.0 / fps))
        })
        .unwrap_or_else(|| Duration::from_millis(33));
    println!("expected frame period: {period:?}");

    let started = Instant::now();
    if let Err(error) = player.play() {
        println!("play failed: {error:?}");
        return;
    }

    // Drive the player and watch for the end, but do NOT measure cadence from
    // these polls: a single-picture slot can be read late when this thread is
    // descheduled, which fabricates a long gap without losing a frame. The
    // authoritative `publication_timeline` is stamped once per publication on
    // the decode thread — the same source the synthetic 60 fps gate uses.
    // Select by wall-clock window rather than by the ring's current length: a
    // long-lived stream can wrap the ring while this probe is running.
    let mut last_state = VideoState::Playing;
    while started.elapsed() < Duration::from_secs(seconds) {
        if let Some(frame) = player.take_frame() {
            player.recycle(frame);
        }
        // Yielding in a tight loop competes with the decode thread and its
        // conversion workers on a phone-sized CPU. The publication timeline is
        // stamped in the player, so sleeping here cannot fabricate a gap; it
        // only keeps the observer from causing one.
        std::thread::sleep(Duration::from_millis(1));

        let state = player.state();
        if state != last_state {
            println!("state -> {state:?} position={:?}", player.position());
            last_state = state.clone();
        }
        if matches!(state, VideoState::Failed(_) | VideoState::Ended) {
            break;
        }
    }
    let elapsed = started.elapsed();
    let stamps: Vec<(Instant, Duration)> = player
        .publication_timeline()
        .into_iter()
        .filter(|(at, _)| *at >= started && *at <= started + elapsed)
        .collect();
    let diagnostics = player.diagnostics();
    let events = player.events();

    println!();
    println!("=== cadence report ===");
    println!("authoritative publications in window: {}", stamps.len());
    println!("wall elapsed:                          {elapsed:?}");
    if !elapsed.is_zero() {
        println!(
            "achieved publication fps:              {:.2}",
            stamps.len() as f64 / elapsed.as_secs_f64()
        );
    }
    println!(
        "published (lifetime): {} reanchors: {} errors: {} seeks: {} plays: {} uptime: {:?}",
        diagnostics.frames_published(),
        diagnostics.reanchors(),
        diagnostics.errors(),
        diagnostics.seeks(),
        diagnostics.plays(),
        diagnostics.uptime()
    );
    println!(
        "layout: {:?} decode: {:?} convert: {:?}",
        diagnostics.pixel_format(),
        diagnostics.decode_time(),
        diagnostics.convert_time()
    );

    let watched = stamps.iter().skip(2);
    let gaps: Vec<Duration> = watched
        .collect::<Vec<_>>()
        .windows(2)
        .map(|pair| pair[1].0.saturating_duration_since(pair[0].0))
        .collect();
    let ratios: Vec<f64> = gaps
        .iter()
        .map(|gap| gap.as_secs_f64() / period.as_secs_f64())
        .collect();
    let mut sorted = ratios.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let long_gaps = ratios.iter().filter(|ratio| **ratio > 1.5).count();

    for (label, low, high) in BUCKETS {
        let count = ratios
            .iter()
            .filter(|ratio| **ratio >= low && **ratio < high)
            .count();
        println!("  {label:>10}: {count}");
    }
    for (name, p) in [("p50", 0.50), ("p95", 0.95), ("p99", 0.99), ("max", 1.0)] {
        let ratio = percentile(&sorted, p);
        let ms = ratio * period.as_secs_f64() * 1000.0;
        println!("{name:>4}: {ratio:6.2}x ({ms:8.2} ms)");
    }
    println!("long gaps (>1.5x): {long_gaps} / {}", ratios.len());

    println!();
    println!("=== events that explain a hitch ===");
    let mut notable = 0;
    for record in &events {
        match record.event() {
            PlaybackEvent::Reanchored { position } => {
                println!(
                    "  #{} at {:?} re-anchor -> {position:?}",
                    record.sequence(),
                    record.at()
                );
                notable += 1;
            }
            PlaybackEvent::Failed { error } => {
                println!(
                    "  #{} at {:?} failed: {error:?}",
                    record.sequence(),
                    record.at()
                );
                notable += 1;
            }
            PlaybackEvent::Seeking { position } => {
                println!(
                    "  #{} at {:?} seek -> {position:?}",
                    record.sequence(),
                    record.at()
                );
                notable += 1;
            }
            _ => {}
        }
    }
    if notable == 0 {
        println!("  (none in the retained {}-event window)", events.len());
    }
    println!(
        "note: the event ring keeps at most {} records; counters above cover the whole run",
        events.len().max(256)
    );

    let allowed_long_gaps = (ratios.len() / 100).max(1);
    let smooth = sorted.last().copied().unwrap_or(0.0) <= 2.0
        && long_gaps <= allowed_long_gaps
        && diagnostics.reanchors() <= 1
        && diagnostics.errors() == 0;
    println!();
    if smooth {
        println!("VERDICT: SMOOTH — the player is publishing at the source's frame rate.");
    } else {
        println!("VERDICT: STUTTER — the player itself is delivering pictures late.");
        if sorted.last().copied().unwrap_or(0.0) > 2.0 {
            println!("  - a published gap exceeded twice the frame period");
        }
        if long_gaps > allowed_long_gaps {
            println!("  - repeated long gaps were measured");
        }
        if diagnostics.reanchors() > 1 {
            println!("  - the clock was repeatedly re-anchored to late pictures");
        }
        if diagnostics.errors() > 0 {
            println!("  - the decoder reported errors");
        }
        println!("  This is a processing-side signal, not a display/compositor measurement.");
    }
}
