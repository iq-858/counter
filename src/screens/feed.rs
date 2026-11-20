use cranpose::prelude::*;
use cranpose::widgets::{BitmapPainter, ContentScale, Icon, Image};
use cranpose_services::media::{self, MediaItem, MediaMetadata};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::components::{action_rail, post_overlay, video_progress};
use crate::data::{Post, video_for};
use crate::theme::*;
use crate::video::{self, Decoder};

// Calmer: require a more deliberate drag before flipping a page.
const SWIPE_POSITION_THRESHOLD: f32 = 0.28;
const SWIPE_VELOCITY_THRESHOLD: f32 = 1000.0;
// Rubber-band factor when dragging past the first/last page.
const EDGE_RESISTANCE: f32 = 0.35;
// Double-tap window for like.
const DOUBLE_TAP_MILLIS: f64 = 300.0;
const DOUBLE_TAP_SLOP: f32 = 40.0;
const DOUBLE_TAP_LIKE_MILLIS: f64 = 650.0;
// Height of the bottom navigation bar that overlays the feed.
const NAV_BAR_H: f32 = 58.0;

#[derive(Clone)]
struct Pager {
    card_index: MutableState<i64>,
    drag_state: MutableState<i64>,
    drag_start_y: MutableState<f32>,
    velocity: MutableState<f32>,
    last_y: MutableState<f32>,
    last_time_ms: MutableState<f64>,
    page_h: MutableState<f32>,
    content_size: Cell<i64>,
    token: MutableState<u64>,
    // Double-tap / play-pause.
    last_tap_ms: MutableState<f64>,
    last_tap_y: MutableState<f32>,
    paused: MutableState<bool>,
}

const DRAG_IDLE: i64 = 0;
const DRAG_DRAGGING: i64 = 1;

/// Media bookkeeping shared with the frame ticker. It keeps audio and video
/// playback in sync with the current card and its play/pause state.
struct Player {
    current: Cell<Option<&'static str>>,
    paused: Cell<bool>,
    audio_available: Cell<bool>,
    decoder: RefCell<Option<Decoder>>,
}

impl Player {
    fn new() -> Self {
        Self {
            current: Cell::new(None),
            paused: Cell::new(false),
            audio_available: Cell::new(false),
            decoder: RefCell::new(None),
        }
    }

    fn sync(
        &self,
        name: &'static str,
        post: Option<&Post>,
        pager: &Pager,
    ) -> (bool, anyhow::Result<Option<video::VideoFrame>>) {
        if name.is_empty() {
            return (false, Ok(None));
        }
        let effective_paused = pager.paused.get();
        let changed = self.current.get() != Some(name);

        if changed {
            *self.decoder.borrow_mut() = None;
            match Decoder::open(name) {
                Ok(decoder) => *self.decoder.borrow_mut() = Some(decoder),
                Err(error) => {
                    self.current.set(Some(name));
                    self.audio_available.set(false);
                    media::pause_media();
                    return (true, Err(error));
                }
            }
            self.current.set(Some(name));

            if let Some(uri) = video::media_uri(name) {
                let mut item = MediaItem::new(uri);
                if let Some(post) = post {
                    item = item.with_metadata(MediaMetadata::titled(post.music));
                }
                match media::open_media(item) {
                    Ok(()) => {
                        self.audio_available.set(true);
                        media::set_media_looping(true);
                        self.paused.set(effective_paused);
                        if !effective_paused && let Err(error) = media::play_media() {
                            eprintln!("feed-sync[{name}]: audio playback failed: {error}");
                        }
                    }
                    Err(error) => {
                        eprintln!("feed-sync[{name}]: audio open failed: {error}");
                        self.audio_available.set(false);
                        media::pause_media();
                    }
                }
            } else {
                eprintln!("feed-sync[{name}]: video media URI could not be created");
                self.audio_available.set(false);
                media::pause_media();
            }
        } else if self.audio_available.get() && effective_paused && !self.paused.get() {
            media::pause_media();
            self.paused.set(true);
        } else if self.audio_available.get() && !effective_paused && self.paused.get() {
            if let Err(error) = media::play_media() {
                eprintln!("feed-sync[{name}]: audio playback failed: {error}");
            }
            self.paused.set(false);
        }

        let decoder = self.decoder.borrow();
        if let Some(decoder) = decoder.as_ref() {
            decoder.set_paused(effective_paused);
        }
        let frame = decoder
            .as_ref()
            .and_then(Decoder::try_frame)
            .transpose()
            .map_err(anyhow::Error::msg);
        (changed, frame)
    }

