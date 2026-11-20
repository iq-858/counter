//! The composables that show a video, and the handle that drives one.

use std::{rc::Rc, time::Duration};

use cranpose_core::{LaunchedEffect, rememberKeyed, rememberMutableStateOf};
use cranpose_ui::{
    Alignment, Button, ButtonSpec, Color, Column, ColumnSpec, ContentScale, Image, ImageBitmap,
    LinearArrangement, Modifier, Row, RowSpec, Spacer, Text, TextStyle, composable,
    widgets::{Box, BoxSpec, Slider, SliderScope, SliderSpec},
};

use crate::{VideoError, VideoInfo, VideoPlayer, VideoSource, VideoState};

/// How far the seek buttons of [`VideoWithControls`] move the clock.
const SEEK_STEP: Duration = Duration::from_secs(5);

/// Height of the ready-made progress track, in logical pixels.
const PROGRESS_TRACK_HEIGHT: f32 = 4.0;

/// Width and height of the ready-made progress thumb, in logical pixels.
const PROGRESS_THUMB: f32 = 12.0;

/// A video that a composable shows and drives.
///
/// [`rememberVideoPlayer`] makes one, and [`Video`] shows its picture. A screen
/// that wants controls of its own reads the handle and composes its own.
#[derive(Clone)]
pub struct VideoPlayerHandle {
    opened: Rc<Result<VideoPlayer, VideoError>>,
}

impl PartialEq for VideoPlayerHandle {
    /// Two handles are equal when they drive the same player.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.opened, &other.opened)
    }
}

impl VideoPlayerHandle {
    /// The player, or the failure that kept its source from opening.
    fn player(&self) -> Result<&VideoPlayer, VideoError> {
        match self.opened.as_ref() {
            Ok(player) => Ok(player),
            Err(error) => Err(error.clone()),
        }
    }

    /// Starts playback, or resumes it where it stopped.
    pub fn play(&self) -> Result<(), VideoError> {
        self.player()?.play()
    }

    /// Holds the picture where it is.
    pub fn pause(&self) -> Result<(), VideoError> {
        self.player()?.pause()
    }

    /// Stops a moving picture, and starts a still one.
    pub fn toggle(&self) -> Result<(), VideoError> {
        self.player()?.toggle()
    }

    /// Moves the clock to `position`.
    pub fn seek(&self, position: Duration) -> Result<(), VideoError> {
        self.player()?.seek(position)
    }

    /// Moves the clock to `fraction` of the media, from zero to one.
    pub fn seek_fraction(&self, fraction: f32) -> Result<(), VideoError> {
        self.player()?.seek_fraction(fraction)
    }

    /// Sets how fast the clock runs, where the decoder supports it.
    pub fn set_speed(&self, speed: f32) -> Result<(), VideoError> {
        self.player()?.set_speed(speed)
    }

    /// Repeats the source when it reaches its end.
    pub fn set_looping(&self, looping: bool) -> Result<(), VideoError> {
        self.player().map(|player| player.set_looping(looping))
    }

    /// Whether the source repeats when it reaches its end.
    pub fn is_looping(&self) -> bool {
        self.player().map_or(false, VideoPlayer::is_looping)
    }

    /// Silences audio a backend carries itself, and starts it again.
    ///
    /// On the web, call this after a person interacts with the page, or the
    /// browser may refuse to start sound.
    pub fn set_muted(&self, muted: bool) -> Result<(), VideoError> {
        self.player()?.set_muted(muted)
    }

    /// Whether audio a backend carries is currently silenced.
    pub fn is_muted(&self) -> bool {
        self.player().map_or(false, VideoPlayer::is_muted)
    }

    /// Sets the volume of audio a backend carries itself, from 0.0 to 1.0.
    ///
    /// Volume and mute are separate: unmute with
    /// [`set_muted`](Self::set_muted) to hear sound.
    pub fn set_volume(&self, volume: f32) -> Result<(), VideoError> {
        self.player()?.set_volume(volume)
    }

    /// The volume of audio a backend carries, from 0.0 to 1.0.
    pub fn volume(&self) -> f32 {
        self.player().map_or(1.0, VideoPlayer::volume)
    }

    /// Where the clock stands.
    pub fn position(&self) -> Duration {
        self.player().map_or(Duration::ZERO, VideoPlayer::position)
    }

