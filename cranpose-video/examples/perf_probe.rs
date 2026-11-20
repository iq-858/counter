//! Measures what one player costs on this machine, and what a reels feed costs
//! when it switches clips.
//!
//! ```text
//! cargo run --release --example perf_probe -- play target/bench/b1080p30a.mp4 8
//! cargo run --release --example perf_probe -- play target/bench/b1080p30a.mp4 8 recycle
//! cargo run --release --example perf_probe -- reels 700 3 a.mp4 b.mp4 c.mp4
//! ```
//!
//! `play` opens one source, reports how long the open took and how long the
//! first picture took after `play()`, then drives the player for the requested
//! number of seconds. By default it takes each picture and drops it, which is
//! what the ready-made widget does: the pixels move into the bitmap, so the
//! player's buffer pool is not refilled. Passing `recycle` hands every picture
//! back instead, which is the pooled path a screen that reuses buffers gets.
//!
//! `reels` is the swipe loop: the next source opens while the old player is
//! still alive, the old one is dropped, the new one plays until its first
//! picture and then holds the screen for `hold` milliseconds.
//!
//! Every line is `key=value`, so a run can be diffed against another. The CPU
//! numbers come from the process itself (`GetProcessTimes` on Windows,
//! `/proc/self/stat` on Linux), so they cover the decoder, the conversion
//! workers and this probe together — not a GPU compositor running in another
//! process.

use std::{
    process,
    time::{Duration, Instant},
};

use cranpose_video::{VideoPlayer, VideoState};

/// How often the simulated screen asks for a picture. A real screen draws at
/// 60 Hz; polling slower than that would hide a player that publishes.
const TICK: Duration = Duration::from_millis(4);

/// CPU time this process has used so far, kernel plus user.
#[cfg(target_os = "windows")]
fn process_cpu_time() -> Option<Duration> {
    use windows::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the call only fills the four locals it is given, and the
    // pseudo-handle is valid for the whole life of the process.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
        .ok()?;
    }
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    // FILETIME counts 100 ns units.
    Some(Duration::from_nanos(
        (ticks(kernel) + ticks(user)).saturating_mul(100),
    ))
}

/// CPU time this process has used so far, kernel plus user.
#[cfg(target_os = "linux")]
fn process_cpu_time() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // The command name may contain spaces and parentheses, so count fields
    // from the last ')'.
    let tail = stat.rsplit(") ").next()?;
    let fields: Vec<&str> = tail.split_whitespace().collect();
    let ticks = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    // The kernel's clock tick is 10 ms on every Linux this probe targets.
    Some(Duration::from_millis(ticks.saturating_mul(10)))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn process_cpu_time() -> Option<Duration> {
    None
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, |count| count.get())
}

fn short_name(source: &str) -> &str {
    source.rsplit(['/', '\\']).next().unwrap_or(source)
}

fn cpu_line(label: &str, elapsed: Duration, cpu: Option<Duration>) {
    let Some(cpu) = cpu else {
        println!("{label}_cpu=unavailable");
        return;
    };
    let wall = elapsed.as_secs_f64();
    let used = cpu.as_secs_f64();
    if wall <= 0.0 {
        return;
    }
    let one_core = used / wall * 100.0;
    println!(
        "{label}_cpu_ms={:.1} {label}_cpu_pct_one_core={one_core:.1} \
         {label}_cpu_pct_machine={:.1}",
        millis(cpu),
        one_core / cores() as f64
    );
}

/// One player, driven the way a screen drives it.
fn play(source: &str, seconds: u64, recycle: bool) {
    let machine = cores();
    println!("mode=play source={source} cores={machine} recycle={recycle}");

    let cpu_before = process_cpu_time();
    let began = Instant::now();
    let player = match VideoPlayer::open(source) {
        Ok(player) => player,
        Err(error) => {
            println!("open_failed={error:?}");
            return;
        }
    };
    let open = began.elapsed();
    let info = player.info();
    println!(
        "info={}x{} fps={:?} live={} seekable={}",
        info.width,
        info.height,
        info.frame_rate,
        player.is_live(),
        player.is_seekable()
    );
    println!("open_ms={:.1}", millis(open));

    let published_before = player.diagnostics().frames_published();
    let play_began = Instant::now();
    if let Err(error) = player.play() {
        println!("play_failed={error:?}");
        return;
    }
    let span = Duration::from_secs(seconds);
    let mut first = None;
    let mut taken = 0u64;
    let mut outcome = String::from("ran");
    while play_began.elapsed() < span {
        if let Some(frame) = player.take_frame() {
            taken += 1;
            first.get_or_insert(play_began.elapsed());
            if recycle {
                player.recycle(frame);
            }
        }
        std::thread::sleep(TICK);
        match player.state() {
            VideoState::Failed(error) => {
                outcome = format!("failed {error}");
                break;
            }
            VideoState::Ended => {
                outcome = String::from("ended");
                break;
            }
            _ => {}
        }
    }
    let elapsed = play_began.elapsed();
    let cpu = process_cpu_time()
        .zip(cpu_before)
        .map(|(after, before)| after.saturating_sub(before));
    let diagnostics = player.diagnostics();
    let published = diagnostics
        .frames_published()
        .saturating_sub(published_before);
    println!(
        "first_picture_ms={}",
        first.map_or_else(|| String::from("none"), |at| format!("{:.1}", millis(at)))
    );
    println!(
        "seconds={:.2} published={published} taken={taken} fps={:.2} outcome={outcome}",
        elapsed.as_secs_f64(),
        published as f64 / elapsed.as_secs_f64()
    );
    println!(
        "decode_us={:.0} convert_us={:.0} reanchors={} errors={} layout={:?}",
        diagnostics.decode_time().as_secs_f64() * 1e6,
        diagnostics.convert_time().as_secs_f64() * 1e6,
        diagnostics.reanchors(),
        diagnostics.errors(),
        diagnostics.pixel_format()
    );
    cpu_line("play", elapsed, cpu);
    if let Some(cpu) = cpu
        && cpu.as_secs_f64() > 0.0
    {
        println!(
            "pictures_per_cpu_s={:.2} cpu_ms_per_picture={:.2}",
            published as f64 / cpu.as_secs_f64(),
            millis(cpu) / published.max(1) as f64
        );
    }
}

