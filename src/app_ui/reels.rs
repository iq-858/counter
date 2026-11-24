//! Reels section: a vertical pager with strict active-page playback gating.
//!
//! Only the settled page composes a `VideoPlayer`. Every other page composes a
//! static poster, so there is never a second decoder open and no unconditional
//! autoplay of mounted pages. The active page drives one player directly,
//! which gives us both `take_frame()` for the picture and `decoded_size()` for
//! the live resolution readout.
//!
//! [`ActiveReel`] and [`InactiveReel`] are shared with the Live section so both
//! pagers use the exact same one-decoder-at-a-time gate. The two sections live
//! in separate arms of the root `match`, so only one pager (and therefore at
//! most one decoder) is composed at any time.

use std::{rc::Rc, time::Duration};

use cranpose::LocalWindowState;
use cranpose::prelude::*;
use cranpose::liquid::prelude::*;
use cranpose_video::{VideoError, VideoPlayer, VideoState};

use super::{theme, widgets::small_button};
use crate::app_model::{self, Clip, ClipKind};

/// Height of the pager inside the window. Shared with the Live section so both
/// pagers lay out identically.
pub(crate) const PAGER_HEIGHT: f32 = 418.0;

/// The pager height for the current window. On a phone the fixed 418 would
/// overflow a short viewport, so the pager takes a fraction of the content
/// height once the window reports its size. Falls back to the desktop constant
/// before the first layout. Reading `LocalWindowState` subscribes the caller,
/// so a rotation or resize relayouts the pager.
#[composable]
pub(crate) fn pager_height() -> f32 {
    let window = LocalWindowState::current();
    match window.and_then(|state| {
        let size = state.size();
        (size.width > 0.0 && size.height > 0.0).then_some(size.height)
    }) {
        Some(height) => (height * 0.62).max(200.0),
        None => PAGER_HEIGHT,
    }
}

/// The picture height for the current window, derived from the same viewport
/// as [`pager_height`] so the two stay in step on small screens.
#[composable]
fn picture_height() -> f32 {
    (pager_height() * 0.54).max(120.0)
}
/// How often the live decoded size is polled.
const RESOLUTION_POLL: Duration = Duration::from_millis(200);
/// Seek distance of the small transport buttons.
const SEEK_STEP: Duration = Duration::from_secs(5);

/// The Reels pager itself.
#[composable]
pub fn ReelsSection(initial_page: usize) {
    let state = rememberPagerState(initial_page);
    let settled = state.settled_page();
    let page_count = app_model::CLIPS.len();
    crate::app_diag::info(
        "reels.gate",
        format!("settled page {settled} of {page_count}"),
    );

    Column(
        Modifier::empty().fill_max_size(),
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
        {
            move || {
                Text(
                    page_label(settled, page_count),
                    Modifier::empty(),
                    theme::heading_style(),
                );
                Text(
                    "Scroll to change clips. Only the settled page opens a decoder; other pages are posters.",
                    Modifier::empty(),
                    theme::muted_style(),
                );
                VerticalPager(
                    Modifier::empty()
                        .fill_max_width()
                        .height(pager_height())
                        .background(theme::SURFACE),
                    state,
                    PagerSpec::default(),
                    page_count,
                    move |page| {
                        let clip = app_model::CLIPS[page];
                        if app_model::active_page(settled, page) {
                            ActiveReel(clip);
                        } else {
                            InactiveReel(clip, page);
                        }
                    },
                );
            }
        },
    );
}

/// Preview of the liquid-glass control design, using Cranpose's OWN liquid
/// runtime (`cranpose::liquid::prelude::*`) exactly like the official
/// `LiquidUiTab` — this is the pattern that runs without the wgpu Surface panic.
#[composable]
pub(crate) fn LiquidPreviewSection() {
    let clip = app_model::CLIPS[0];
    let source = app_model::resolved_source(&clip);
    let playing = remember(|| mutableStateOf(true)).with(|s| *s);
    let muted = remember(|| mutableStateOf(false)).with(|s| *s);

    LiquidTheme(LiquidThemeSpec::system(SchemeMode::Dark), move || {
        Column(
            Modifier::empty().fill_max_size().padding(12.0),
            ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
            {
                let playing = playing;
                let muted = muted;
                move || {
                    Text(
                        "Liquid-glass controls — تصميم Cranpose الأصلي",
                        Modifier::empty(),
                        theme::heading_style(),
                    );
                    // ISOLATION: no Video here for now — glass buttons only, to
                    // verify whether the wgpu adapter-lock panic comes from the
                    // video player being dropped while shader warm-up threads run.
                    let _ = &source;

                    // Glass transport buttons from Cranpose's own liquid runtime.
                    let p = playing;
                    let m = muted;
                    GlassIconButtonGroup(
                        Modifier::empty(),
                        GlassIconButtonGroupSpec::default(),
                        move |scope| {
                            let play = p;
                            scope.action(icons::CHECK, "Play / pause", move || {
                                play.set(!play.get());
                            });
                            let mute = m;
                            scope.action(icons::SETTINGS, "Mute", move || {
                                mute.set(!mute.get());
                            });
                        },
                    );
                }
            },
        );
    });
}