    /// How long the media runs, when the container states it.
    pub fn duration(&self) -> Option<Duration> {
        self.player().ok().and_then(VideoPlayer::duration)
    }

    /// What the player is doing now.
    pub fn state(&self) -> VideoState {
        match self.opened.as_ref() {
            Ok(player) => player.state(),
            Err(error) => VideoState::Failed(error.clone()),
        }
    }

    /// How fast the clock runs.
    pub fn speed(&self) -> f32 {
        self.player().map_or(1.0, VideoPlayer::speed)
    }

    /// What the decoder learned when it opened the source.
    pub fn info(&self) -> VideoInfo {
        self.player()
            .map_or_else(|_| VideoInfo::default(), VideoPlayer::info)
    }

    /// The width and height of the picture currently being shown, in pixels.
    ///
    /// This follows the decoded frame, so an adaptive or rotated stream updates
    /// it mid-playback. Returns `None` until the first picture arrives.
    pub fn decoded_size(&self) -> Option<(u32, u32)> {
        self.player().ok().and_then(VideoPlayer::decoded_size)
    }
}

/// Opens `source` once, and keeps the player until the composable leaves.
///
/// This is the one call a screen needs. Everything else in this module builds
/// on it, and a screen with its own controls uses the handle directly.
#[composable]
pub fn rememberVideoPlayer(source: impl Into<VideoSource> + 'static) -> VideoPlayerHandle {
    let source = source.into();
    // Key the player on its source: a Reels page reuses this call site for a
    // different clip, and a position-keyed `remember` would keep showing the
    // old video. Changing the source drops the old player (closing its threads)
    // and opens the new one.
    let opened: Rc<Result<VideoPlayer, VideoError>> =
        rememberKeyed(source, |source| Rc::new(VideoPlayer::open(source.clone())));
    VideoPlayerHandle { opened }
}

/// Plays a video inside the box `modifier` gives it.
///
/// The player opens, decodes and draws on its own, on the media framework of
/// the platform the application runs on.
#[composable]
pub fn Video(source: impl Into<VideoSource> + 'static, modifier: Modifier) {
    picture(rememberVideoPlayer(source), modifier);
}

/// Plays a video above the buttons that drive it.
///
/// A screen that wants other controls composes [`Video`] beside them instead.
#[composable]
pub fn VideoWithControls(source: impl Into<VideoSource> + 'static, modifier: Modifier) {
    with_controls(rememberVideoPlayer(source), modifier);
}

/// Ready-made controls for a player a screen already owns.
///
/// Compose [`Video`] with this below it when the screen wants the stock
/// buttons and progress slider but keeps its own layout. A source with no
/// duration, such as a live stream, shows a disabled progress slider.
#[composable]
pub fn VideoControls(handle: VideoPlayerHandle, modifier: Modifier) {
    let rate = rememberMutableStateOf(|| handle.speed());
    // The clock, state and duration live in the player, not in composition
    // state. Reading them once leaves the status line frozen. A per-frame tick
    // re-evaluates them as the position, state and speed change.
    let tick = rememberMutableStateOf(|| 0u64);
    let tick_state = tick.clone();
    LaunchedEffect(handle.clone(), async move |scope| {
        loop {
            scope.with_frame_nanos(|_| ()).await;
            tick_state.set(tick_state.get().wrapping_add(1));
        }
    });
    Column(
        modifier,
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
        {
            let controls = handle;
            let status_tick = tick;
            move || {
                // Subscribe this composition to the tick so the status text is
                // rebuilt on every clock advance instead of once at open.
                let _ = status_tick.get();
                Text(status(&controls), Modifier::empty(), TextStyle::default());
                VideoProgressSlider(controls.clone(), Modifier::empty().fill_max_width());
                Row(
                    Modifier::empty(),
                    RowSpec::new().horizontal_arrangement(LinearArrangement::SpacedBy(6.0)),
                    {
                        let toggle = controls.clone();
                        let back = controls.clone();
                        let forward = controls.clone();
                        let restart = controls.clone();
                        let speed = controls.clone();
                        move || {
                            control("play / pause".to_owned(), {
                                let player = toggle.clone();
                                move || {
                                    let _ = player.toggle();
                                }
                            });
                            control("-5 s".to_owned(), {
                                let player = back.clone();
                                move || {
                                    let target = player.position().saturating_sub(SEEK_STEP);
                                    let _ = player.seek(target);
                                }
                            });
                            control("+5 s".to_owned(), {
                                let player = forward.clone();
                                move || {
                                    let _ = player.seek(player.position() + SEEK_STEP);
                                }
                            });
                            control("restart".to_owned(), {
                                let player = restart.clone();
                                move || {
                                    let _ = player.seek(Duration::ZERO);
                                    let _ = player.play();
                                }
                            });
                            control(format!("{:.2}x", rate.get()), {
                                let player = speed.clone();
                                move || {
                                    let next = if rate.get() < 0.75 {
                                        1.0
                                    } else if rate.get() < 1.5 {
                                        2.0
                                    } else {
                                        0.5
                                    };
                                    rate.set(next);
                                    let _ = player.set_speed(next);
                                }
                            });
                        }
                    },
                );
            }
        },
    );
}