    fn audio_available(&self) -> bool {
        self.audio_available.get()
    }
}

impl Pager {
    fn new(content_size: i64) -> Self {
        Self {
            card_index: mutableStateOf(0_i64),
            drag_state: mutableStateOf(DRAG_IDLE),
            drag_start_y: mutableStateOf(0.0_f32),
            velocity: mutableStateOf(0.0_f32),
            last_y: mutableStateOf(0.0_f32),
            last_time_ms: mutableStateOf(0.0_f64),
            page_h: mutableStateOf(800.0_f32),
            content_size: Cell::new(content_size),
            token: mutableStateOf(0_u64),
            last_tap_ms: mutableStateOf(-1000.0_f64),
            last_tap_y: mutableStateOf(0.0_f32),
            paused: mutableStateOf(false),
        }
    }

    fn bump(&self) {
        self.token.update(|t| *t = t.wrapping_add(1));
    }
}

pub fn feed_screen(
    posts: Vec<Post>,
    on_like: impl Fn(usize) + 'static,
    on_comment: impl Fn(usize) + 'static,
    on_follow: impl Fn(usize) + 'static,
) -> NodeId {
    let posts = Rc::new(posts);
    let count = posts.len() as i64;
    let on_like = Rc::new(on_like);
    let on_comment = Rc::new(on_comment);
    let on_follow = Rc::new(on_follow);
    let video_enabled = !std::env::args().any(|a| a == "--no-video");

    let pager = rememberPager(count);

    let pager_input = pager.clone();
    let gesture = Modifier::empty().pointer_input("feed-pager", move |scope| {
        let pager = pager_input.clone();
        async move {
            scope
                .await_pointer_event_scope(|await_scope| async move {
                    loop {
                        let event = await_scope.await_pointer_event().await;
                        handle_pager_event(&pager, &event);
                    }
                })
                .await;
        }
    });

    // The heart burst and play/pause overlay need a per-frame ticker; this ticker
    // also pumps video/audio playback, manages play/pause, and republishes the
    // current track's progress fraction.
    let progress = rememberMutableStateOf(|| 0.0_f32);
    let video_frame = rememberMutableStateOf(|| None::<cranpose::ImageBitmap>);
    let video_error = rememberMutableStateOf(|| None::<String>);
    let pager_anim = pager.clone();
    let screen_posts = posts.clone();
    let screen_progress = progress.clone();
    let screen_video_frame = video_frame.clone();
    let screen_video_error = video_error.clone();
    let player = Rc::new(Player::new());
    LaunchedEffectAsync(0u64, move |scope| {
        let pager = pager_anim.clone();
        let screen_posts = screen_posts.clone();
        let screen_progress = screen_progress.clone();
        let screen_video_frame = screen_video_frame.clone();
        let screen_video_error = screen_video_error.clone();
        let player = player.clone();
        Box::pin(async move {
            let clock = scope.runtime().frame_clock();
            let mut prev_nanos: u64 = 0;
            loop {
                if !scope.is_active() {
                    break;
                }
                let now = clock.next_perpetual_frame().await;
                if !scope.is_active() {
                    break;
                }
                if prev_nanos == 0 {
                    prev_nanos = now;
                    continue;
                }
                let _dt_ms = (now.saturating_sub(prev_nanos)) as f64 / 1_000_000.0;
                prev_nanos = now;

                // No overlay animations; only video plays.

                // Media pipeline housekeeping: make sure the card under the
                // cursor owns the audio, and that play/pause tracks the user
                // toggle plus in-flight drags.
                let idx = pager.card_index.get().max(0) as usize;
                if video_enabled {
                    let name = screen_posts
                        .get(idx)
                        .map(|post| video_for(post))
                        .unwrap_or("");
                    let (changed, result) = player.sync(name, screen_posts.get(idx), &pager);
                    if changed {
                        screen_video_frame.set(None);
                        screen_video_error.set(None);
                        screen_progress.set(0.0);
                    }
                    match result {
                        Ok(Some(frame)) => match cranpose::ImageBitmap::from_rgba8(
                            frame.width,
                            frame.height,
                            frame.pixels,
                        ) {
                            Ok(bitmap) => screen_video_frame.set(Some(bitmap)),
                            Err(error) => screen_video_error.set(Some(error.to_string())),
                        },
                        Ok(None) => {}
                        Err(error) => {
                            eprintln!("feed-sync[{name}]: video playback failed: {error}");
                            screen_video_error.set(Some(error.to_string()));
                        }
                    }
                }

                if video_enabled {
                    if player.audio_available()
                        && let Some(frac) = media::playback_progress().fraction()
                    {
                        let prev = screen_progress.get();
                        if (prev - frac).abs() > 0.001 {
                            screen_progress.set(frac);
                        }
                    } else if screen_progress.get() != 0.0 {
                        screen_progress.set(0.0);
                    }
                }
            }
        })
    });

    let like_outer = on_like.clone();
    let cmt_outer = on_comment.clone();
    let follow_outer = on_follow.clone();
    let pager_layout = pager.clone();

    BoxWithConstraints(
        Modifier::empty().fill_max_size().then(gesture),
        move |scope| {
            let h = scope.constraints().max_height;
            if h.is_finite() && h > 1.0 {
                let prev = pager_layout.page_h.get();
                if (prev - h).abs() > 0.5 {
                    pager_layout.page_h.set(h);
                    pager_layout.drag_state.set(DRAG_IDLE);
                }
            }

            let idx = pager_layout.card_index.get() as usize;

            let posts_c = posts.clone();
            let like_c = like_outer.clone();
            let cmt_c = cmt_outer.clone();
            let follow_c = follow_outer.clone();
            let pager_pages = pager_layout.clone();
            let screen_progress_c = progress.clone();
            let screen_video_frame_c = video_frame.clone();
            let screen_video_error_c = video_error.clone();

            Box(
                Modifier::empty().fill_max_size().clip_to_bounds(),
                BoxSpec::new().content_alignment(Alignment::TOP_START),
                move || {
                    let posts_ac = posts_c.clone();
                    let like_ac = like_c.clone();
                    let cmt_ac = cmt_c.clone();
                    let follow_ac = follow_c.clone();
                    let pager_ac = pager_pages.clone();
                    let prog_ac = screen_progress_c.clone();
                    let frame = screen_video_frame_c.get();
                    let video_error = screen_video_error_c.get();
                    let video_ok = video_enabled;

                    if idx < posts_ac.len() {
                        let post = posts_ac[idx].clone();
                        let active = pager_ac.drag_state.get() == DRAG_IDLE;
                        let playing = active && !pager_ac.paused.get();
                        feed_page(
                            post,
                            active,
                            playing,
                            prog_ac.get(),
                            video_ok,
                            frame,
                            video_error,
                            like_ac.clone(),
                            cmt_ac.clone(),
                            follow_ac.clone(),
                            idx,
                        );
                    }
                },
            );
        },
    )
}