/// A human-readable page indicator, kept pure for unit testing.
fn page_label(settled: usize, page_count: usize) -> String {
    format!(
        "REELS · page {} of {}",
        settled.saturating_add(1),
        page_count
    )
}

/// The one page that owns playback. Also used by the Live section.
#[composable]
pub(crate) fn ActiveReel(clip: Clip) {
    let source = app_model::resolved_source(&clip);
    // Key the player on its source: a page slot reused for a different clip
    // drops the old player (and its decode threads) and opens the new one.
    let kind = clip.kind;
    let opened: Rc<Result<VideoPlayer, VideoError>> = rememberKeyed(source.clone(), move |key| {
        Rc::new(if kind == ClipKind::Hls {
            VideoPlayer::open_hls(key.to_string())
        } else {
            VideoPlayer::open(key.to_string())
        })
    });

    // `ImageBitmap` is Clone but not PartialEq, so every published frame must
    // count as a change; the never-equal hook is made for exactly this case.
    let picture = rememberMutableStateOfNeverEqual(|| None::<ImageBitmap>);
    let resolution = rememberMutableStateOf(|| None::<(u32, u32)>);

    // One frame pump: clear the previous source's picture -> open -> play ->
    // take the newest frame on every window frame. The state is not keyed, so
    // without the clear a slow or failed new source would keep showing the old
    // clip's last frame until its own first picture arrived.
    let pump_player = opened.clone();
    let pump_picture = picture.clone();
    let pump_resolution = resolution.clone();
    let pump_key = source.clone();
    let pump_log_key = source.clone();
    LaunchedEffect(pump_key, async move |scope| {
        pump_picture.set(None);
        pump_resolution.set(None);
        let Ok(player) = pump_player.as_ref() else {
            if let Err(error) = pump_player.as_ref() {
                crate::app_diag::error(
                    "reels.open",
                    format!("{}: decoder open failed: {error}", pump_log_key),
                );
            }
            return;
        };
        crate::app_diag::info("reels.open", format!("{}: player ready", pump_log_key));
        if let Err(error) = player.play() {
            crate::app_diag::error("reels.play", format!("{}: {error}", pump_log_key));
        }
        let mut failed_logged = false;
        loop {
            scope.with_frame_nanos(|_| ()).await;
            // A source-keyed player is normally closed by the effect being
            // cancelled, but guard against any other close path so the pump
            // does not poll a dead handle for the life of the screen.
            if player.is_closed() {
                crate::app_diag::warn(
                    "reels.pump",
                    format!("{}: player closed while its pump was live", pump_log_key),
                );
                break;
            }
            // Surface a decoder failure into the on-screen error view even when
            // no diagnostic sink message accompanied it. Log once per failure.
            if let VideoState::Failed(ref reason) = player.state() {
                if !failed_logged {
                    crate::app_diag::error(
                        "reels.state",
                        format!("{}: decoder failed: {}", pump_log_key, reason),
                    );
                    failed_logged = true;
                }
            } else {
                failed_logged = false;
            }
            if let Some(frame) = player.take_frame() {
                let (width, height) = (frame.width(), frame.height());
                match ImageBitmap::from_rgba8(width, height, frame.into_rgba()) {
                    Ok(image) => pump_picture.set(Some(image)),
                    Err(error) => crate::app_diag::error(
                        "reels.frame",
                        format!(
                            "{}: {}x{} bitmap failed: {error}",
                            pump_log_key, width, height
                        ),
                    ),
                }
            }
        }
    });

    // `decoded_size()` is a cheap cached read that does not consume a frame, so
    // polling it shows dynamic resolution changes on adaptive streams.
    let res_player = opened.clone();
    let res_state = resolution.clone();
    let res_key = source.clone();
    let res_log_key = source.clone();
    LaunchedEffect(res_key, async move |_scope| {
        let Ok(player) = res_player.as_ref() else {
            return;
        };
        loop {
            // Mirror the frame pump: a player closed by any path other than a
            // source-key change must not be polled for the life of the screen.
            if player.is_closed() {
                crate::app_diag::warn(
                    "reels.resolution",
                    format!("{}: player closed while polling size", res_log_key),
                );
                break;
            }
            res_state.set(player.decoded_size());
            delay(RESOLUTION_POLL).await;
        }
    });

    let failure = match opened.as_ref() {
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    };

    Column(
        Modifier::empty().fill_max_size().padding(12.0),
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
        {
            let failure = failure.clone();
            let shown_picture = picture.clone();
            let shown_resolution = resolution.clone();
            let player_ref = opened.clone();
            let clip = clip;
            move || {
                match failure.clone() {
                    Some(error) => Text(
                        format!("could not open decoder: {error}"),
                        Modifier::empty().padding(12.0),
                        theme::fail_style(),
                    ),
                    None => match shown_picture.get() {
                        Some(image) => Image(
                            image,
                            None,
                            Modifier::empty().fill_max_width().height(picture_height()),
                            Alignment::CENTER,
                            ContentScale::Fit,
                            1.0,
                            None,
                        ),
                        None => Box(
                            Modifier::empty()
                                .fill_max_width()
                                .height(picture_height())
                                .background(theme::SURFACE_RAISED),
                            BoxSpec::default(),
                            || {},
                        ),
                    },
                };

                Text(clip.title, Modifier::empty(), theme::heading_style());
                Text(clip.author, Modifier::empty(), theme::muted_style());
                Text(
                    format!("{} · {}", clip.format, clip.note),
                    Modifier::empty(),
                    theme::muted_style(),
                );
                let status = match player_ref.as_ref() {
                    Ok(player) => format!(
                        "{} · {}",
                        app_model::format_resolution(shown_resolution.get()),
                        player_status(player)
                    ),
                    Err(error) => format!("error: {error}"),
                };
                Text(status, Modifier::empty(), theme::accent_style());

                let toggle = player_ref.clone();
                let back = player_ref.clone();
                let forward = player_ref.clone();
                let restart = player_ref.clone();
                let speed = player_ref.clone();
                Row(
                    Modifier::empty(),
                    RowSpec::new().horizontal_arrangement(LinearArrangement::SpacedBy(6.0)),
                    move || {
                        small_button("play / pause", {
                            let player = toggle.clone();
                            move || {
                                if let Ok(p) = player.as_ref() {
                                    let _ = p.toggle();
                                }
                            }
                        });
                        small_button("-5 s", {
                            let player = back.clone();
                            move || {
                                if let Ok(p) = player.as_ref() {
                                    let _ = p.seek(p.position().saturating_sub(SEEK_STEP));
                                }
                            }
                        });
                        small_button("+5 s", {
                            let player = forward.clone();
                            move || {
                                if let Ok(p) = player.as_ref() {
                                    let _ = p.seek(p.position() + SEEK_STEP);
                                }
                            }
                        });
                        small_button("restart", {
                            let player = restart.clone();
                            move || {
                                if let Ok(p) = player.as_ref() {
                                    let _ = p.seek(Duration::ZERO);
                                    let _ = p.play();
                                }
                            }
                        });
                        small_button("speed", {
                            let player = speed.clone();
                            move || {
                                if let Ok(p) = player.as_ref() {
                                    let next = if p.speed() < 0.75 {
                                        1.0
                                    } else if p.speed() < 1.5 {
                                        2.0
                                    } else {
                                        0.5
                                    };
                                    let _ = p.set_speed(next);
                                }
                            }
                        });
                    },
                );
            }
        },
    );
}

