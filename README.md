# cranpose-video

Video playback for [Cranpose](https://crates.io/crates/cranpose) through each platform's own media stack.

This crate gives you two ways to play video:

1. **Engine layer** — `VideoPlayer`, a plain Rust API that opens a source, decodes frames and controls playback. It knows nothing about UI.
2. **UI layer** — `rememberVideoPlayer`, `VideoPlayerHandle` and `VideoFrame`, built on the `widget` feature, for use inside `#[composable]` functions.

Both layers use the same decoder per platform. The UI layer does not add a second player: a handle is a cloneable wrapper around one `VideoPlayer`, and the picture composable drives that same player's frame pump.

> Scope note: this repository contains this crate only. The crate has been **tested on real devices on Windows and iOS** by the maintainer. The CI matrix below is a separate compile + unit-test signal; a green or red CI job does not replace or negate that device testing.

---

## Requirements and versions

| Item | Value |
|---|---|
| Rust edition | 2024 (requires rustc **1.85+**) |
| Cranpose crates | `0.28.0` (caret: `>=0.28.0, <0.29.0`) — migration branch |
| Default features | `widget`, `player` |
| Optional system deps | GStreamer 1.0 dev packages for Linux; an Android NDK for Android cross-checks |

Cargo feature flags:

| Feature | What it adds |
|---|---|
| `widget` (default) | `rememberVideoPlayer`, `VideoPlayerHandle`, `Video`, `VideoFrame`, `VideoWithControls`, `VideoControls`, progress sliders. Depends on `cranpose`, `cranpose-core`, `cranpose-ui`. |
| `player` (default) | `widget` + `native-audio` + the standalone `cargo run` demo window. |
| `native-audio` | Plays the audio carried by a source on native platforms through the in-process audio backend. The browser owns sound itself. |
| `desktop` | The `cargo run` window shell and wgpu renderer. Kept out of `player` so mobile apps do not pull in the desktop host. |
| `linux-gstreamer` | Linux decoding through GStreamer. Without it, Linux builds and `is_supported()` returns `false`. |
| `upstream-video` | Additive zero-copy presentation via Cranpose 0.28's `cranpose::PlayerSurface` and `cranpose_services::DecodedFrames`. Opt-in; the default CPU-RGBA path is unchanged. |

```sh
cargo check                                   # default: widget + player
cargo test --features player,native-audio      # engine + widget test suites
cargo check --features player,native-audio,linux-gstreamer   # Linux with a decoder
cargo run -- clip.mp4                          # needs the `desktop` feature
```

---

## Architecture: how playback works

```text
rememberVideoPlayer(source)
  └─ rememberKeyed(source, |s| Rc::new(VideoPlayer::open(s.clone())))
        └─ VideoPlayerHandle { opened: Rc<Result<VideoPlayer, VideoError>> }

VideoFrame(handle, modifier)
  └─ picture(handle, modifier)
       ├─ rememberMutableStateOf(|| None::<ImageBitmap>)
       ├─ LaunchedEffect(handle): clear old picture -> play -> each window frame:
       │     if closed -> stop
       │     take_frame() -> into_rgba() -> ImageBitmap::from_rgba8() -> publish
       └─ Image(..., ContentScale::Fit) draws the latest frame
```

`rememberKeyed` is the key to clip changes. While the source is unchanged, the same player is reused. When the source changes, `init` opens a new player, the old `Rc` is dropped (closing its decode threads), the effect is cancelled, and the previous picture is cleared before the new one arrives.

---

## Platform support and verification

Evidence: GitHub Actions run **45709461351** (branch `master`, 2026-11-25 22:00 UTC, commit "Add rustdoc to SegmentKey byte-range fields"). **All seven jobs passed** on this exact code.

| CI job | Result | Notes |
|---|---|---|
| `native-windows` | **PASS** (4m52s) | Check (`player,native-audio`) and full tests. |
| `native-linux` | **PASS** (3m20s) | GStreamer dev packages; check with `linux-gstreamer`, tests serial. |
| `native-macos` | **PASS** (4m10s) | Full check and all tests. |
| `cross-wasm` | **PASS** (46s) | `wasm32-unknown-unknown`, `--no-default-features --features widget`. |
| `cross-android` aarch64 | **PASS** (1m18s) | NDK clang, `--no-default-features --features widget`. |
| `cross-android` armv7 | **PASS** (1m03s) | Same, second ABI. |
| `cross-ios` | **PASS** (1m07s) | `aarch64-apple-ios`; the earlier `NSNotification`/`block2` failure is fixed. |

The wall-clock 60 fps pacing test is skipped on the macOS CI runner, which cannot sustain a 60 fps cadence; the deterministic synthetic pacing tests still run there. Windows avoids `--all-features` because Cranpose 0.28's `mach-dxcompiler` build script refuses to download a mutable GitHub release — an upstream CI-environment issue, not this crate's code.

Historical evidence from the earlier app-containing repository layout (those workflows have been removed in this library-only split):

| Build workflow | Result |
|---|---|
| `cranpose-build.yml`, iOS debug, run **45302179025** | **PASS** (7m36s) — produced an `.ipa`. |
| `Build Android APK`, run **45046825409** | **PASS** (21m56s) — produced a signed `.apk`. |

### Device testing

| Platform | Device-tested | Notes |
|---|---|---|
| **Windows** | **Yes** | Playback tested by the maintainer on a real Windows machine. |
| **iOS** | **Yes** | Playback tested by the maintainer on a real iOS device, using an `.ipa` from a successful `cranpose-build.yml` run. |
| macOS / Linux / Android / Web | Not device-tested here | These appear only in the CI matrix above. |

What this means, honestly:

- **Windows and iOS playback is device-verified** by the maintainer. This is independent of CI.
- **Windows, Linux, WebAssembly and Android compile + test green** on CI.
- **macOS** compiles; two timing assertions in `tests/pacing.rs` are unreliable on that CI runner. No macOS device test is claimed.
- The **iOS widget-only cross-check** fails on one Objective-C observer method in this reduced feature set. That is a build-configuration issue, **not** a failure of the iOS playback that was tested on device with the full app feature set.
- There is no automated device-test job in this repository. A CI job is never presented as a substitute for the device results above.

---

## API guide

### Sources: `VideoSource`

```text
pub enum VideoSource { Path(Arc<Path>), Url(Arc<str>), Bytes(Arc<[u8]>) }
```

Constructors: `VideoSource::path(...)`, `VideoSource::url(...)`, `VideoSource::bytes(...)`. Plain strings with a scheme (`http://`, `https://`, `file://`) are read as URLs; anything else is a device path. `&str` and `String` convert directly, so `VideoPlayer::open("clip.mp4")` and `rememberVideoPlayer("https://.../master.m3u8")` work.

### Engine: `VideoPlayer`

| Method | Meaning |
|---|---|
| `open(source)` / `open_hls(source)` | Open with automatic routing / force HLS when the URL hides the `.m3u8` suffix. |
| `with_decoder(decoder)` | Supply your own `VideoDecoder` on an unsupported target. |
| `play`, `pause`, `toggle`, `stop` | Transport control. `stop` returns to the first frame. |
| `seek(Duration)`, `seek_fraction(f32)` | Absolute position / fraction of the duration. |
| `set_speed(f32)`, `set_looping(bool)` | Rate and repeat (`set_looping` returns `()` here). |
| `set_muted(bool)`, `set_volume(f32)` | Audio carried by the backend. |
| `take_frame()` | Take the newest decoded `VideoFrame`; call every frame you want to draw. |
| `position()`, `duration()`, `state()`, `is_playing()`, `info()`, `decoded_size()` | Live reads. `duration()` is `None` for live streams; `is_playing()` is the Media3-style `isPlaying` convenience derived from `state()`. |
| `is_closed()`, `close()` | Lifecycle; `close()` needs `&mut self`. `state()` reads `Idle` after close. `VideoPlayerHandle` has no explicit close — it closes with its player on composable exit. |

```rust,no_run
use cranpose_video::{VideoPlayer, VideoState};
use std::time::Duration;

fn play_file(path: &str) -> Result<(), cranpose_video::VideoError> {
    let mut player = VideoPlayer::open(path)?;
    let info = player.info();
    println!("{}x{} at {:?}", info.width, info.height, info.frame_rate);
    player.play()?;
    player.seek(Duration::from_secs(30))?;
    player.set_speed(0.5)?;
    let frame = player.take_frame();
    println!("frame: {:?}", frame.is_some());
    let _ = matches!(player.state(), VideoState::Playing | VideoState::Paused);
    player.close();
    Ok(())
}
```

### Tracks, media facts, looping and buffering

Four additions to the engine layer. All four are also readable from
`VideoPlayerHandle` inside a `#[composable]`.

#### Container facts: `MediaInfo`

`media()` returns what the backend learned at open time. A decoder that states
nothing returns the empty value, whose `is_empty()` is `true`.

| Item | Meaning |
|---|---|
| `container` | `Container::Mp4`, `Matroska`, `MpegTs`, `Elementary` or `Unknown`. |
| `video_codec`, `audio_codec` | The names the container states, such as `"h264"` and `"aac"`. |
| `bitrate` | Bits per second, when stated. `kilobit_rate()` for a display. |
| `duration` | `None` for a live stream. |
| `tracks` | Every stream, in the backend's own order. |
| `count_of(kind)` | How many streams of one `TrackKind` there are. |
| `summary()` | One line for a log or an about screen. |

```rust,no_run
use cranpose_video::{TrackKind, VideoPlayer};

fn describe(player: &VideoPlayer) {
    let media = player.media();
    if media.is_empty() {
        println!("the backend states nothing about this source");
        return;
    }
    println!("{}", media.summary());
    println!(
        "{} audio, {} subtitle",
        media.count_of(TrackKind::Audio),
        media.count_of(TrackKind::Subtitle),
    );
}
```

#### Track selection: `TrackList`, `TrackInfo`

`audio_tracks()` and `subtitle_tracks()` each return a snapshot `TrackList`.
An audio list starts on the container's default track; a subtitle list starts
**off**, because showing subtitles unasked is not what a viewer expects.

| Method | Meaning |
|---|---|
| `len()`, `is_empty()`, `tracks()` | The streams it holds. |
| `selected()` / `selected_track()` | The selected backend index / its `TrackInfo`. |
| `contains(index)` | Whether that index is one of the streams. |
| `select(index)` | Select by backend index, or `None` to turn the kind off. |
| `select_language(tag, forced_only)` | First stream that speaks `tag`; returns the index it chose. |
| `default_index()` / `default_on()` | The rule the list would start on. |

`TrackInfo::speaks(tag)` compares the **primary** subtag, so `"en"` matches
`"en-GB"`. Selection is by the backend's own index, not by a position inside
the list: a subtitle stream that sits after two audio streams has index `2`.

```rust,no_run
use cranpose_video::{VideoError, VideoPlayer};

fn pick_arabic(player: &VideoPlayer) -> Result<(), VideoError> {
    let mut tracks = player.audio_tracks();
    if let Some(index) = tracks.select_language("ar", false) {
        player.set_audio_track(Some(index))?;
    }
    // Off is always allowed; an absent index is refused and changes nothing.
    player.set_subtitle_track(None)?;
    Ok(())
}
```

#### Looping a section: `LoopRange`

`set_loop_range(Some(range))` repeats only that section. Setting a range also
turns looping on, so a screen does not need two calls.

`LoopRange::new(start, end)` orders the pair itself, so passing the ends the
other way round still works, and widens a span shorter than `MIN_RANGE`
(50 ms) to it — a loop never spins the decode thread on the same frames.

| Method | Meaning |
|---|---|
| `new(start, end)` / `from_length(start, length)` | Build. |
| `start()`, `end()`, `length()` | Bounds. |
| `contains(position)` | Start inclusive, end exclusive. |
| `is_past_end(position)` | Whether the play head should wrap. |
| `fraction_at(position)` | Progress through the range, `0.0..=1.0`. |
| `clamped_to(duration)` | Fit a range to the source length; takes `Option<Duration>` because a live stream has none. |

```rust,no_run
use cranpose_video::{LoopRange, VideoPlayer};
use std::time::Duration;

fn repeat_intro(player: &VideoPlayer) {
    player.set_loop_range(Some(LoopRange::new(
        Duration::from_secs(2),
        Duration::from_secs(5),
    )));
    // Back to repeating the whole source:
    player.set_loop_range(None);
}
```

#### Buffering: `Buffering`, `BufferingState`

`buffering()` reports how far the backend has filled its window, so a screen
shows a real ring instead of an endless spinner.

| Item | Meaning |
|---|---|
| `state()` | `Idle`, `Buffering`, or `Filling` (decoding ahead of the clock). |
| `fraction()` | `0.0..=1.0` of the window. **An idle buffer is `0.0`, not `1.0`.** |
| `percent()` | The same as `0..=100`, for a ring. |
| `is_idle()`, `is_waiting()` | The flags to branch on. |
| `since()`, `held_for(now)` | When the state last changed, and for how long. |
| `is_stalled(now, limit)` | Waiting longer than `limit` — offer a retry. |

```rust,no_run
use cranpose_video::VideoPlayer;
use std::time::{Duration, Instant};

fn overlay(player: &VideoPlayer) {
    let buffering = player.buffering();
    if buffering.state().is_waiting() {
        println!("buffering {}%", buffering.percent());
        if buffering.is_stalled(Instant::now(), Duration::from_secs(10)) {
            println!("stalled — offer a retry");
        }
    }
}
```

#### Playback settings: `PlaybackParameters`, `RepeatMode`

These are pure data types, usable before a player exists. Both follow the
Media3 model and are exported from the crate root.

| Item | Meaning |
|---|---|
| `PlaybackParameters::new()` | Speed `1.0`, pitch `1.0`. |
| `with_speed(speed)` / `from_parts(speed, pitch)` | Clamped into `MIN_SPEED..=MAX_SPEED` (`0.25..=4.0`). |
| `speed()` / `pitch()` | The two multipliers. Pitch `1.0` keeps voices natural when speed changes. |
| `RepeatMode::{Off, One, All}` | Stop, repeat this item, repeat the list. `next()` cycles them for a button. |
| `repeats_at(position, duration, has_next)` | Pure decision a player makes at the end; a live source with no duration never rewinds. |

```rust,no_run
use cranpose_video::{PlaybackParameters, RepeatMode};

let settings = PlaybackParameters::with_speed(1.5);
assert_eq!(settings.speed(), 1.5);
assert_eq!(settings.pitch(), 1.0); // speed without chipmunk voices
assert_eq!(RepeatMode::Off.next(), RepeatMode::One);
```

#### Player pool: `PlayerPool`, `PooledPlayer`

For a feed, [`PlayerPool`] joins the decoder budget with audio ownership. A
slot reserved at `Priority::Active` is the audible one; promoting a slot moves
sound to it; dropping a slot releases its decode budget immediately and hands
audio to the highest-ranked survivor. All handles share one bounded state.

```rust,no_run
use cranpose_video::{PlayerPool, Priority};

let pool = PlayerPool::new();
let first = pool.reserve(Priority::Active);
let next = pool.reserve(Priority::Next);
pool.promote(next.id(), Priority::Active);
assert!(next.is_audio_owner());
pool.release(first.id());
assert_eq!(pool.len(), 1);
```

Use this alongside the **one visible player** rule in the Reels scenario: the
pool bounds how many clips may stay warm, while the pager decides which of them
actually composes a picture.

New player-connected methods on `PlayerPool`:
`reserve_player`, `attach_player`, `with_player`, `resume_player` (calls
`player.play()` and makes the slot audible), `suspend_player` (calls
`player.pause()`), `release_player`, `current_active_player`,
`next_prepared_player`. The audio owner is the slot whose player is playing.

#### Events

Three variants join `PlaybackEvent`, so a diagnostics screen sees these
changes in order with everything else: `TrackChanged { kind, index }`,
`BufferingChanged { state, fraction }` and `Looped`.

#### Writing your own backend

`VideoDecoder` gained five **defaulted** methods — `media()`, `buffering()`,
`set_audio_track()`, `set_subtitle_track()` and `set_loop_range()` — so every
existing backend keeps compiling. Override only what your demuxer can state;
the defaults are correct for a decoder that carries one track and reads a file
as fast as the player asks.

### UI layer: `rememberVideoPlayer`, `VideoPlayerHandle`, composables

| Function | Meaning | Use it when |
|---|---|---|
| `rememberVideoPlayer(source)` | Opens one player keyed by source; returns a `VideoPlayerHandle`. Draws nothing. | You want custom controls. |
| `VideoFrame(handle, modifier)` | Draws the picture for **that** handle, owning the frame pump. | You already have a handle. |
| `VideoFrameWithPoster(handle, modifier, poster)` | Same picture, but shows a poster until the first frame and after a source change. | You want a shutter instead of a blank box on clip switches. |
| `Video(source, modifier)` | Shortcut: `rememberVideoPlayer(source)` + picture. | No custom controls needed. |
| `VideoWithControls(source, modifier)` | Picture plus the stock buttons and progress slider. | You want ready-made controls. |
| `VideoControls(handle, modifier)` | Stock controls for a handle you own, with your own layout. | Custom layout, stock controls. |
| `VideoProgressSlider(handle, modifier)` | Stock seek bar. | A custom control bar. |
| `VideoProgressSliderWith(handle, modifier, spec, content)` | Seek bar with your own thumb/track composable. | Fully custom progress UI. |
| `progress_fraction_for(position, duration)` | Pure helper returning `0.0..=1.0`. | Custom sliders. |

`VideoPlayerHandle` exposes `play`, `pause`, `toggle`, `stop`, `seek`, `seek_fraction`, `set_speed`, `set_looping`, `is_looping`, `is_playing`, `set_muted`, `is_muted`, `set_volume`, `volume`, `position`, `duration`, `state`, `speed`, `info`, `decoded_size`. All setters return `Result<(), VideoError>`; getters are pure. Getters return a neutral value instead of failing when the source failed to open.

```rust,no_run
use cranpose::{composable, prelude::*};
use cranpose_video::{VideoFrame, VideoPlayerHandle, rememberVideoPlayer};
use std::time::Duration;

#[composable]
fn Player(source: &'static str) {
    let handle: VideoPlayerHandle = rememberVideoPlayer(source);
    VideoFrame(handle.clone(), Modifier::empty().fill_max_width());

    let toggle = handle.clone();
    Button(Modifier::empty().padding(6.0), ButtonSpec::new(), move || {
        let _ = toggle.toggle();
    }, || { Text("Play / Pause", Modifier::empty(), TextStyle::default()); });

    let seek = handle.clone();
    Button(Modifier::empty().padding(6.0), ButtonSpec::new(), move || {
        let _ = seek.seek(Duration::from_secs(10));
    }, || { Text("+10s", Modifier::empty(), TextStyle::default()); });
}
```

Rule: **use one handle for both picture and controls**. Calling `Video(source, ..)` and `rememberVideoPlayer(source)` in the same screen opens two players for the same file.

### Detecting the source format

| Function | What it detects |
|---|---|
| `VideoSource::is_hls()` | `true` when the URL **path** ends in `.m3u8` (case-insensitive, percent-decoded) before any query or fragment. Local paths and byte buffers are never HLS. |
| `VideoSource::url(...)`, `as_url()`, `as_path()`, `as_bytes()` | Which of the three source variants you hold. |
| `VideoPlayer::open(source)` | Routes to HLS when `is_hls()` is true, otherwise to the native decoder. |
| `VideoPlayer::open_hls(source)` | Forces HLS for URLs whose playlist suffix is hidden by a query/CDN rewrite. |
| `VideoInfo { width, height, duration, frame_rate, rotation }` | What the **container states** after a successful open. `display_size()` applies rotation. No codec-name field is exposed. |
| `VideoPlayer::decoded_size()` / `handle.decoded_size()` | The **live** decoded size, `None` until the first frame. Follows adaptive/rotated streams. |
| `is_supported()`, `platform()`, `decoder_name()` | Build-time decoder availability and identity. |
| `set_diagnostic_sink`, `start_log_file`, `log_note`, `log_path`, `is_logging`, `stop_log_file` | Diagnostics; sink takes `Option<Box<dyn Fn(&str, &str, &str) + Send + Sync>>`. |
| `parse_subtitles(&str)`, `active_at(&[SubtitleCue], Duration)` | SRT/WebVTT parsing and cue lookup. |

HLS caveat: `rememberVideoPlayer` always calls `open`, never `open_hls`. A server URL like `/stream?id=123` is therefore treated as a plain file and will fail. Either make the URL path end in `.m3u8` before the query, or use the engine layer and call `VideoPlayer::open_hls` yourself.

### Zero-copy presentation (opt-in)

On the `v028-migration` line, the `upstream-video` feature re-exports Cranpose
0.28's `PlayerSurface` as `VideoHardwareSurface`. An application that owns an
upstream `DecodedFrames` source can present it directly:

```text
use cranpose_video::{VideoFrame, rememberVideoPlayer};

let handle = rememberVideoPlayer(source);
handle.attach_hardware_frames(frames); // same API, GPU path
VideoFrame(handle, Modifier::empty().fill_max_width());
```

`HardwareVideo::open_stream(codec, width, height, annex_b)` is the direct
hardware-decode entry point: it splits an elementary H.264/HEVC stream into
access units and feeds Cranpose's official decoder on a worker. Container
demuxing is still the caller's responsibility.

`HardwareVideo` wraps Cranpose 0.28's `VideoDecoder`: `submit(AccessUnit)`
feeds the hardware decoder, `frames()` returns `DecodedFrames` for the surface,
and `wait_for_frame().await` uses the decoder's own wake-up instead of polling.
`HardwareVideo::capabilities(codec)` reports device limits from the platform.

The **same public API** now drives both paths. Attach those frames with
`player.attach_hardware_frames(frames)` or
`handle.attach_hardware_frames(frames)`; from then on `VideoFrame`, `Video`,
`VideoWithControls`, the progress slider and the stock controls keep working,
but the picture is drawn by `PlayerSurface` instead of the CPU RGBA pump.
Without attached frames the previous path remains the fallback, unchanged.

That path is supplied by Cranpose's own hardware decode foundation (Metal on
Apple, Vulkan AHardwareBuffer import on Android) and does not pass through this
crate's CPU RGBA buffer. It is **not** claimed as verified on a device here:
Cranpose itself documents that the surface draws nothing where the GPU lacks
the import extensions or renders through OpenGL ES. The crate's own
`Video`/`VideoFrame` path remains the tested CPU fallback.

