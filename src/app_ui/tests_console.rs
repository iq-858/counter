//! In-app decode test console.
//!
//! Runs a paced first-frame smoke test for every local clip. The sweep is
//! submitted with `launchBlocking`, so opening decoders and polling frames
//! happens on the shared blocking executor, never on the UI thread. There is
//! no network dependency: the HLS catalogue entry is excluded by
//! `decode_cases()`.

use std::time::{Duration, Instant};

use cranpose::prelude::*;
use cranpose_video::VideoPlayer;

use super::{theme, widgets::small_button};
use crate::app_model::{self, Clip};

/// How long one clip may take to produce its first decoded frame.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
/// Poll interval between `take_frame()` attempts inside the timeout loop.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The honest result of one clip's open + first-frame test.
#[derive(Clone, PartialEq)]
pub struct CaseResult {
    pub id: &'static str,
    pub title: String,
    pub passed: bool,
    pub elapsed: Duration,
    pub detail: String,
    pub size: Option<(u32, u32)>,
}

/// Runs one local clip: open, play, wait for a first frame, close.
///
/// Called only from the blocking executor. `take_frame()` pumps the decoder,
/// and the deadline bounds a clip that never publishes a picture.
fn run_case(clip: &Clip) -> CaseResult {
    let started = Instant::now();
    let source = app_model::resolved_source(clip);
    let mut player = match VideoPlayer::open(source) {
        Ok(player) => player,
        Err(error) => {
            return CaseResult {
                id: clip.id,
                title: clip.title.to_owned(),
                passed: false,
                elapsed: started.elapsed(),
                detail: format!("open failed: {error}"),
                size: None,
            };
        }
    };

    if let Err(error) = player.play() {
        player.close();
        return CaseResult {
            id: clip.id,
            title: clip.title.to_owned(),
            passed: false,
            elapsed: started.elapsed(),
            detail: format!("play failed: {error}"),
            size: None,
        };
    }

    let deadline = Instant::now() + FIRST_FRAME_TIMEOUT;
    let mut first_frame = None;
    while Instant::now() < deadline {
        if let Some(frame) = player.take_frame() {
            first_frame = Some(frame);
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    let (passed, detail, size) = match first_frame {
        Some(frame) => (
            true,
            "first frame decoded".to_owned(),
            Some((frame.width(), frame.height())),
        ),
        None => (
            false,
            format!(
                "no first frame within {} ms",
                FIRST_FRAME_TIMEOUT.as_millis()
            ),
            player.decoded_size(),
        ),
    };

    player.close();
    CaseResult {
        id: clip.id,
        title: clip.title.to_owned(),
        passed,
        elapsed: started.elapsed(),
        detail,
        size,
    }
}

/// Blocking work for the whole sweep. Capture-free, so it is `Send + 'static`.
fn run_sweep() -> Vec<CaseResult> {
    app_model::decode_cases().iter().map(run_case).collect()
}

/// Pure summary of a finished sweep.
fn summarize(rows: &[CaseResult]) -> (usize, usize, &'static str) {
    let passed = rows.iter().filter(|row| row.passed).count();
    let total = rows.len();
    (passed, total, app_model::verdict(passed, total))
}

/// The test console section.
#[composable]
pub fn TestsSection() {
    let results = rememberMutableStateOf(|| Vec::<CaseResult>::new());
    let running = rememberMutableStateOf(|| false);
    let message = rememberMutableStateOf(|| {
        String::from(
            "Idle. The sweep opens each offline clip on a background executor; nothing runs on the UI thread.",
        )
    });

    Column(
        Modifier::empty().fill_max_size().padding(12.0),
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
        {
            let results = results.clone();
            let running = running.clone();
            let message = message.clone();
            move || {
                Text(
                    "DECODE TESTS · paced first-frame sweep",
                    Modifier::empty(),
                    theme::heading_style(),
                );
                Text(
                    "Per-clip player open, play, and first-frame timeout. Local files only — no HLS, no network.",
                    Modifier::empty(),
                    theme::muted_style(),
                );

                let label = if running.get() {
                    "Running…"
                } else {
                    "Run paced decode stress"
                };
                let on_results = results.clone();
                let on_running = running.clone();
                let on_message = message.clone();
                small_button(label, move || {
                    if on_running.get() {
                        return;
                    }
                    on_running.set(true);
                    on_results.set(Vec::new());
                    on_message.set("Running local decode sweep…".to_owned());

                    let done_results = on_results.clone();
                    let done_running = on_running.clone();
                    let done_message = on_message.clone();
                    let launched = launchBlocking(run_sweep, move |outcome| {
                        done_running.set(false);
                        match outcome {
                            Ok(rows) => {
                                let (passed, total, verdict) = summarize(&rows);
                                done_message.set(format!(
                                    "{verdict} · {passed} of {total} local clips produced a first frame"
                                ));
                                done_results.set(rows);
                            }
                            Err(error) => {
                                done_message.set(format!("launch failed: {error:?}"));
                                done_results.set(Vec::new());
                            }
                        }
                    });
                    if let Err(error) = launched {
                        on_running.set(false);
                        on_message.set(format!("could not start work: {error:?}"));
                    }
                });

                Text(message.get(), Modifier::empty(), theme::accent_style());

                for row in results.get() {
                    Column(
                        Modifier::empty()
                            .fill_max_width()
                            .padding(8.0)
                            .background(theme::SURFACE),
                        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(2.0)),
                        {
                            let row = row;
                            move || {
                                Text(
                                    format!(
                                        "{} · {} · [{}] · {}",
                                        if row.passed { "PASS" } else { "FAIL" },
                                        row.title,
                                        row.id,
                                        app_model::format_elapsed(row.elapsed)
                                    ),
                                    Modifier::empty(),
                                    if row.passed {
                                        theme::pass_style()
                                    } else {
                                        theme::fail_style()
                                    },
                                );
                                Text(row.detail.clone(), Modifier::empty(), theme::muted_style());
                                Text(
                                    format!(
                                        "decoded size: {}",
                                        app_model::format_resolution(row.size)
                                    ),
                                    Modifier::empty(),
                                    theme::muted_style(),
                                );
                            }
                        },
                    );
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(passed: bool) -> CaseResult {
        CaseResult {
            id: "test",
            title: "Test".to_owned(),
            passed,
            elapsed: Duration::from_millis(12),
            detail: String::new(),
            size: None,
        }
    }

    #[test]
    fn summarize_handles_empty_partial_and_complete_runs() {
        assert_eq!(summarize(&[]), (0, 0, "NO DATA"));
        assert_eq!(summarize(&[row(true), row(false)]), (1, 2, "FAIL"));
        assert_eq!(summarize(&[row(true), row(true)]), (2, 2, "PASS"));
    }
}
