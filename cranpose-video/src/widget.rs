//! The composables that show a video, and the handle that drives one.

use std::{rc::Rc, time::Duration};

use cranpose_core::{LaunchedEffect, rememberKeyed, rememberMutableStateOf};
use cranpose_ui::{
    Alignment, Button, ButtonSpec, Color, Column, ColumnSpec, ContentScale, Image, ImageBitmap,
    LinearArrangement, Modifier, Row, RowSpec, Spacer, Text, TextStyle, composable,
    widgets::{Box, BoxSpec, Slider, SliderScope, SliderSpec},
};

use cranpose_liquid::{
    GlassIconButton, GlassIconButtonGroup, GlassIconButtonGroupScope, GlassIconButtonGroupSpec,
    GlassButtonSpec,
};
use crate::{VideoError, VideoInfo, VideoPlayer, VideoSource, VideoState};

// Material Design media glyphs (24x24 viewBox, Apache-2.0), drawn as vectors by
// cranpose-ui's IconWith through the liquid glass icon buttons. Keeping them
// here avoids a hard dependency on a particular icon-pack version.
const ICON_PLAY: &str =
    "M8 5v14l11-7z";
#[allow(dead_code)]
const ICON_PAUSE: &str =
    "M6 19h4V5H6v14zm8-14v14h4V5h-4z";
#[allow(dead_code)]
const ICON_STOP: &str =
    "M6 6h12v12H6z";
#[allow(dead_code)]
const ICON_REPLAY: &str =
    "M12 5V1L7 6l5 5V7c3.31 0 6 2.69 6 6s-2.69 6-6 6-6-2.69-6-6H4c0 4.42 3.58 8 8 8s8-3.58 8-8-3.58-8-8-8z";
const ICON_SKIP_BACK: &str =
    "M11 18V6l-8.5 6 8.5 6zm.5-6l8.5 6V6l-8.5 6z";
const ICON_SKIP_FORWARD: &str =
    "M4 18l8.5-6L4 6v12zm9-12v12l8.5-6-8.5-6z";
#[allow(dead_code)]
const ICON_VOLUME: &str =
    "M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z";
const ICON_MUTE: &str =
    "M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3L3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4L9.91 6.09 12 8.18V4z";
const ICON_REPEAT: &str =
    "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z";

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

    /// Returns to the first frame and stops there.
    ///
    /// Surfaces `VideoPlayer::stop()`, which a live source rejects with
    /// `UnsupportedOperation`. Additive: callers that never used it are
    /// unaffected.
    pub fn stop(&self) -> Result<(), VideoError> {
        self.player()?.stop()
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

// --- Pro controls: additive, iOS-style transport widgets -------------------
// Every composable here takes a caller-owned handle and never opens a player.
// They follow docs/WIDGET-CONTROLS-DESIGN.md. Nothing existing changes shape.

/// One element of the extended control bar ([`VideoControlsPro`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProControl {
    /// Play / pause toggle.
    PlayPause,
    /// Seek back by the configured step.
    Back,
    /// Seek forward by the configured step.
    Forward,
    /// Return to the first frame and play.
    Restart,
    /// Stop and return to the first frame (hidden for live sources).
    Stop,
    /// Mute / unmute toggle.
    Mute,
    /// Volume slider.
    Volume,
    /// Loop on/off toggle.
    Loop,
    /// Playback speed cycle button.
    Speed,
}

/// Where the extended bar sits relative to the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GlassStyle {
    /// A floating translucent bar above the picture (the default).
    #[default]
    Floating,
    /// A bar below the picture, matching the built-in [`VideoControls`] layout.
    Inline,
}

/// Immutable configuration for [`VideoControlsPro`].
#[derive(Clone, PartialEq)]
pub struct VideoControlsOptions {
    /// Which controls to show, in order.
    pub elements: &'static [ProControl],
    /// How far the back/forward buttons move the clock.
    pub seek_step: Duration,
    /// Speeds the speed button cycles through.
    pub speeds: &'static [f32],
    /// Where the bar sits relative to the picture.
    pub style: GlassStyle,
    /// Whether to show the progress slider.
    pub show_progress: bool,
    /// Whether to show the status line.
    pub show_status: bool,
}

impl Default for VideoControlsOptions {
    fn default() -> Self {
        Self {
            elements: &[
                ProControl::PlayPause,
                ProControl::Back,
                ProControl::Forward,
                ProControl::Restart,
                ProControl::Stop,
                ProControl::Mute,
                ProControl::Volume,
                ProControl::Loop,
                ProControl::Speed,
            ],
            seek_step: SEEK_STEP,
            speeds: &[0.5, 1.0, 2.0],
            style: GlassStyle::Floating,
            show_progress: true,
            show_status: true,
        }
    }
}