### Controller and streaming policy

`PlaybackController` is a small trait implemented by both `VideoPlayer` and
`VideoPlayerHandle`, so a UI layer can drive either through one interface.
`VideoSurface` plus `CpuSurface` name a presentation target and its size; this
is an abstraction only and does not claim GPU access.

`StreamPolicy` joins `LoadControl` (when playback may start and whether to keep
reading ahead) with `AbrController` (which representation to fetch). It accepts
both native `Representation` values and DASH representations converted from
`parse_mpd`. It makes decisions; the streaming backend still performs the
downloads and decoding.

### Event delivery

`EventBus` is a bounded, clone-shared log of `PlaybackEventRecord`s. It publishes
the full event set (`FrameAvailable`, `FirstFrameReady`, `PositionChanged`,
`DurationChanged`, `VideoSizeChanged`, `SeekStarted/Completed`, buffering,
decoder changes, errors and resource pressure). Consecutive position events are
**coalesced by default**, so a 60 fps clock does not flood the UI; disable it
with `set_coalesce_positions(false)` when a caller needs every tick.

```rust,no_run
use cranpose_video::{EventBus, PlaybackEventKind};
use std::time::Duration;

let bus = EventBus::new(64);
let marker = bus.publish(PlaybackEventKind::PlaybackStarted, Duration::ZERO);
bus.publish(PlaybackEventKind::PositionChanged, Duration::from_millis(16));
for event in bus.since(marker) {
    println!("{:?} @ {:?}", event.kind, event.position);
}
```