/// A page with no player at all: a cheap poster and an honest status line.
/// Also used by the Live section.
#[composable]
pub(crate) fn InactiveReel(clip: Clip, page: usize) {
    Column(
        Modifier::empty().fill_max_size().padding(12.0),
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
        move || {
            Box(
                Modifier::empty()
                    .fill_max_width()
                    .height(picture_height())
                    .background(theme::SURFACE_RAISED),
                BoxSpec::default(),
                || {},
            );
            Text(
                format!("page {} · {}", page + 1, clip.title),
                Modifier::empty(),
                theme::heading_style(),
            );
            Text(clip.author, Modifier::empty(), theme::muted_style());
            Text(
                format!("{} · {}", clip.format, clip.note),
                Modifier::empty(),
                theme::muted_style(),
            );
            Text(
                "PAUSED — not the settled page; no player is open.",
                Modifier::empty(),
                theme::accent_style(),
            );
        },
    );
}

/// Where the clock stands, how long the media runs, and what the player does.
fn player_status(player: &VideoPlayer) -> String {
    let duration = player.duration().map_or_else(|| "live".to_owned(), clock);
    let state = match player.state() {
        VideoState::Idle => "idle",
        VideoState::Loading => "loading",
        VideoState::Playing => "playing",
        VideoState::Paused => "paused",
        VideoState::Ended => "ended",
        VideoState::Failed(_) => "failed",
    };
    format!(
        "{} / {} · {state} · {:.2}x",
        clock(player.position()),
        duration,
        player.speed()
    )
}

/// A position as a clock reads it.
fn clock(position: Duration) -> String {
    let minutes = position.as_secs() / 60;
    let seconds = position.as_secs() % 60;
    format!("{minutes}:{seconds:02}.{}", position.subsec_millis() / 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_label_is_one_based_and_handles_empty_catalogues() {
        assert_eq!(page_label(0, 6), "REELS · page 1 of 6");
        assert_eq!(page_label(4, 6), "REELS · page 5 of 6");
        assert_eq!(page_label(0, 0), "REELS · page 1 of 0");
    }

    #[test]
    fn clock_formats_minutes_seconds_and_tenths() {
        assert_eq!(clock(Duration::ZERO), "0:00.0");
        assert_eq!(clock(Duration::from_millis(3_250)), "0:03.2");
        assert_eq!(clock(Duration::from_secs(125)), "2:05.0");
    }
}