/// A ready-made progress slider for a player a screen already owns.
///
/// Dragging seeks with [`VideoPlayerHandle::seek_fraction`]. While it is held,
/// playback updates do not move the thumb under the pointer.
#[composable]
pub fn VideoProgressSlider(handle: VideoPlayerHandle, modifier: Modifier) {
    // A container can state its duration after opening (a browser reads it
    // from metadata). Keep the enabled flag in state so the slider becomes
    // usable when that happens instead of staying disabled from the first
    // composition.
    let has_duration = rememberMutableStateOf(|| handle.duration().is_some());
    let duration_state = has_duration.clone();
    let duration_handle = handle.clone();
    LaunchedEffect(duration_handle.clone(), async move |scope| {
        loop {
            scope.with_frame_nanos(|_| ()).await;
            duration_state.set(duration_handle.duration().is_some());
        }
    });
    let spec = SliderSpec::new()
        .thumb_extent(PROGRESS_THUMB)
        .enabled(has_duration.get());
    VideoProgressSliderWith(handle, modifier, spec, |scope| {
        progress_visual(scope.value(), scope.track_extent(), scope.thumb_offset())
    });
}

/// A progress slider whose track and thumb are caller-supplied composables.
///
/// `content` receives the framework's [`SliderScope`], so a screen can draw a
/// chapter bar, buffering range, labels, or any other visual while this
/// composable owns the player value and the seek on release.
#[composable]
pub fn VideoProgressSliderWith(
    handle: VideoPlayerHandle,
    modifier: Modifier,
    spec: SliderSpec,
    content: impl FnMut(SliderScope) + 'static,
) {
    let progress =
        rememberMutableStateOf(|| progress_fraction_for(handle.position(), handle.duration()));
    let dragging = rememberMutableStateOf(|| false);
    let effect_handle = handle.clone();
    let effect_progress = progress.clone();
    let effect_dragging = dragging.clone();
    // Follow the current player: a new source restarts the thumb updates on
    // the reopened player instead of polling a handle that is no longer shown.
    let effect_key = effect_handle.clone();
    LaunchedEffect(effect_key, async move |scope| {
        loop {
            scope.with_frame_nanos(|_| ()).await;
            if !effect_dragging.get() {
                effect_progress.set(progress_fraction_for(
                    effect_handle.position(),
                    effect_handle.duration(),
                ));
            }
        }
    });
    let _ = Slider(
        modifier,
        progress.get(),
        {
            let progress = progress.clone();
            let dragging = dragging.clone();
            move |value| {
                dragging.set(true);
                progress.set(value.clamp(0.0, 1.0));
            }
        },
        {
            let handle = handle.clone();
            let progress = progress.clone();
            let dragging = dragging.clone();
            move || {
                dragging.set(false);
                let _ = handle.seek_fraction(progress.get());
            }
        },
        spec,
        content,
    );
}

/// Maps a clock position to `0.0..=1.0` of a known duration.
///
/// A missing or empty duration yields `0.0`, which is the safe value for a
/// live source and for a container that has not stated its length yet.
pub fn progress_fraction_for(position: Duration, duration: Option<Duration>) -> f32 {
    match duration {
        Some(duration) if duration > Duration::ZERO => {
            let fraction = position.as_secs_f64() / duration.as_secs_f64();
            fraction.clamp(0.0, 1.0) as f32
        }
        _ => 0.0,
    }
}