This is an event log, not a wake-up primitive: the UI still drives drawing
through Cranpose. It gives applications ordered observations without unbounded
queues or 60 position events per second by default.

### DASH, ABR and disk cache

Three more engine layers are available without new dependencies:

- `parse_mpd(&str)` reads an MPD's representations, segment template,
  initialization segment and static/dynamic flag. It is structural parsing
  only — no segment download or decode is performed.
- `AbrController` with `Representation`, `AbrMode` and `AbrConfig` implements a
  conservative selector: downgrades react immediately, upgrades require several
  good throughput samples and enough buffered media. Fixed `Lowest`, `Medium`
  and `Highest` modes are also available.
- `DiskCache` writes payloads under a root directory with safe encoded keys and
  atomic rename, and evicts oldest files when the byte budget is exceeded.

```text
let manifest = parse_mpd(mpd_text)?;
let mut abr = AbrController::new(AbrConfig::default());
let index = abr.sample(throughput_bps, buffer_millis, &reps);
let cache = DiskCache::open("./cache", 512 * 1024 * 1024)?;
cache.put("segment-1", &bytes)?;
```

These are building blocks, not a complete DASH player: scheduling, segment
fetching and synchronization remain application or platform-backend work.

### Segment scheduling

`SegmentScheduler` tracks HLS/fMP4 segment fetches without an HTTP client:
`SegmentKey` distinguishes initialization (`EXT-X-MAP`), media and byte-range
requests with distinct cache keys. `request_next()` respects a bounded
in-flight window, `complete/fail` require the current generation, and
`invalidate()` cancels stale requests on an ABR switch or feed swipe.