fn rememberPager(content_size: i64) -> Pager {
    remember(|| Pager::new(content_size)).with(|pager| pager.clone())
}

fn handle_pager_event(pager: &Pager, event: &PointerEvent) {
    let h = pager.page_h.get().max(1.0);
    let content = pager.content_size.get();
    match event.kind {
        PointerEventKind::Down => {
            if pager.drag_state.get() != DRAG_IDLE {
                return;
            }
            let now = event_time_ms(event);
            let y = event.position.y;
            let _is_double = now - pager.last_tap_ms.get() < DOUBLE_TAP_MILLIS
                && (y - pager.last_tap_y.get()).abs() < DOUBLE_TAP_SLOP
                && pager.last_tap_ms.get() > 0.0;
            pager.last_tap_ms.set(now);
            pager.last_tap_y.set(y);
            pager.drag_state.set(DRAG_DRAGGING);
            pager.drag_start_y.set(y);
            pager.last_y.set(y);
            pager.last_time_ms.set(now);
            pager.velocity.set(0.0);
            event.consume();
        }
        PointerEventKind::Move => {
            if pager.drag_state.get() != DRAG_DRAGGING {
                return;
            }
            let y = event.position.y;
            let t = event_time_ms(event);
            let dt = (t - pager.last_time_ms.get()).max(0.001);
            let dy = y - pager.last_y.get();
            pager.velocity.set(dy / dt as f32 * 1000.0);
            pager.last_y.set(y);
            pager.last_time_ms.set(t);
            event.consume();
        }
        PointerEventKind::Up => {
            if pager.drag_state.get() != DRAG_DRAGGING {
                return;
            }
            let dragged = pager.last_y.get() - pager.drag_start_y.get();
            let vel = pager.velocity.get();
            let idx = pager.card_index.get();

            // A tap (no meaningful movement) toggles play/pause instead of paging.
            if dragged.abs() < 6.0 && vel.abs() < 120.0 {
                pager.paused.update(|p| *p = !*p);
                pager.drag_state.set(DRAG_IDLE);
                pager.bump();
                event.consume();
                return;
            }

            let forward_met =
                dragged < -h * SWIPE_POSITION_THRESHOLD || vel < -SWIPE_VELOCITY_THRESHOLD;
            let backward_met =
                dragged > h * SWIPE_POSITION_THRESHOLD || vel > SWIPE_VELOCITY_THRESHOLD;

            let mut next = idx;
            if forward_met && idx < content - 1 {
                next = idx + 1;
            } else if backward_met && idx > 0 {
                next = idx - 1;
            }

            pager.card_index.set(next);
            pager.drag_state.set(DRAG_IDLE);
            pager.velocity.set(0.0);
            pager.bump();
            event.consume();
        }
        PointerEventKind::Cancel => {
            if pager.drag_state.get() == DRAG_DRAGGING {
                pager.drag_state.set(DRAG_IDLE);
                pager.velocity.set(0.0);
                pager.bump();
            }
        }
        _ => {}
    }
}