/// The stock track, played portion and thumb of [`VideoProgressSlider`].
#[composable]
fn progress_visual(value: f32, track_extent: f32, thumb_offset: f32) {
    Box(
        Modifier::empty()
            .fill_max_width()
            .height(PROGRESS_TRACK_HEIGHT)
            .background(Color(0.24, 0.26, 0.30, 0.90)),
        BoxSpec::default(),
        || {},
    );
    Box(
        Modifier::empty()
            .height(PROGRESS_TRACK_HEIGHT)
            .width(track_extent * value)
            .background(Color(0.36, 0.62, 0.98, 1.0)),
        BoxSpec::default(),
        || {},
    );
    Box(
        Modifier::empty()
            .width(PROGRESS_THUMB)
            .height(PROGRESS_THUMB)
            .offset(thumb_offset, (PROGRESS_TRACK_HEIGHT - PROGRESS_THUMB) * 0.5)
            .background(Color(0.92, 0.95, 1.0, 1.0)),
        BoxSpec::default(),
        || {},
    );
}

/// The picture of one player, which takes a new frame on every frame of the
/// window.
#[composable]
fn picture(handle: VideoPlayerHandle, modifier: Modifier) {
    let picture = rememberMutableStateOf(|| None::<ImageBitmap>);
    let failure = handle.player().err();
    let frames = handle;
    // Key the frame pump on the handle so a new source stops the old loop and
    // starts one bound to the reopened player. A unit key would keep pumping
    // the previous player forever after a Reels page changes clip.
    let frames_key = frames.clone();
    let frame_picture = picture.clone();
    LaunchedEffect(frames_key, async move |scope| {
        let Ok(player) = frames.opened.as_ref() else {
            return;
        };
        // A new source restarts this effect. Clear the picture left by the old
        // one so its last frame is never shown as part of the new video.
        frame_picture.set(None);
        let _ = player.play();
        loop {
            scope.with_frame_nanos(|_| ()).await;
            if player.is_closed() {
                // The player is gone; polling it every frame would keep this
                // task alive for the life of the screen for no picture at all.
                break;
            }
            if let Some(frame) = player.take_frame() {
                let (width, height) = (frame.width(), frame.height());
                // The picture's bytes move into the bitmap, so the decoder and
                // the texture share one buffer.
                if let Ok(image) = ImageBitmap::from_rgba8(width, height, frame.into_rgba()) {
                    frame_picture.set(Some(image));
                }
            }
        }
    });
    match failure {
        Some(error) => Text(
            error.to_string(),
            modifier.padding(8.0),
            TextStyle::default(),
        ),
        None => match picture.get() {
            Some(image) => Image(
                image,
                None,
                modifier,
                Alignment::CENTER,
                ContentScale::Fit,
                1.0,
                None,
            ),
            None => Spacer(modifier),
        },
    };
}

/// The picture with the clock, progress and buttons that drive it.
#[composable]
fn with_controls(handle: VideoPlayerHandle, modifier: Modifier) {
    Column(
        modifier,
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
        {
            let controls = handle;
            move || {
                picture(controls.clone(), Modifier::empty().fill_max_width());
                VideoControls(controls.clone(), Modifier::empty());
            }
        },
    );
}

/// One button of the controls.
#[composable]
fn control(label: impl Into<String> + 'static, on_click: impl FnMut() + 'static) {
    let label = label.into();
    Button(
        Modifier::empty().padding(2.0),
        ButtonSpec::new(),
        on_click,
        move || {
            Text(
                label.clone(),
                Modifier::empty().padding(8.0),
                TextStyle::default(),
            );
        },
    );
}

/// Where the clock stands, how long the media runs, and what the player does.
fn status(handle: &VideoPlayerHandle) -> String {
    let duration = handle
        .duration()
        .map_or_else(|| String::from("live"), clock);
    let state = match handle.state() {
        VideoState::Idle => "idle",
        VideoState::Loading => "loading",
        VideoState::Playing => "playing",
        VideoState::Paused => "paused",
        VideoState::Ended => "ended",
        VideoState::Failed(_) => "failed",
    };
    format!(
        "{} / {duration} · {state} · {:.2}x",
        clock(handle.position()),
        handle.speed()
    )
}

/// The position as a clock reads it.
fn clock(position: Duration) -> String {
    let minutes = position.as_secs() / 60;
    let seconds = position.as_secs() % 60;
    format!("{minutes}:{seconds:02}.{}", position.subsec_millis() / 100)
}