impl VideoControlsOptions {
    /// Creates the default configuration.
    pub fn new() -> Self {
        Self::default()
    }
    /// Sets which controls appear, in order.
    pub fn elements(mut self, elements: &'static [ProControl]) -> Self {
        self.elements = elements;
        self
    }
    /// Sets the back/forward seek step.
    pub fn seek_step(mut self, step: Duration) -> Self {
        self.seek_step = step;
        self
    }
    /// Sets the speeds the speed button cycles through.
    pub fn speeds(mut self, speeds: &'static [f32]) -> Self {
        self.speeds = speeds;
        self
    }
    /// Sets the bar's position relative to the picture.
    pub fn style(mut self, style: GlassStyle) -> Self {
        self.style = style;
        self
    }
    /// Shows or hides the progress slider.
    pub fn show_progress(mut self, show: bool) -> Self {
        self.show_progress = show;
        self
    }
    /// Shows or hides the status line.
    pub fn show_status(mut self, show: bool) -> Self {
        self.show_status = show;
        self
    }
}

/// Mute / unmute button for a caller-owned handle.
#[composable]
pub fn VideoMuteButton(handle: VideoPlayerHandle, modifier: Modifier) {
    let muted = rememberMutableStateOf(|| handle.is_muted());
    let toggle = handle;
    let flag = muted.clone();
    Button(
        modifier.padding(2.0),
        ButtonSpec::new(),
        move || {
            let next = !flag.get();
            flag.set(next);
            let _ = toggle.set_muted(next);
        },
        move || {
            // Unicode glyphs stand in for SF Symbols: cranpose-ui has no vector
            // icon component, so a widely-supported glyph is the portable choice.
            let glyph = if muted.get() { "\u{1F507}" } else { "\u{1F50A}" }; // 🔇 / 🔊
            Text(glyph.to_owned(), Modifier::empty().padding(8.0), TextStyle::default());
        },
    );
}

/// Loop on/off button for a caller-owned handle.
#[composable]
pub fn VideoLoopButton(handle: VideoPlayerHandle, modifier: Modifier) {
    let looping = rememberMutableStateOf(|| handle.is_looping());
    let toggle = handle;
    let flag = looping.clone();
    Button(
        modifier.padding(2.0),
        ButtonSpec::new(),
        move || {
            let next = !flag.get();
            flag.set(next);
            let _ = toggle.set_looping(next);
        },
        move || {
            let glyph = if looping.get() { "\u{21BB}" } else { "\u{21BA}" }; // ↻ / ↺
            Text(glyph.to_owned(), Modifier::empty().padding(8.0), TextStyle::default());
        },
    );
}

/// Stop button for a caller-owned handle. Disabled for live sources.
#[composable]
pub fn VideoStopButton(handle: VideoPlayerHandle, modifier: Modifier) {
    let live = handle.duration().is_none();
    let stopper = handle;
    Button(
        modifier.padding(2.0),
        ButtonSpec::new(),
        move || {
            let _ = stopper.stop();
        },
        move || {
            // A square glyph reads as "stop" across platforms and scripts.
            let glyph = if live { "\u{25A0}·" } else { "\u{25A0}" }; // ■ / ■·
            Text(glyph.to_owned(), Modifier::empty().padding(8.0), TextStyle::default());
        },
    );
}

/// Cycles playback speed through `speeds` for a caller-owned handle.
#[composable]
pub fn VideoSpeedButton(
    handle: VideoPlayerHandle,
    modifier: Modifier,
    speeds: &'static [f32],
) {
    let index = rememberMutableStateOf(|| 0usize);
    let setter = handle;
    let cursor = index.clone();
    let values = speeds;
    Button(
        modifier.padding(2.0),
        ButtonSpec::new(),
        move || {
            if values.is_empty() {
                return;
            }
            let next = (cursor.get() + 1) % values.len();
            cursor.set(next);
            let _ = setter.set_speed(values[next]);
        },
        move || {
            let shown = if values.is_empty() {
                1.0
            } else {
                values[index.get() % values.len()]
            };
            Text(format!("{shown:.2}x"), Modifier::empty().padding(8.0), TextStyle::default());
        },
    );
}

/// Volume slider for a caller-owned handle, reusing the built-in slider.
#[composable]
pub fn VideoVolumeSlider(handle: VideoPlayerHandle, modifier: Modifier, spec: SliderSpec) {
    let level = rememberMutableStateOf(|| handle.volume());
    let setter = handle;
    let level_on_change = level.clone();
    let _ = Slider(
        modifier,
        level.get(),
        move |value: f32| {
            level_on_change.set(value.clamp(0.0, 1.0));
            let _ = setter.set_volume(value.clamp(0.0, 1.0));
        },
        move || {},
        spec,
        move |_scope: SliderScope| {},
    );
}