### Predictive preload levels

`PreloadCoordinator` schedules sources by distance from the selected item and
tracks six levels: `Metadata`, `Bytes`, `Source`, `Decoder`, `FirstFrame` and
`Prepared`. `PreloadConfig` sets how many items may reach each level and the
parallel-download budget. It reuses `PreloadManager` generations, so
`cancel` / `cancel_obsolete` reject late chunks and completions. It makes
scheduling decisions only — HTTP and decoder work remain application-side.

### Preparing incomplete files

`PreloadManager` tracks a source that arrives in pieces: `begin` returns a
generation, `offer_chunk` appends bytes, `finish` returns the full buffer once,
and `mark_prepared` records that a player slot was opened from it. A cancelled
attempt bumps its generation, so late chunks and completions are rejected and
their bytes are released. It performs no network I/O itself.

### Capability reporting

`VideoCapabilities::detect()` reports what the **selected backend** can be
asked to decode. It is a build/platform-level answer, not a device probe: a
particular machine may still lack a hardware codec or a GStreamer plugin.

```rust,no_run
use cranpose_video::{VideoCapabilities, VideoCodec};

let caps = VideoCapabilities::detect();
println!("{:?}: {:?}", caps.platform(), caps.supported_codecs());
if caps.supports_codec(VideoCodec::H264) {
    // The backend can be asked; an open may still fail on a specific device.
}
```

