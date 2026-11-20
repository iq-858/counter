//! Runs many players at once, the way a short-video feed does.
//!
//! A reels-style screen keeps several players alive — one on screen, two ready
//! to slide in, the rest parked — and the question that decides battery life is
//! what the parked ones cost. This probe opens `n` players on one source and
//! walks them through the phases a feed produces:
//!
//! ```text
//! cargo run --release --example stress_probe -- clip.mp4 4 5
//! ```
//!
//! `clip.mp4` is the source, `4` is how many players to keep open and `5` is how
//! many seconds each phase runs. Phases:
//!
//! 1. **open** — every player opens its source, all paused.
//! 2. **play** — every player plays, and the screen takes pictures from all of
//!    them, which is what a feed does when it keeps a couple of tiles live.
//! 3. **park** — every player is paused and the screen keeps asking for
//!    pictures. Nothing should be decoded, converted, published or woken: a
//!    parked player must be free.
//! 4. **seek** — a scrub through each player, twice, to show that a seek while
//!    parked still costs only the one picture it publishes.
//! 5. **close** — every player closes, and the probe reports what is left.
//!
//! The probe prints its own process id so an external sampler can read the CPU
//! time of the whole run (`Get-Process -Id <pid>` on Windows, `/proc/<pid>` on
//! Linux). `frames` per phase is the number this file exists to make visible:
//! the parked phase must report **zero**.

use std::{
    thread,
    time::{Duration, Instant},
};

use cranpose_video::VideoPlayer;

/// How often the simulated screen asks every player for a picture. A screen
/// draws at 60 Hz, so asking any slower would hide a player that publishes.
const TICK: Duration = Duration::from_millis(16);

/// What one phase measured.
struct Phase {
    name: &'static str,
    elapsed: Duration,
    frames: u64,
}

impl Phase {
    fn report(&self, players: usize) {
        let per_second = if self.elapsed.is_zero() {
            0.0
        } else {
            self.frames as f64 / self.elapsed.as_secs_f64()
        };
        println!(
            "  {:<6} {:>8.2} s  {:>6} frames  {:>8.2} pictures/s  {:>6.2} per player",
            self.name,
            self.elapsed.as_secs_f64(),
            self.frames,
            per_second,
            per_second / players.max(1) as f64
        );
    }
}

/// Takes a picture from every player, the way a screen composes its tiles.
fn pump(players: &[VideoPlayer]) -> u64 {
    let mut frames = 0;
    for player in players {
        if let Some(frame) = player.take_frame() {
            frames += 1;
            player.recycle(frame);
        }
    }
    frames
}

/// Ticks every player for `span`, counting what they published.
fn run(players: &[VideoPlayer], span: Duration) -> Phase {
    let started = Instant::now();
    let mut frames = 0;
    while started.elapsed() < span {
        frames += pump(players);
        thread::sleep(TICK);
    }
    Phase {
        name: "",
        elapsed: started.elapsed(),
        frames,
    }
}

fn main() {
    let source = std::env::args().nth(1).expect("pass a path or URL to open");
    let count: usize = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(4);
    let seconds: u64 = std::env::args()
        .nth(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    let span = Duration::from_secs(seconds);

    println!("pid:      {}", std::process::id());
    println!("platform: {}", cranpose_video::platform().name());
    println!("decoder:  {}", cranpose_video::decoder_name());
    println!(
        "load:     {count} players on {source}, {seconds} s per phase (a {} ms screen tick)",
        TICK.as_millis()
    );

    // Phase 1: open. A feed opens the visible item and its neighbours.
    let started = Instant::now();
    let mut opened = Vec::new();
    let mut refused = 0;
    for _ in 0..count {
        match VideoPlayer::open(source.as_str()) {
            Ok(player) => opened.push(player),
            Err(error) => {
                println!("  open failed: {error:?}");
                refused += 1;
            }
        }
    }
    println!();
    println!("=== phases ===");
    Phase {
        name: "open",
        elapsed: started.elapsed(),
        frames: 0,
    }
    .report(count.max(1));
    println!(
        "  info      {}x{} {:?}",
        opened.first().map_or(0, |player| player.info().width),
        opened.first().map_or(0, |player| player.info().height),
        opened.first().and_then(|player| player.info().frame_rate),
    );
    if opened.is_empty() {
        println!("no player opened; nothing to measure");
        return;
    }

    // Phase 2: play. Every player runs, which a feed does for a couple of tiles
    // at once; the rest of a real feed would stay parked.
    for player in &opened {
        let _ = player.play();
    }
    let mut playing = run(&opened, span);
    playing.name = "play";
    playing.report(opened.len());

    // Phase 3: park. Paused players must cost nothing at all: the screen keeps
    // asking, and the answer stays zero.
    for player in &opened {
        let _ = player.pause();
    }
    thread::sleep(Duration::from_millis(50));
    // Drain the picture each player published before it was paused, so the
    // parked phase measures only what a paused player produces.
    let _ = pump(&opened);
    let mut parked = run(&opened, span);
    parked.name = "park";
    parked.report(opened.len());

    // Phase 4: seek. A scrub on a parked player publishes one picture per seek
    // and decodes nothing else.
    let started = Instant::now();
    let mut frames = 0;
    for player in &opened {
        let Some(duration) = player.duration() else {
            continue;
        };
        for step in [0.75f32, 0.25, 0.5] {
            let _ = player.seek(duration.mul_f32(step));
            // Give the decode thread one turn to publish the sought picture.
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                let taken = pump(std::slice::from_ref(player));
                if taken > 0 {
                    frames += taken;
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
    Phase {
        name: "seek",
        elapsed: started.elapsed(),
        frames,
    }
    .report(opened.len());

    // Phase 5: the counters the player itself keeps, then close.
    let published: u64 = opened
        .iter()
        .map(|player| player.diagnostics().frames_published())
        .sum();
    let errors: u64 = opened
        .iter()
        .map(|player| player.diagnostics().errors())
        .sum();
    println!("  total     {published} pictures published, {errors} errors");

    let started = Instant::now();
    for mut player in opened {
        player.close();
    }
    println!("  closed in {:.2} s", started.elapsed().as_secs_f64());
    if refused > 0 {
        println!("note: {refused} of {count} players refused to open");
    }
    println!();
    println!(
        "parked players publishing pictures is the number to watch: it must be 0, \
         and the CPU time of this process while parked should be almost nothing."
    );
}