/// Extended transport bar in the iOS style, over a caller-owned handle.
///
/// Consumes the given handle only — it never calls `rememberVideoPlayer`, so
/// no extra decoder is created. See `docs/WIDGET-CONTROLS-DESIGN.md`.
#[composable]
pub fn VideoControlsPro(
    handle: VideoPlayerHandle,
    modifier: Modifier,
    options: VideoControlsOptions,
) {
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
            let opts = options;
            move || {
                let _ = status_tick.get();
                if opts.show_status {
                    Text(status(&controls), Modifier::empty(), TextStyle::default());
                }
                if opts.show_progress {
                    VideoProgressSlider(controls.clone(), Modifier::empty().fill_max_width());
                }
                Row(
                    Modifier::empty(),
                    RowSpec::new().horizontal_arrangement(LinearArrangement::SpacedBy(6.0)),
                    {
                        let bar = controls.clone();
                        let elements = opts.elements;
                        let step = opts.seek_step;
                        let speeds = opts.speeds;
                        move || {
                            for element in elements {
                                let player = bar.clone();
                                let seek_step = step;
                                match *element {
                                    ProControl::PlayPause => control("\u{25B6}".to_owned(), move || {
                                        let _ = player.toggle();
                                    }),
                                    ProControl::Back => control("\u{226A}".to_owned(), move || {
                                        let _ = player.seek(player.position().saturating_sub(seek_step));
                                    }),
                                    ProControl::Forward => control("\u{22A9}".to_owned(), move || {
                                        let _ = player.seek(player.position() + seek_step);
                                    }),
                                    ProControl::Restart => control("\u{21BA}".to_owned(), move || {
                                        let _ = player.seek(Duration::ZERO);
                                        let _ = player.play();
                                    }),
                                    ProControl::Stop => VideoStopButton(player, Modifier::empty()),
                                    ProControl::Mute => VideoMuteButton(player, Modifier::empty()),
                                    ProControl::Volume => VideoVolumeSlider(player, Modifier::empty(), SliderSpec::default()),
                                    ProControl::Loop => VideoLoopButton(player, Modifier::empty()),
                                    ProControl::Speed => VideoSpeedButton(player, Modifier::empty(), speeds),
                                }
                            }
                        }
                    },
                );
            }
        },
    );
}

/// Liquid-glass transport bar: vector icons, a real frosted backdrop and a
/// spring press animation — all from `cranpose-liquid`.
///
/// Additive and self-contained: it consumes a caller-owned handle and opens no
/// player. The backdrop blur and press scale come from the framework, so this
/// composable renders on every target the widget feature builds for.
#[composable]
pub fn VideoControlsLiquid(
    handle: VideoPlayerHandle,
    modifier: Modifier,
    options: VideoControlsOptions,
) {
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
            let bar = handle;
            let status_tick = tick;
            let opts = options;
            move || {
                let _ = status_tick.get();
                if opts.show_status {
                    Text(status(&bar), Modifier::empty(), TextStyle::default());
                }
                if opts.show_progress {
                    VideoProgressSlider(bar.clone(), Modifier::empty().fill_max_width());
                }
                let play = bar.clone();
                let back = bar.clone();
                let forward = bar.clone();
                let mute = bar.clone();
                let repeat = bar.clone();
                GlassIconButtonGroup(
                    Modifier::empty(),
                    GlassIconButtonGroupSpec::default(),
                    move |scope: &GlassIconButtonGroupScope| {
                        scope.action(ICON_PLAY, "Play or pause", move || {
                            let _ = play.toggle();
                        });
                        scope.action(ICON_SKIP_BACK, "Back 5 seconds", move || {
                            let _ = back.seek(back.position().saturating_sub(opts.seek_step));
                        });
                        scope.action(ICON_SKIP_FORWARD, "Forward 5 seconds", move || {
                            let _ = forward.seek(forward.position() + opts.seek_step);
                        });
                        scope.action(ICON_REPEAT, "Toggle loop", move || {
                            let _ = repeat.set_looping(!repeat.is_looping());
                        });
                        scope.action(ICON_MUTE, "Mute or unmute", move || {
                            let _ = mute.set_muted(!mute.is_muted());
                        });
                    },
                );
            }
        },
    );
}

/// One liquid-glass icon button with a press animation, for callers that want a
/// single control (for example a custom overlay) rather than the full bar.
#[composable]
pub fn VideoLiquidIconButton(
    handle: VideoPlayerHandle,
    modifier: Modifier,
    icon: &'static str,
    action: VideoLiquidAction,
) {
    let target = handle;
    // The frosted material and the spring press scale live inside GlassIconButton
    // (LiquidMotion::snappy()). No per-frame work runs on the composition thread.
    GlassIconButton(
        modifier,
        GlassButtonSpec::glass(),
        44.0,
        move || {
            let _ = match action {
                VideoLiquidAction::Toggle => target.toggle(),
                VideoLiquidAction::Stop => target.stop(),
                VideoLiquidAction::Replay => {
                    let _ = target.seek(Duration::ZERO);
                    target.play()
                }
                VideoLiquidAction::Mute => target.set_muted(!target.is_muted()),
                VideoLiquidAction::Loop => target.set_looping(!target.is_looping()),
            };
        },
        icon,
    );
}

/// The action a [`VideoLiquidIconButton`] performs when pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoLiquidAction {
    /// Toggle play/pause.
    Toggle,
    /// Stop and return to the first frame.
    Stop,
    /// Seek to the beginning and play.
    Replay,
    /// Toggle mute.
    Mute,
    /// Toggle looping.
    Loop,
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
