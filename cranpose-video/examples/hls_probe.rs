//! Opens a source and reports what this build does with it.
//!
//! ```text
//! cargo run -p cranpose-video --example hls_probe -- https://host/index.fmp4.m3u8
//! ```

use std::time::{Duration, Instant};

use cranpose_video::{VideoPlayer, VideoState, start_log_file, stop_log_file};

fn main() {
    let url = std::env::args().nth(1).expect("pass a path or URL to open");
    let seconds: u64 = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(15);
    let log = "target/hls_probe.log";
    let _ = std::fs::remove_file(log);
    let _ = start_log_file(log);

    println!("decoder: {}", cranpose_video::decoder_name());
    let player = match VideoPlayer::open(url.as_str()) {
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
    let mut last_state = VideoState::Playing;
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Some(frame) = player.take_frame() {
            frames += 1;
            if frames <= 3 || frames % 30 == 0 {
                println!(
                    "frame {frames}: pts={:?} {}x{}",
                    frame.pts(),
                    frame.width(),
                    frame.height()
                );
            }
            player.recycle(frame);
        }
        std::thread::sleep(Duration::from_millis(30));

        let state = player.state();
        if state != last_state {
            println!(
                "state -> {state:?} info={:?} position={:?}",
                player.info(),
                player.position()
            );
            last_state = state.clone();
        }
        if matches!(state, VideoState::Failed(_) | VideoState::Ended) {
            break;
        }
    }
    println!(
        "done: frames={frames} position={:?} state={:?} info={:?}",
        player.position(),
        player.state(),
        player.info()
    );
    drop(player);
    stop_log_file();
    if let Ok(text) = std::fs::read_to_string(log) {
        println!("=== {log} ===");
        print!("{text}");
    }
}