fn event_time_ms(event: &PointerEvent) -> f64 {
    if let Some(t) = event.time_ms {
        t as f64
    } else if let Some(n) = event.animation_time_nanos {
        n as f64 / 1_000_000.0
    } else {
        0.0
    }
}

fn feed_page(
    post: Post,
    active: bool,
    playing: bool,
    progress: f32,
    video_enabled: bool,
    video_frame: Option<cranpose::ImageBitmap>,
    video_error: Option<String>,
    on_like: Rc<dyn Fn(usize)>,
    on_comment: Rc<dyn Fn(usize)>,
    on_follow: Rc<dyn Fn(usize)>,
    _index: usize,
) -> NodeId {
    let id = post.id;

    Box(
        Modifier::empty().fill_max_size().clip_to_bounds(),
        BoxSpec::new(),
        move || {
            if video_enabled {
                let video_frame = video_frame.clone();
                let video_error = video_error.clone();
                Box(
                    Modifier::empty().fill_max_size().background(BG),
                    BoxSpec::new().content_alignment(Alignment::CENTER),
                    move || {
                        if let Some(frame) = video_frame.clone() {
                            Image(
                                BitmapPainter(frame),
                                Some("Video".to_owned()),
                                Modifier::empty().fill_max_size(),
                                Alignment::CENTER,
                                ContentScale::Crop,
                                1.0,
                                None,
                            );
                        } else if video_error.is_some() {
                            Text(
                                "Video could not be played",
                                Modifier::empty(),
                                ts(16.0, TEXT, FontWeight::BOLD),
                            );
                        } else {
                            Text(
                                "Loading video...",
                                Modifier::empty(),
                                ts(16.0, TEXT_DIM, FontWeight::BOLD),
                            );
                        }
                    },
                );
            }

            feed_tabs();

            let like = on_like.clone();
            let cmt = on_comment.clone();
            let follow = on_follow.clone();
            let post_a = post.clone();
            Box(
                Modifier::empty()
                    .fill_max_size()
                    .padding_each(12.0, 12.0, 12.0, 70.0),
                BoxSpec::new().content_alignment(Alignment::BOTTOM_END),
                move || {
                    let like = like.clone();
                    let cmt = cmt.clone();
                    let follow = follow.clone();
                    action_rail(
                        &post_a,
                        move || like(id),
                        move || cmt(id),
                        move || follow(id),
                    );
                },
            );

            let follow_o = on_follow.clone();
            post_overlay(&post, move || follow_o(id));

            Box(
                Modifier::empty()
                    .fill_max_size()
                    .padding_each(0.0, 0.0, 0.0, NAV_BAR_H),
                BoxSpec::new().content_alignment(Alignment::BOTTOM_CENTER),
                move || {
                    video_progress(progress);
                },
            );
        },
    )
}

