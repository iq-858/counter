# cranpose-video

Video playback for [Cranpose](https://crates.io/crates/cranpose) through each platform's own media stack.

This crate gives you two ways to play video:

1. **Engine layer** — `VideoPlayer`, a plain Rust API that opens a source, decodes frames and controls playback. It knows nothing about UI.
2. **UI layer** — `rememberVideoPlayer`, `VideoPlayerHandle` and `VideoFrame`, built on the `widget` feature, for use inside `#[composable]` functions.

Both layers use the same decoder per platform. The UI layer does not add a second player: a handle is a cloneable wrapper around one `VideoPlayer`, and the picture composable drives that same player's frame pump.

> Scope note: this repository contains this crate only. It is **compile + unit-test verified** on CI. Real-device playback is reported separately and is not claimed by the matrix.

---

## Requirements and versions

| Item | Value |
|---|---|
| Rust edition | 2024 (requires rustc **1.85+**) |
| Cranpose crates | `0.24.0` (caret: `>=0.24.0, <0.25.0`) |
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

```sh
cargo check                                   # default: widget + player
cargo test --features player,native-audio      # engine + widget test suites
cargo check --features player,native-audio,linux-gstreamer   # Linux with a decoder
cargo run -- clip.mp4                          # needs the `desktop` feature
```

---

## Architecture: how playback works

```
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

Evidence: GitHub Actions run **45308233149** (branch `master`, 2026-11-24 01:52 UTC, commit "Add VideoFrame(handle) for custom designs"). This was the last full matrix run of this code.

| CI job | Result | Notes |
|---|---|---|
| `native-windows` | **PASS** (8m08s) | `cargo check --all-features` and tests, Windows runner. |
| `native-linux` | **PASS** (4m16s) | GStreamer dev packages installed; check with `linux-gstreamer`, tests serial. |
| `cross-wasm` | **PASS** (43s) | `wasm32-unknown-unknown`, `--no-default-features --features widget`. |
| `cross-android` aarch64 | **PASS** (1m22s) | NDK clang, `--no-default-features --features widget`. |
| `cross-android` armv7 | **PASS** (2m16s) | Same, second ABI. |
| `native-macos` | **FAIL** (3m53s) | Two timing assertions in `tests/pacing.rs` failed: `smooth_60fps_playback_has_no_long_gaps` and `an_injected_stall_is_detected_at_60fps`. 145 library tests plus all other integration suites passed. This is runner jitter in timing tests, not a decoder failure. |
| `cross-ios` | **FAIL** (1m07s) | `error[E0599]`: `addObserverForName_object_queue_usingBlock` not found on `Retained<NSNotificationCenter>` at `src/platform/ios_route.rs:33` under the widget-only feature set. |

Historical evidence from the earlier app-containing repository layout (those workflows have been removed in this library-only split):

| Build workflow | Result |
|---|---|
| `cranpose-build.yml`, iOS debug, run **45302179025** | **PASS** (7m36s) — produced an `.ipa`. |
| `Build Android APK`, run **45046825409** | **PASS** (21m56s) — produced a signed `.apk`. |

What this means, honestly:

- **Windows, Linux, WebAssembly and Android compile + test green** on CI.
- **macOS** compiles; its two pacing tests are unreliable on the CI runner.
- **iOS** compile-check with the reduced widget feature set fails on one Objective-C observer method; a full iOS app build succeeded earlier with a different feature set.
- No automated real-device playback test exists in this repository. Do not read a green compile job as a device-tested claim.

---

## API guide

### Sources: `VideoSource`

```rust
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
| `position()`, `duration()`, `state()`, `info()`, `decoded_size()` | Live reads. `duration()` is `None` for live streams. |
| `is_closed()`, `close()` | Lifecycle; `close()` needs `&mut self`. |

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

### UI layer: `rememberVideoPlayer`, `VideoPlayerHandle`, composables

| Function | Meaning | Use it when |
|---|---|---|
| `rememberVideoPlayer(source)` | Opens one player keyed by source; returns a `VideoPlayerHandle`. Draws nothing. | You want custom controls. |
| `VideoFrame(handle, modifier)` | Draws the picture for **that** handle, owning the frame pump. | You already have a handle. |
| `Video(source, modifier)` | Shortcut: `rememberVideoPlayer(source)` + picture. | No custom controls needed. |
| `VideoWithControls(source, modifier)` | Picture plus the stock buttons and progress slider. | You want ready-made controls. |
| `VideoControls(handle, modifier)` | Stock controls for a handle you own, with your own layout. | Custom layout, stock controls. |
| `VideoProgressSlider(handle, modifier)` | Stock seek bar. | A custom control bar. |
| `VideoProgressSliderWith(handle, modifier, spec, content)` | Seek bar with your own thumb/track composable. | Fully custom progress UI. |
| `progress_fraction_for(position, duration)` | Pure helper returning `0.0..=1.0`. | Custom sliders. |

`VideoPlayerHandle` exposes `play`, `pause`, `toggle`, `stop`, `seek`, `seek_fraction`, `set_speed`, `set_looping`, `is_looping`, `set_muted`, `is_muted`, `set_volume`, `volume`, `position`, `duration`, `state`, `speed`, `info`, `decoded_size`. All setters return `Result<(), VideoError>`; getters are pure. Getters return a neutral value instead of failing when the source failed to open.

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

#[derive(Clone)]
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

Dependencies are declared as `0.24.0`, which Cargo reads as `>=0.24.0, <0.25.0`. That admits patch and maintenance releases in the 0.24 line, but **not** 0.25+: in the 0.x range Cargo treats a minor bump as potentially breaking, and Cranpose's composable/runtime APIs are exactly the kind that can change there. Widening the range to `>=0.24, <1` would let a new minor version compile against code written for the old one and fail at build time in your application; that is why the crate does not do it.

Practical options when a newer Cranpose arrives:

1. Upgrade this crate's `cranpose`, `cranpose-core` and `cranpose-ui` requirements together to the new minor, run `cargo test`, and fix any API changes in `src/widget.rs`.
2. Test a candidate version without editing this repository by adding to your application:
   ```toml
   [patch.crates-io]
   cranpose = { path = "../cranpose-fork" }
   ```
3. Pin exact builds with `Cargo.lock`. Applications get reproducibility; the library authors get a clear upgrade point.

The engine layer (`VideoPlayer`, `VideoSource`, decoders) does not depend on Cranpose crates and is therefore unaffected by UI-runtime upgrades. Only the `widget` feature touches Cranpose APIs.

---

## Known limitations

- macOS pacing tests are timing-sensitive and failed on the CI runner in run 45308233149.
- The widget-only iOS cross-check fails on `NSNotificationCenter` observer availability; the full iOS app build succeeded earlier with a larger feature set.
- Linux needs `linux-gstreamer` plus GStreamer dev packages for decoding.
- watchOS has no video decoder (`is_supported()` is false).
- GPU zero-copy, HDR device rendering and real-device performance numbers are **not implemented or verified** here.
- No arbitrary HTTP headers can be attached to a `VideoSource`; use signed URLs.

## License

Apache-2.0 OR MIT. See `LICENSE` and `LICENSE-MIT`.