`DecoderKind` distinguishes the platform stack, a platform software path, an
application-supplied decoder (`VideoPlayer::with_decoder`), and no decoder.
No in-crate GPU capability is claimed here — see the rendering limitations above.

### Other exported clusters

The crate root also exports these groups; each has rustdoc at its source file:

- **Colour and frame formats** (`src/color.rs`, `src/frame.rs`): `ColorInfo`, `HdrMetadata`, `ColorPrimaries`, `TransferFunction`, `ChromaLocation`, `ColorMatrix`, `ColorRange`, `PixelFormat`, `YuvColor`. `VideoFrame` exposes `format()`, `color()`, `rgba()`, `into_rgba()`. `ImageBitmap::from_rgba8` is used by the built-in picture composable; `set_native_nv12` / `native_nv12` are available on `VideoPlayer` for backends that carry native frames.
- **Explicit reads** (`src/decode_result.rs`): `ReadOutcome` and `FrameStamp` distinguish no frame, a new frame, and a timestamp from an ambiguous empty buffer.
- **Resources** (`src/resource.rs`): `VideoResourceManager`, `ResourceLimits`, `PrefetchGuard` bound decoder slots, frame bytes and prefetches. `PlayerPool` wraps them with audio ownership.
- **Session and recovery** (`src/session.rs`, available on non-Linux/native targets): `PlaybackSession`, `PlaybackState`, `InterruptionReason`, `SessionEvent`, `RECOVERY_COOLDOWN`, `RECOVERY_COOLDOWN_MAX`. Audio route changes and interruptions are handled here.
- **Telemetry** (`src/telemetry.rs`): `PlaybackDiagnostics`, `EventRecord`, `PlaybackEvent`, `SessionId` expose counters, timings, pixel format and an ordered event history via `VideoPlayer::diagnostics()` / `events()`.
- **Player extras** (`src/player.rs`): `is_live()`, `is_seekable()`, `speed()`, `is_muted()`, `volume()`, `media_info()`, `loop_range()`, `recycle(frame)`, `set_audio_group(...)`, `note_route_changed()`.

`PlayerPool::promote` returns a **non-owning view** of the slot. Keep or drop it; dropping it does not release the slot or steal audio ownership. Only the handle from `reserve` owns the slot. This is intentional so a temporary returned by `promote(...)` cannot silence the promoted page.

### Feed infrastructure primitives

Four small, target-independent types cover the bookkeeping a video-first feed
needs around the player. They are pure data: no HTTP client, no platform API,
and no decoder is touched.