/// The swipe loop of a reels feed.
fn reels(hold: Duration, rounds: usize, sources: &[String]) {
    let machine = cores();
    println!(
        "mode=reels cores={machine} rounds={rounds} sources={} hold_ms={:.0}",
        sources.len(),
        millis(hold)
    );

    let cpu_before = process_cpu_time();
    let began = Instant::now();
    let mut current: Option<VideoPlayer> = None;
    let mut switches: Vec<(String, Duration, Option<Duration>)> = Vec::new();
    let mut failures = 0usize;
    let mut taken = 0u64;
    for round in 0..rounds {
        for source in sources {
            let switch_began = Instant::now();
            // Open first, drop second: the two players overlap briefly, as they
            // do when a feed builds the next clip before removing the old one.
            let next = VideoPlayer::open(source.as_str());
            drop(current.take());
            let player = match next {
                Ok(player) => player,
                Err(error) => {
                    println!(
                        "round={round} source={} open_failed={error:?}",
                        short_name(source)
                    );
                    failures += 1;
                    continue;
                }
            };
            if let Err(error) = player.play() {
                println!(
                    "round={round} source={} play_failed={error:?}",
                    short_name(source)
                );
                failures += 1;
                continue;
            }
            let mut first = None;
            let mut frames = 0u64;
            while switch_began.elapsed() < hold {
                // The picture is dropped, not recycled: that is what the
                // ready-made widget does, because the pixels move into its own
                // bitmap and never come back to the player.
                if player.take_frame().is_some() {
                    frames += 1;
                    taken += 1;
                    first.get_or_insert(switch_began.elapsed());
                }
                std::thread::sleep(TICK);
            }
            println!(
                "round={round} source={} switch_ms={:.1} first_picture_ms={} pictures={frames}",
                short_name(source),
                millis(switch_began.elapsed()),
                first.map_or_else(|| String::from("none"), |at| format!("{:.1}", millis(at)))
            );
            switches.push((short_name(source).to_owned(), switch_began.elapsed(), first));
            current = Some(player);
        }
    }
    drop(current);
    let elapsed = began.elapsed();
    let cpu = process_cpu_time()
        .zip(cpu_before)
        .map(|(after, before)| after.saturating_sub(before));

    println!(
        "switches={} failures={failures} pictures={taken} seconds={:.2}",
        switches.len(),
        elapsed.as_secs_f64()
    );
    if !switches.is_empty() {
        let worst = switches
            .iter()
            .max_by_key(|(_, switch, _)| *switch)
            .expect("a non-empty list has a maximum");
        let mean = switches
            .iter()
            .map(|(_, switch, _)| *switch)
            .sum::<Duration>()
            / switches.len() as u32;
        let slowest_first = switches
            .iter()
            .filter_map(|(_, _, first)| *first)
            .max()
            .unwrap_or_default();
        println!(
            "switch_mean_ms={:.1} switch_worst_ms={:.1} worst_source={} slowest_first_picture_ms={:.1}",
            millis(mean),
            millis(worst.1),
            worst.0,
            millis(slowest_first)
        );
    }
    cpu_line("reels", elapsed, cpu);
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    match mode.as_str() {
        "play" => {
            let source = std::env::args()
                .nth(2)
                .expect("pass a path or URL after `play`");
            let seconds: u64 = std::env::args()
                .nth(3)
                .and_then(|value| value.parse().ok())
                .unwrap_or(8);
            let recycle = std::env::args().nth(4).as_deref() == Some("recycle");
            play(&source, seconds, recycle);
        }
        "reels" => {
            let hold: u64 = std::env::args()
                .nth(2)
                .and_then(|value| value.parse().ok())
                .unwrap_or(700);
            let rounds: usize = std::env::args()
                .nth(3)
                .and_then(|value| value.parse().ok())
                .unwrap_or(3);
            let sources: Vec<String> = std::env::args().skip(4).collect();
            assert!(
                !sources.is_empty(),
                "pass at least one source after `reels <hold-ms> <rounds>`"
            );
            reels(Duration::from_millis(hold), rounds, &sources);
        }
        _ => {
            eprintln!(
                "usage:\n  perf_probe play <source> [seconds] [recycle]\n  \
                 perf_probe reels <hold-ms> <rounds> <source>...\n\
                 pid={} platform={} decoder={}",
                process::id(),
                cranpose_video::platform().name(),
                cranpose_video::decoder_name()
            );
        }
    }
}
