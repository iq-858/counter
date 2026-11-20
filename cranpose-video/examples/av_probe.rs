//! Plays a source and reports what both tracks did, with crate diagnostics on
//! stderr.
//!
//! ```text
//! cargo run --no-default-features --features native-audio --example av_probe -- clip.mp4
//! ```

use std::time::{Duration, Instant};

use cranpose_video::{VideoPlayer, VideoState, set_diagnostic_sink, start_log_file, stop_log_file};

fn main() {
    set_diagnostic_sink(Some(Box::new(|level, target, message| {
        eprintln!("[{level}] {target}: {message}");
    })));
    let log = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "target/av_probe.log".to_owned());
    // The log file installs its own sink, so it is started last and the whole
    // session is read back from the file at the end.
    let _ = std::fs::remove_file(&log);
    match start_log_file(&log) {
        Ok(()) => println!("log: {log}"),
        Err(error) => eprintln!("log file unavailable: {error}"),
    }

    let source = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/bench/b1080p30a.mp4".to_owned());
    let seconds: u64 = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);

    println!("decoder: {}", cranpose_video::decoder_name());
    let player = match VideoPlayer::open(source.as_str()) {
        Ok(player) => player,
        Err(error) => {
            println!("open failed: {error:?}");
            return;
        }
    };
    println!("opened: {:?} state={:?}", player.info(), player.state());
    if let Err(error) = player.play() {
        println!("play failed: {error:?}");
        return;
    }

    let start = Instant::now();
    let mut frames = 0u64;
    let mut first_pts = None;
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Some(frame) = player.take_frame() {
            frames += 1;
            first_pts.get_or_insert(frame.pts());
            player.recycle(frame);
        }
        std::thread::sleep(Duration::from_millis(10));
        if matches!(player.state(), VideoState::Failed(_) | VideoState::Ended) {
            break;
        }
    }

    let diagnostics = player.diagnostics();
    println!(
        "video: frames={frames} first_pts={first_pts:?} state={:?} errors={} reanchors={} decode={:?} convert={:?}",
        player.state(),
        diagnostics.errors(),
        diagnostics.reanchors(),
        diagnostics.decode_time(),
        diagnostics.convert_time()
    );
    println!(
        "audio: muted={} volume={}",
        player.is_muted(),
        player.volume()
    );
    drop(player);
    stop_log_file();
    if let Ok(text) = std::fs::read_to_string(&log) {
        println!("=== {log} ===");
        print!("{text}");
    }
}