| Type | Purpose |
|---|---|
| `LoadControl` | Buffer bounds and thresholds: `with_min_buffer`, `with_max_buffer`, `with_buffer_for_playback`, `with_rebuffer_threshold`; queries `should_start`, `should_continue_loading`, `should_rebuffer`, `is_stalled`. |
| `MediaCache` | Byte-bounded LRU of source payloads: `put`, `get`, `remove`, `clear`, `bytes`, `len`. A hit converts to `VideoSource::bytes(...)`. Clones share one store. |
| `AudioFocus`, `AudioFocusState` | Focus state machine: `should_pause`, `should_duck`, `volume_multiplier`, `apply`, `abandon`. The platform owns the real focus request; this decides what the player should do. |
| `PlayerPool` / `PooledPlayer` | Decoder-slot budget plus one audible owner, shown earlier. |

```rust,no_run
use cranpose_video::{AudioFocus, AudioFocusState, LoadControl, MediaCache, VideoSource};
use std::time::Duration;

let control = LoadControl::new().with_min_buffer(Duration::from_millis(500));
assert!(control.should_start(Duration::from_secs(1)));
assert!(control.should_continue_loading(Duration::from_secs(1)));

let cache = MediaCache::with_limit(1024);
cache.put("clip-a", vec![1u8; 64]);
if let Some(bytes) = cache.get("clip-a") {
    let _source = VideoSource::bytes(bytes);
}

let mut focus = AudioFocusState::new(0.25);
assert!(!focus.apply(AudioFocus::Transient)); // duck, do not pause
assert_eq!(focus.volume_multiplier(), 0.25);
assert!(focus.abandon());
```

These are deliberately not a substitute for platform caching or audio sessions:
`MediaCache` holds bytes in memory only, and `AudioFocus` does not register a
system focus request.

### Video feed orchestration

`VideoFeedController` owns the state machine of a vertical feed: selection,
preparation stages, generation tokens that reject stale completions, and a
bounded `PlayerPool` with one audible owner. It does **not** open decoders or
download bytes — the application performs that work and reports it through
`begin_preload` / `set_preload_stage` / `finish_preload`. That separation keeps
the orchestration testable on every target and leaves native I/O to the
platform backends.

```rust,no_run
use cranpose_video::{
    FeedItem, FeedStage, PreloadStrategy, VideoFeedConfig, VideoFeedController,
};

let config = VideoFeedConfig {
    preload_next: 2,
    preload_strategy: PreloadStrategy::Adaptive,
    ..Default::default()
};
let mut feed = VideoFeedController::new(config);
feed.set_items(vec![FeedItem::new("a", "a.mp4"), FeedItem::new("b", "b.mp4")]);

assert!(feed.set_current_index(0));
let generation = feed.begin_preload(1).expect("the next item can be prepared");
assert!(feed.finish_preload(1, generation));
assert_eq!(feed.stage(1), Some(FeedStage::Prepared));

assert!(feed.set_current_index(1));
assert_eq!(feed.stage(1), Some(FeedStage::Playing));
assert_eq!(feed.pool().audible(), feed.active_slot());
```

Switching selection cancels other in-flight operations and bumps their
generation, so a late completion from the old item cannot replace the new one.
Only the selected slot is audible; prepared slots are bounded by
`max_concurrent_decoders`.

---

## Scenarios

### 1. Simple screen, no custom controls

```rust,no_run
use cranpose::{composable, prelude::*};
use cranpose_video::{Video, VideoWithControls};

#[composable]
fn Screen() {
    Video("clip.mp4", Modifier::empty().fill_max_width());
    VideoWithControls("clip.mp4", Modifier::empty().fill_max_width());
}
```

### 2. Mixed MP4 and HLS from a server

```rust,no_run
use cranpose::{composable, prelude::*};
use cranpose_video::{VideoFrame, rememberVideoPlayer};

#[derive(Clone, PartialEq)]
struct Clip { title: String, source: String }

#[composable]
fn PlayerPage(clip: Clip) {
    // Works for MP4 URLs and for HLS URLs whose path ends in .m3u8.
    let handle = rememberVideoPlayer(clip.source);
    VideoFrame(handle.clone(), Modifier::empty().fill_max_width());
    Text(clip.title, Modifier::empty(), TextStyle::default());
}
```

### 3. Reels pager: exactly one decoder open

```rust,no_run
use cranpose::{composable, prelude::*};
use cranpose_video::{VideoFrame, rememberVideoPlayer};

const CLIPS: &[&str] = &["a.mp4", "b.mp4", "c.mp4"];

#[composable]
fn ReelsScreen() {
    let pager = rememberPagerState(0usize);
    let settled = pager.settled_page();
    Column(Modifier::empty().fill_max_size(), ColumnSpec::default(), move || {
        VerticalPager(
            Modifier::empty().fill_max_width().weight(1.0),
            pager,
            PagerSpec::default(),
            CLIPS.len(),
            move |page| {
                if page == settled { PlayerPage(CLIPS[page]); }
                else { Text("poster", Modifier::empty(), TextStyle::default()); }
            },
        );
    });
}

#[composable]
fn PlayerPage(source: &'static str) {
    let handle = rememberVideoPlayer(source);
    VideoFrame(handle.clone(), Modifier::empty().fill_max_width());
}
```

Do not render every clip as a `Video`: each mounted call opens its own decoder. Gate playback on the settled page and show posters elsewhere.

### 4. Server-fed feed with infinite scroll

State: `clips`, `next_page`, `loading`, `requested`, `ended`, `error`, `request`. A `LaunchedEffect` keyed by `request` performs the fetch; another keyed by `pager.target_page()` decides when to request the next page (prefetch before the user arrives). The `requested` flag prevents duplicate requests during fast scrolling.

```rust,ignore
// Shape of the loop (HTTP client omitted — use your own):
//   target = pager.target_page()
//   if target + 3 >= clips.len() && !loading && !ended && !requested {
//       requested = true; request += 1;                 // triggers fetch effect
//   }
// fetch effect: GET page `next_page`; on Ok(non-empty) extend clips and
// next_page += 1; on Ok(empty) ended = true; on Err keep next_page unchanged;
// then loading = false; requested = false.
// Guard the pager with clips.len() > 0 and clamp settled_page().min(len - 1).
```

### 5. Live HLS

