//! Switches between sources as fast as a reels feed does and reports what each
//! open delivered.
//!
//! Every source opens while the previous player is still alive, then the old
//! one is dropped: that is the case where two decoders briefly share one audio
//! device, which is what a swipe between clips actually does.
//!
//! ```text
//! cargo run --no-default-features --features native-audio --example switch_probe -- clip.mp4 other.mp4
//! ```

use std::time::{Duration, Instant};

use cranpose_video::{VideoPlayer, start_log_file, stop_log_file};

/// How long each clip holds the screen before the next one replaces it.
const HOLD: Duration = Duration::from_millis(500);

/// How many times the list is played through.
const ROUNDS: usize = 4;

fn main() {
    let mut sources: Vec<String> = std::env::args().skip(1).collect();
    if sources.is_empty() {
        sources = vec![
            "target/bench/b1080p30a.mp4".to_owned(),
            "target/bench/b720p25.mp4".to_owned(),
            "target/bench/b1080p25.mp4".to_owned(),
        ];
    }
    let log = "target/switch_probe.log";
    let _ = std::fs::remove_file(log);
    let _ = start_log_file(log);

    let mut failures = 0usize;
    let mut total_frames = 0u64;
    let mut worst_open = Duration::ZERO;
    let mut current: Option<VideoPlayer> = None;
    for round in 0..ROUNDS {
        for source in &sources {
            let began = Instant::now();
            // Open first, drop second: the two players overlap, as they do when
            // a feed builds the next clip before it removes the old one.
            let next = VideoPlayer::open(source.as_str());
            drop(current.take());
            let player = match next {
                Ok(player) => player,
                Err(error) => {
                    println!("round {round} {source}: open failed {error:?}");
                    failures += 1;
                    continue;
                }
            };
            let _ = player.play();
            let mut frames = 0u64;
            let mut first = None;
            while began.elapsed() < HOLD {
                if let Some(frame) = player.take_frame() {
                    frames += 1;
                    first.get_or_insert(frame.pts());
                    player.recycle(frame);
                }
                std::thread::sleep(Duration::from_millis(4));
            }
            total_frames += frames;
            worst_open = worst_open.max(began.elapsed());
            println!(
                "round {round} {:>28}: open+hold={:?} first={first:?} frames={frames} state={:?}",
                source.rsplit('/').next().unwrap_or(source),
                began.elapsed(),
                player.state()
            );
            current = Some(player);
        }
    }
    drop(current);
    stop_log_file();

    let text = std::fs::read_to_string(log).unwrap_or_default();
    let count = |target: &str| {
        text.lines()
            .filter(|line| line.contains(&format!(" {target} ")))
            .count()
    };
    println!("total frames={total_frames} open failures={failures} slowest switch={worst_open:?}");
    println!(
        "log: audio setups={} first samples={} device renders={} first pictures={} errors={}",
        count("mf_audio.setup"),
        count("mf_audio.sample"),
        count("mf_audio.render"),
        count("player.video"),
        text.lines()
            .filter(|line| line.contains(" WARN ") || line.contains(" ERROR "))
            .count()
    );
}