fn feed_tabs() -> NodeId {
    Box(
        Modifier::empty().fill_max_width().height(44.0).padding(8.0),
        BoxSpec::new().content_alignment(Alignment::TOP_CENTER),
        || {
            Row(
                Modifier::empty(),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(18.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                || {
                    tab_label("Following", false);
                    tab_label("For You", true);
                },
            );
        },
    )
}

fn tab_label(text: &'static str, active: bool) -> NodeId {
    let color = if active { TEXT } else { TEXT_DIM };
    Box(
        Modifier::empty(),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Column(
                Modifier::empty(),
                ColumnSpec::new()
                    .horizontal_alignment(HorizontalAlignment::CenterHorizontally)
                    .vertical_arrangement(LinearArrangement::SpacedBy(4.0)),
                move || {
                    Text(text, Modifier::empty(), ts(15.0, color, FontWeight::BOLD));
                    Box(
                        Modifier::empty()
                            .width(if active { 20.0 } else { 0.0 })
                            .height(3.0)
                            .background(TEXT)
                            .rounded_corners(1.5),
                        BoxSpec::new(),
                        || {},
                    );
                },
            );
        },
    )
}

fn pause_indicator(t: f32) -> NodeId {
    // Pop in for the first 30%, hold, then fade for the rest.
    let alpha = if t < 0.3 {
        t / 0.3
    } else if t > 0.7 {
        (1.0 - t) / 0.3
    } else {
        1.0
    };
    let scale = 0.85 + 0.15 * (t.min(0.3) / 0.3);
    Box(
        Modifier::empty().fill_max_size(),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Box(
                Modifier::empty()
                    .width(84.0)
                    .height(84.0)
                    .scale(scale)
                    .background(BG.with_alpha(0.5 * alpha))
                    .rounded_corners(42.0),
                BoxSpec::new().content_alignment(Alignment::CENTER),
                move || {
                    Icon(crate::icons::PAUSE, None, 40.0, TEXT.with_alpha(alpha));
                },
            );
        },
    )
}