```rust,no_run
use cranpose::{composable, prelude::*};
use cranpose_video::Video;

#[composable]
fn Live() {
    // duration() is None for a live playlist; a progress slider is disabled.
    Video("https://cdn.example.com/live/master.m3u8?token=abc",
          Modifier::empty().fill_max_width());
}
```

### 6. Bytes held by the application

```rust,no_run
use cranpose_video::{VideoPlayer, VideoSource};

fn open_bundled(bytes: Vec<u8>) -> Result<VideoPlayer, cranpose_video::VideoError> {
    VideoPlayer::open(VideoSource::bytes(bytes))
}
```

### 7. Diagnostics

```rust,no_run
use cranpose_video::{set_diagnostic_sink, start_log_file};

fn enable_diagnostics() {
    let _ = start_log_file("video.log");
    set_diagnostic_sink(Some(Box::new(|level, target, message| {
        eprintln!("[{level}] {target}: {message}");
    })));
}
```

---

## Future compatibility with Cranpose

Dependencies are declared as `0.28.0`, which Cargo reads as `>=0.28.0, <0.29.0`. That admits patch releases in the 0.28 line but **not** 0.29+: in the 0.x range Cargo treats a minor bump as potentially breaking, and Cranpose's composable/runtime APIs are exactly the kind that can change there. This branch tracks the latest upstream line; `master` remains the verified 0.24 line until this migration passes the full CI matrix.

Practical options when a newer Cranpose arrives:

1. Upgrade this crate's `cranpose`, `cranpose-core` and `cranpose-ui` requirements together to the new minor, run `cargo test`, and fix any API changes in `src/widget.rs`.
2. Test a candidate version without editing this repository by adding to your application:
   ```toml
   [patch.crates-io]
   cranpose = { path = "../cranpose-fork" }
   ```
3. Pin exact builds with `Cargo.lock`. Applications get reproducibility; the library authors get a clear upgrade point.

The engine layer (`VideoPlayer`, `VideoSource`, decoders) does not depend on Cranpose crates and is therefore unaffected by UI-runtime upgrades. Only the `widget` feature touches Cranpose APIs.

This policy was confirmed as correct by the Cranpose maintainer in [Cranpose#2477](https://github.com/samoylenkodmitry/Cranpose/issues/2477): every published crate shares the workspace version; patch releases preserve API compatibility (checked with `cargo semver-checks`), and breaking changes bump the minor. See [`docs/release.md`](https://github.com/samoylenkodmitry/Cranpose/blob/main/docs/release.md#versioning).

### Relation to upstream video work

Upstream is building video-first support directly on its own zero-copy hardware foundation (`VideoDecoder`, `DecodedFrames`, `PlayerSurface`, `PlayerSurfaceIntake`) under [Cranpose#2481](https://github.com/samoylenkodmitry/Cranpose/issues/2481), which supersedes #2197 and #2477. That is the authoritative path for zero-copy GPU playback, Media3 API parity, and event-driven presentation. This crate remains a **community crate on the CPU-RGBA path**; it does not claim to be that foundation.

Techniques from that direction that fit here are added additively: `VideoFrameWithPoster` (shutter until the first frame), `is_playing()`, `PlaybackParameters`, `RepeatMode`, `MediaInfo`/`TrackList` facts, `Buffering`, `LoopRange`, and `PlayerPool`. What is deliberately **not** implemented here is the upstream hard agreement set: zero-copy GPU import (D3D11/Metal/AHardwareBuffer/DMABUF/WebGPU), zero steady-frame allocations, event-driven frame delivery instead of per-frame polling, and Media3 types wired into `cranpose-services`. Those need upstream internals and device benchmarks; see #2481.

A widget-only iOS build previously failed because `objc2-foundation` enabled `NSNotification` without `block2`, which gates `addObserverForName_object_queue_usingBlock`. The maintainer identified this in #2477; `block2` is now enabled in `Cargo.toml`.

---

## Known limitations

- macOS pacing tests are timing-sensitive and failed on the CI runner in run 45308233149.
- The widget-only iOS CI cross-check previously failed on `NSNotificationCenter` observer availability; the missing `block2` feature is now enabled (maintainer guidance in #2477) and the cross-check needs a fresh CI run to confirm. iOS playback itself was device-tested with the full app feature set and works.
- Linux needs `linux-gstreamer` plus GStreamer dev packages for decoding.
- watchOS has no video decoder (`is_supported()` is false).
- GPU zero-copy and HDR device rendering are **not implemented or verified** here. Playback is device-tested on Windows and iOS; no quantitative performance numbers are claimed.
- No arbitrary HTTP headers can be attached to a `VideoSource`; use signed URLs.

## Full API reference

Every item below is exported by the crate root (`pub use` in `src/lib.rs`) and is
compiled into this crate's doctests because the README is included as crate
docs. Groups map to files: `player.rs` (engine), `widget.rs` (UI),
`pool.rs` (feed pools), `preload.rs` / `preload_coordinator.rs` (incomplete
sources and predictive preloading), `feed.rs` (feed states), `cache.rs` /
`disk_cache.rs` (storage), `dash.rs` / `abr.rs` / `stream_policy.rs` /
`playback_params.rs` / `repeat_mode.rs` / `loop_range.rs` (streaming),
`upstream.rs` (Cranpose 0.28 bridge), `events.rs` / `telemetry.rs` (monitoring),
`resource.rs` / `buffer.rs` / `media.rs` / `tracks.rs` / `frame.rs` /
`color.rs` / `subtitle.rs` / `capabilities.rs` / `platform/*` (mixed engine).

### Engine — `VideoPlayer`

| Method | Meaning |
|---|---|
| `open(source)` / `open_hls(source)` | Open, routing by `is_hls()` / forcing HLS. |
| `with_decoder(decoder)` | Supplies a custom `VideoDecoder` on unsupported targets. |
| `info()` → `VideoInfo { width, height, duration, frame_rate, rotation }` | What the container states; `display_size()` applies rotation. |
| `state()` → `VideoState` (`Idle/Loading/Playing/Paused/Ended/Failed`) | Current state. |
| `is_playing()`, `is_live()`, `is_seekable()`, `is_closed()` | Booleans for the common states. |
| `position()`, `duration()`, `speed()`, `volume()`, `is_muted()` | Live values; `duration()` is `None` for live streams. |
| `play()`, `pause()`, `toggle()`, `stop()`, `seek(Duration)`, `seek_fraction(f32)`, `set_speed(f32)`, `set_looping(bool)`, `set_muted(bool)`, `set_volume(f32)` | Transport; setters return `Result<(), VideoError>`. |
| `take_frame()` → `Option<VideoFrame>` | Takes the newest decoded frame. |
| `recycle(frame)` | Returns the frame buffer to the decoder pool. |
| `set_native_nv12(bool)` / `native_nv12()` | Uses/wants native NV12 frames when the backend supports them. |
| `media()`, `media_info()`, `audio_tracks()`, `subtitle_tracks()`, `set_audio_track(idx)`, `set_subtitle_track(idx)` | Media facts and track selection. |
| `buffering()` → `Buffering` (`state/fraction/percent/since/held_for/is_stalled/is_idle`) | Buffering window, stalls, retry hints. |
| `set_audio_group(u64)` | Adds the player to an audio group. |
| `events()` → `Vec<EventRecord>`, `publication_timeline()`, `diagnostics()`, `recovery_events()` | Event history and recovery info. |
| `note_route_changed()` | Notifies the player that the device audio route changed. |
| `attach_hardware_frames(frames)` / `hardware_frames()` | Attaches upstream-0.28 `DecodedFrames` for the GPU path (when built with `upstream-video`). |
| `close()` | Explicit close (`&mut self`). |

### Widget — `VideoPlayerHandle`, composables

| Function | Meaning |
|---|---|
| `rememberVideoPlayer(source)` | Opens one player keyed by source, returns a `VideoPlayerHandle`. |
| `VideoFrame(handle, modifier)` | Draws the picture; also routes to `PlayerSurface` if `attach_hardware_frames` was called. |
| `VideoFrameWithPoster(handle, modifier, poster)` | Poster until first frame and after source change. |
| `Video(source, modifier)` | Shortcut: `rememberVideoPlayer` + picture. |
| `VideoWithControls(source, modifier)` | Picture + stock controls and progress slider. |
| `VideoControls(handle, modifier)` | Stock controls for a handle. |
| `VideoProgressSlider(handle, modifier)` / `VideoProgressSliderWith(handle, modifier, spec, content)` | Progress sliders; the latter takes a custom thumb/track. |
| `progress_fraction_for(position, duration)` | Pure helper `0.0..=1.0`. |

`VideoPlayerHandle` mirrors the `VideoPlayer` methods above, all with `&self`.

### Feeds — `VideoFeedController`, `PlayerPool`

`VideoFeedController` lifecycle: `Idle/Requested/Downloading/Preparing/Prepared/Ready/Playing/Paused/Transitioning/Completed/Failed/Cancelled`. Key methods: `set_items`, `set_current_index`, `begin_preload`, `set_preload_stage`, `finish_preload`, `cancel_preload`, `cancel_all`, `preload_next`, `preload_previous`, `current_item`, `stage`, `pool`.

`PlayerPool` methods: `reserve`, `reserve_player`, `attach_player`, `with_player`, `promote`, `resume_player`, `suspend_player`, `release`, `release_player`, `audible`, `ranked`, `suspended`, `len`, `total_frame_bytes`, `is_decoding`, `is_audio_owner`, `set_frame_bytes`, `current_active_player`, `next_prepared_player`.

### Predictive preloading — `PreloadCoordinator`

Six levels (`PreloadLevel`): `Metadata, Bytes, Source, Decoder, FirstFrame, Prepared`. Methods: `new`, `config`, `manager`, `task`, `schedule`, `advance`, `cancel`, `cancel_obsolete`, `next_prepared`. Uses the generation-safe `PreloadManager`.

### Incomplete sources — `PreloadManager`, `PreloadStage`

Stages: `Idle/Requested/Downloading/Preparing/Prepared/Failed/Cancelled`. Methods: `begin`, `offer_chunk`, `finish`, `mark_prepared`, `cancel`, `stage`, `buffered`, `clear`. No I/O — the application feeds chunks.

### Caching

`MediaCache` (in-memory LRU): `with_limit`, `limit`, `bytes`, `len`, `is_empty`, `get`, `put`, `remove`, `clear`.
`DiskCache` (persistent): `open(root, max_bytes)`, `get`, `put`, `remove`, `bytes`, `root`, `limit`.

### Streaming / ABR

`parse_mpd(input)` → `DashManifest { dynamic, duration_ms, segment_template, initialization, representations }`.
`AbrController`: `new`, `mode`, `current`, `set_mode`, `sample(throughput_bps, buffer_millis, reps)`.
`StreamPolicy`: `load_control`, `abr`, `set_mode`, `set_manual_index`, `select(throughput_bps, buffer, reps)`, `dash_representations(reps)`.
`BufferSnapshot { buffered, waiting }` used by the policy.

### Events

`EventBus::new(capacity)`; `publish(kind, position)`, `events()`, `since(seq)`, `last()`, `clear()`, `set_coalesce_positions(bool)`, `coalesces_positions()`. Event kinds: `FrameAvailable`, `PlaybackStarted/Paused`, `BufferingStarted/Ended`, `PositionChanged`, `DurationChanged`, `FirstFrameReady`, `VideoSizeChanged`, `SeekStarted/Completed`, `Ended`, `DecoderChanged`, `Error`, `ResourcePressure`.

### Misc

`HardwareVideo` (upstream): `open_stream(codec, w, h, annex_b)`, `submit`, `frames`, `wait_for_frame`, `stats`, `capabilities`.
`VideoHardwareSurface` — re-exported `cranpose::PlayerSurface` (Android Metal/Vulkan, Apple Metal).

## License

Apache-2.0 OR MIT. See `LICENSE` and `LICENSE-MIT`.
