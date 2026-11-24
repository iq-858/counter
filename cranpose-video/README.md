# Cranpose Video

`cranpose-video` decodes video with the media stack of the platform it runs on, hands the pictures to a Cranpose screen as RGBA8 or as the decoder's own NV12 planes, and reports what the container states about the track. It is separate from [`cranpose-media`](https://docs.rs/cranpose-media), which plays audio.

The crate is a single source tree that chooses one backend at compile time. This document records the backend list, what the current update changed on each one, the faults the update fixes, and complete examples from a one-line composable up to a custom decoder.

### Evidence provenance

> **Read this before using any number below.** A recursive search of the current `D:\counter` tree found none of the named raw logs (`probe-final-after-tables.log`, `pacing-hrtimer.log`, `probe-repeat-final.log`, `perf-final.log`, `doc-final-after-docupdates.log`, `verify-final-after-docupdates.log`, `pacing-nodefault-final.log`). Numeric results that cite those names are preserved from earlier session reports and are marked **`[REPORT-ONLY]`**: they are historical claims, not independently reproducible artifacts in this checkout. **`[SOURCE-VERIFIED]`** is a fact read directly from the cited source, optionally with a unit test, but **not** a device measurement. **`[HYPOTHESIS]`** is a runtime or device consequence inferred from source, not measured. No Linux/Reels device run has completed — WSL was fully removed — so measured slots are intentionally empty and marked **`[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]`**; no value may be invented. If none of these markers is present, treat the statement as qualitative, not as a fresh benchmark.

---

## What this update adds

Everything below is **additive**: no public function signature changed, no variant was added to `VideoState`, no field was added to an existing public struct, and no older example stops compiling. New hooks on `VideoDecoder` have default implementations, so an existing decoder keeps its behaviour.

| Area | Addition in this update |
| --- | --- |
| HLS throughput | Bounded parallel fragment fetch (`PARALLEL_FETCH = 4`), results placed back by playlist index, adaptive per-fragment deadlines, and off-thread prefetch of the next window's fragments — including a separate audio rendition. |
| HLS adaptation | Every fragment's arrival rate is measured, and the rendition is chosen again at each window boundary from that estimate, with headroom for the container and its bursts. A slow link drops to a smaller picture instead of stalling on fragments it cannot fetch in time; a link with room moves up. |
| HLS timing | One `tfdt` base per track, not one delta shared across tracks; per-window rebasing; discontinuity carried on `Segment` and reset at a window boundary. No drift accumulates between video and audio. |
| HLS correctness | UTF-8 BOM stripping, query/fragment-only URL resolution, zero-length byte-range rejection, saturating range continuation, full `Content-Range` validation, and a malformed `MEDIA-SEQUENCE` that keeps the last valid value. |
| Sound | PCM is never resampled to hide sync drift, so pitch never changes. Drift is absorbed by holding/releasing the clock. A wall-clock audio head re-anchors against published pictures with a tolerance and cooldown, instead of seeking on every frame. |
| Native audio | One Media Foundation source reader carries both audio and video on Windows; decoded PCM plays through `cpal` on one media timeline. `carries_audio()` prevents a decoder that already owns both tracks from being paired twice. |
| Lifecycle | `[SOURCE-VERIFIED]` `VideoPlayer::close()` raises a one-way shutdown flag, sends `Command::Shutdown` to the worker and **detaches** its thread (drops the `JoinHandle`) instead of joining it, so a decoder stuck in `read()` cannot hold `close()` or `Drop`. The worker still owns the `Box<dyn VideoDecoder>` and closes it when it returns. |
| Observability | New `SessionId`, `PlaybackEvent`, `EventRecord` and `PlaybackDiagnostics` types with a bounded 256-event ring and counters. Pull-based: `player.events()` and `player.diagnostics()`. No global logger, no callback, no change to `VideoState`. |
| Pixel path | Windows reads NV12 straight from the decoder instead of RGB32, so its video processor stops converting every picture. NV12 converts to RGBA across every core, into a pooled buffer, on the decode thread. A screen that samples two textures takes the planes untouched. |
| Planar everywhere | Every backend that can hand over 4:2:0 now does. Windows asks Media Foundation for NV12, Apple asks AVFoundation for `420v`, Android copies the codec's planes — including the interleaved chroma plane it used to refuse — and GStreamer's caps ask for NV12. |
| HLS layout | The HLS stream reports the layout of the decoder inside the open window, so a live NV12 stream plays. It used to report RGBA and then hand the request to a planar decoder, which failed on the first picture. |
| Run it | `cargo run -- <path or URL>` plays a source in a window with the controls and its sound, behind the default `player` feature. |
| Frame cost | No picture allocates: RGBA buffers and plane buffers both come back to the slot when the screen recycles a frame. |
| Diagnostics | `PlaybackDiagnostics::{decode_time, convert_time, pixel_format}` state what one picture costs and which layout it arrived in. |
| Platform fixes | Android JNI/MediaCodec field access and buffer lifecycle; Windows MF audio clock; container detection for MPEG-TS temp files. |
| Build hygiene | `SessionId` uses an `AtomicUsize` so wasm32 builds cleanly; the re-anchor telemetry hook is compiled only off wasm. |
| Review coverage | Eight static documents accompany the code: a cross-library technique catalog, a mobile-platform audit, a cross-platform UI/video review, a full Rust code/architecture review, two native-integration reviews (phone and desktop), a source-level comparison with the current upstream `cranpose-services` decoder, and a vendor-source hardware-decode reference. They are review-only — no device run is claimed — and their findings are recorded under "Problems solved in this update" and the verification matrix. |

The public additions are `VideoPlayer::{session, events, diagnostics, open_hls}`, `VideoDecoder::{carries_audio, sync_clock, close, observe_shutdown, attach_session, recover}` (all defaulted), the `with_decoder` constructor that already existed, and the telemetry types. A second, additive recovery wave exports `PlaybackSession`, `PlaybackState`, `InterruptionReason`, `SessionEvent`, `RECOVERY_COOLDOWN` and `RECOVERY_COOLDOWN_MAX` on non-Linux native targets: they are the bounded retry record the player and its decoders report into so a failed audio device, route change, or network read is retried instead of becoming terminal. Nothing was removed, and no existing signature, enum variant or public field changed. The recovery behaviour is `[SOURCE-VERIFIED]` in source and unit tests; an audible return on a real device is `[HYPOTHESIS]` until a device run is captured. A third, additive wave exports `start_log_file`, `stop_log_file`, `log_note`, `log_path` and `is_logging`: one opt-in file that records what a session delivered and every failure it swallowed.

### Review reports

Six reviews were run over the whole source tree and the surrounding ecosystem, plus a seventh source-level comparison with upstream Cranpose and an eighth reference built from official FFmpeg, mpv, Android and Apple documentation. All eight are **static**: they read code and public vendor sources, and none of them ran on a device or measured performance. They are kept as separate documents so a reader can check every claim against its `file:line` reference or source URL.

| Report | Question it answers |
| --- | --- |
| [`docs/technique-catalog.md`](docs/technique-catalog.md) | What Makepad, FFmpeg, GStreamer, mpv and VLC do that this crate can adopt additively — GPU completion fences, per-frame colour metadata, rejecting or repacking untrusted NV12, stream-change negotiation, and releasing GPU/Surface resources on drop. |
| [`docs/mobile-platform-audit.md`](docs/mobile-platform-audit.md) | Static Android, Apple, Linux and HLS findings — chroma order, BT.2020/PQ/HLG, clean aperture, and the thread/memory budget when many reels play at once. |
| [`docs/cross-platform-ui-video-review.md`](docs/cross-platform-ui-video-review.md) | Which native playback surface fits each OS. The project's decision is **native windows only**: Makepad `media`/`VideoFrameSession` with `YuvPlanes` is the integration path, with UniFFI bindings only as a language bridge to that same native surface. A Tauri/WebView or Electron wrapper is **excluded** — it is not a native window and would put a browser between the decoder and the screen. |
| [`docs/rust-code-architecture-review.md`](docs/rust-code-architecture-review.md) | Full source review: SEV1–SEV4 findings, a runtime-delay map, the architecture verdict, ranked speed techniques, and a phased implementation plan with verification commands. |
| [`docs/phone-integration-review.md`](docs/phone-integration-review.md) | How the library should work and draw video on Android and iOS: the current CPU/RGBA pipeline versus the target native flow (`MediaCodec` → `AHardwareBuffer`/Surface, `AVAssetReader` → `CVPixelBuffer`/Metal), surface ownership, thread/memory budget, audio sync, and the exact measurements needed to prove iOS. |
| [`docs/desktop-integration-review.md`](docs/desktop-integration-review.md) | How the library should work and draw video on Windows, macOS and Linux: MF→NV12→D3D11, AVFoundation→Metal, GStreamer→DMABUF/EGL into Makepad; pull-based slots, one decode thread, deferred conversion, and the `fence`/`reanchor`/`convert_time` measurement points before any GPU migration. |
| [`docs/upstream-decoder-comparison.md`](docs/upstream-decoder-comparison.md) | Where the current upstream decoder API actually lives, how its feed thread, bounded frame queue, zero-copy `ExternalSurface`/IOSurface paths, capability query and waker work, and which parts can be adopted additively without changing this crate's public API. |
| [`docs/hardware-decode-reference.md`](docs/hardware-decode-reference.md) | What the official FFmpeg, mpv, Android and Apple sources actually say about GPU/CPU/hybrid decode, copy-back cost, decoder/output pairing, and low-power high-load design — and which of those rules this tree already follows. |

**Windowing decision — native windows only.** The library is drawn through the platform's own surface and its own UI toolkit. Makepad is the reference integration: its `VideoFrameSession` consumes `YuvPlanes` or RGBA directly, so the decoder's output reaches the compositor without a browser in between. UniFFI is acceptable only as a language binding to that same native surface. Web views embedded in a native shell (Tauri, Electron, WebView2, WKWebView, Android `WebView`) are explicitly out of scope: they are not native windows, and they would add a canvas readback and a second compositor between the decoder and the screen. The `<video>` backend listed below is different — it is the browser's own native media path for a build that targets the web itself.

### How the library should work and draw video (native only)

Both integration reviews agree on one model: the player stays **pull-based** with a slot pool and one decode thread; NV12 is kept until the last moment; HLS is a separate layer; and the screen takes a frame, draws it, then recycles it. What changes per OS is only where the decoded picture crosses into the native surface:

| OS | Target native flow | Who owns the surface | Status in this tree |
| --- | --- | --- | --- |
| Windows | Media Foundation → NV12 → D3D11 texture → Makepad | Decoder/session owns it until the frame is recycled | Source-confirmed RGBA/CPU path; GPU texture intake is an untested add-only proposal |
| macOS | AVFoundation → `CVPixelBuffer`/IOSurface → Metal texture → Makepad | Session owns the reader and buffer lifetime | Source-confirmed NV12/BGRA CPU copy; no GPU intake here |
| Linux | GStreamer → DMABUF/EGL/VAAPI → Makepad | Sink/session owns the buffer | Source-confirmed NV12 `appsink` copy; DMABUF path untested |
| Android | `MediaCodec` → Surface/`AHardwareBuffer` → Makepad texture intake (or near-term normalised NV12 sampled by GPU) | Codec/session owns the Surface across `flush`/`seek`/window change | Source-confirmed `null` Surface + CPU NV12 copy + per-frame `ImageBitmap`; native flow is a hypothesis |
| iOS | `AVAssetReader` → `CMSampleBuffer`/`CVPixelBuffer` → Metal texture → Makepad | Session owns the reader; `cancelReading` on `open_at`/close | AVFoundation path exists and is supported; reader cancellation is source-fixed but device-unverified; no GPU intake |

Two facts gate every native-flow change: **do not return a buffer to the pool before GPU completion** (a fence is the measurement point), and **do not assume chroma order or colour range** on Android/iOS — carry them explicitly or the picture shows swapped channels or wrong colour before any speed gain. The full reasoning, ownership rules and phased plans are in the two integration reports.

**[SOURCE-VERIFIED current]** `pixels.rs:33` sets `PARALLEL_MIN_PIXELS = 3840 * 2160`; the guard at `pixels.rs:190` is `width * height < PARALLEL_MIN_PIXELS`. So **720p and 1080p both stay on one core** — splitting begins at **4K**, not 1080p. Earlier reviews claimed a 1080p threshold; that number was superseded after repeated release runs showed forcing fan-out at 1080p was not reliably faster (0.86×–1.14×) while 4K was consistently faster (~1.23×–1.41×). Android **does** read `rowStride`/`pixelStride` and colour data; the open Android question is chroma order, clean aperture, and BT.2020/full-range coefficients, not a missing stride read.

---

## Showing a video

### Level 1 — one composable

One composable plays a file. It opens the source, decodes it on the media stack of the platform, draws the picture and takes a new frame on every frame of the window.

```rust,no_run
use cranpose_ui::Modifier;
use cranpose_video::Video;

fn screen() {
    Video("clip.mp4", Modifier::empty().fill_max_size());
}
```

`VideoWithControls` adds the clock and the buttons that drive it, so a screen needs no player of its own:

```rust,no_run
use cranpose_ui::Modifier;
use cranpose_video::VideoWithControls;

fn screen() {
    VideoWithControls("a.mp4", Modifier::empty().fill_max_width());
    VideoWithControls("b.mp4", Modifier::empty().fill_max_width());
}
```

A screen that wants its own controls takes a handle from `rememberVideoPlayer`, draws the picture with `VideoFrame`, and composes its own widgets beside it. One handle drives both. The handle opens the file once, keeps the position across recompositions, and closes the player when the composable leaves.

```rust,no_run
use cranpose::{composable, prelude::*};
use cranpose_video::{VideoFrame, VideoPlayerHandle, rememberVideoPlayer};
use std::time::Duration;

// `rememberVideoPlayer` opens the source ONCE (keyed by source) and returns a
// VideoPlayerHandle. It does NOT draw anything by itself.
// `VideoFrame(handle, ..)` draws the picture for THAT handle — so the picture
// and your custom controls share one player. (`Video(source, ..)` is the
// shortcut that opens a private handle when you need no custom controls.)
#[composable]
fn MyPlayer(source: &'static str) {
    let handle: VideoPlayerHandle = rememberVideoPlayer(source);

    // Draw the picture for this handle. The composable owns the frame pump
    // (`take_frame -> ImageBitmap`) internally — no manual thread needed.
    VideoFrame(handle.clone(), Modifier::empty().fill_max_width());

    // The handle is a CONTROL surface: it drives the player while `VideoFrame` draws.
    // Every method returns Result except pure getters.
    let _ = handle.play();                                   // start / resume
    let _ = handle.pause();                                  // hold the picture
    let _ = handle.toggle();                                 // play <-> pause
    let _ = handle.seek(Duration::from_secs(10));            // absolute position
    let _ = handle.seek_fraction(0.5);                       // 0.0..=1.0 of media
    let _ = handle.set_speed(1.5);                           // playback rate
    let _ = handle.set_looping(true);                        // repeat at end
    let _ = handle.set_muted(true);                          // silence backend audio
    let _ = handle.set_volume(0.8);                          // 0.0..=1.0

    // Pure getters (no Result):
    let _pos: Duration = handle.position();
    let _dur: Option<Duration> = handle.duration();
    let _speed: f32 = handle.speed();
    let _muted: bool = handle.is_muted();
    let _volume: f32 = handle.volume();
    let _looping: bool = handle.is_looping();
    let _state = handle.state();                             // VideoState
    let _info = handle.info();                               // VideoInfo
    let _size = handle.decoded_size();                       // live resolution
}
```

### VideoPlayerHandle — ملخص الدوال

| الدالة | المعنى |
|---|---|
| `play` / `pause` / `toggle` | تشغيل / إيقاف مؤقت / تبديل |
| `seek(Duration)` / `seek_fraction(f32)` | قفزة زمنية / بنسبة من المدة |
| `set_speed(f32)` / `speed()` | السرعة |
| `set_looping(bool)` / `is_looping()` | التكرار |
| `set_muted(bool)` / `is_muted()` | الكتم |
| `set_volume(f32)` / `volume()` | مستوى الصوت |
| `position()` / `duration()` | الموضع / المدة (None للبث الحي) |
| `state()` / `info()` / `decoded_size()` | الحالة / معلومات الملف / أبعاد الإطار الحالية |

القواعد:
- `rememberVideoPlayer(source)` يفتح مفكّكاً واحداً في موقع الاستدعاء، ويُعيد استخدامه ما دام المصدر نفسه — تغيير المصدر يُسقط القديم ويغلق خيوطه.
- `VideoFrame(handle, ..)` هي التي ترسم وتضخّ الإطارات لهذا المقبض — لا تحتاج كتابة `LaunchedEffect` بنفسك.
- استخدم **مقبضاً واحداً** للصورة ولأزرارك: `VideoFrame(handle, ..)` + دوال `handle`. لا تخلط `Video(source, ..)` مع `rememberVideoPlayer(source)` — كل منهما يفتح مفكّكاً مستقلاً.
- المقبض يُغلق تلقائياً عند خروج الـ composable.

The `widget` feature carries this module and is on by default. A screen that only decodes frames turns it off and uses `VideoPlayer` below, which pulls no UI crates.

---

## Run it

```sh
cargo run -- https://live-hls-web-aja.getaj.net/AJA/index.m3u8
cargo run -- "C:\clips\a.mp4"
```

The `player` feature, on by default, builds `src/main.rs`: one window, the picture filling it, the clock and the buttons under it, and the sound the source carries. With no argument it plays `tests/fixtures/clip.mp4`, so a fresh checkout shows a picture without a file of its own. A build that only decodes frames turns the feature off with `--no-default-features --features widget`.

---

## Backends

A build selects one backend from the target's operating system at compile time.

| Target | Backend | Decoded by |
| --- | --- | --- |
| Windows | Media Foundation source reader | Platform decoder into NV12; RGB32 with its video processor only for a track that asks for a turn |
| macOS, iOS | AVFoundation `AVAssetReader` | Platform decoder into NV12 (`420v`), which is its native output; BGRA only when a track refuses planes |
| Android | `MediaExtractor` + `MediaCodec` | Platform decoder into a YUV420 image, copied into NV12 planes here |
| Web | Detached `<video>` element and 2D canvas | Browser media pipeline, drawn to a canvas and read back; HLS through the browser or `hls.js` |
| Linux | GStreamer, behind the `linux-gstreamer` feature | `uridecodebin`, `videoconvert` into NV12, `videoflip` and an `appsink`, from the plugins the system installs |
| watchOS | none | `AVAssetReader` is absent from watchOS, so `VideoPlayer::open` returns `VideoError::Unsupported` |

`platform()` names the selection, `decoder_name()` gives its name for a log, and `is_supported()` states whether the build decodes anything.

The web has no thread to decode on and nothing to block a frame on, so there the browser keeps the clock: the element plays at the playback rate and `take_frame` reads whichever picture the element shows. Every other backend decodes ahead on a thread of its own.

### Which systems this update touched

| System | Backend existed before? | Direct native-code edit in this update? | What it receives |
| --- | --- | --- | --- |
| **Windows** | Yes, Media Foundation | **Yes** | Single source reader for audio+video, pitch-preserving float PCM through `cpal`, MF clock sync, and the shared changes below. |
| **Android** | Yes, MediaCodec | **Yes** | JNI `BufferInfo` fields read as fields, output/input buffer lifecycle, JNI exception checks, timeouts, and the shared changes below. |
| **iOS** | **Yes, AVFoundation (`src/platform/apple.rs`)** | **Yes — one small reader-lifecycle fix** | Shared changes plus `cancelReading()` before replacing the reader in `open_at`; the source fix is unverified on an iOS device. |
| **macOS** | Yes, AVFoundation | No | The same shared changes as iOS. |
| **Linux** | Yes, GStreamer, feature-gated | No | The same shared changes, plus the defaulted decoder hooks for application-supplied decoders. |
| **Web / wasm** | Yes, `<video>` + canvas | No | Shared telemetry and decoder hooks; wasm-specific build fixes for the new code. |
| **watchOS** | No decoder | No | Unchanged. `open` still returns `VideoError::Unsupported`; a custom decoder still works. |

**Direct answer about iOS:** iOS was already a supported platform before this update and it is still supported. This update made one direct native iOS change: `open_at` now calls `cancelReading()` before replacing the old `AVAssetReader`, matching `close`, so a seek no longer drops a reader that is still decoding old media. The rest of the Apple backend is unchanged. The iOS build also gains every shared improvement (HLS, audio sync, temp files, lifecycle, telemetry) because those modules compile for `target_os = "ios"`. No iOS device run was performed, so the fix is **source-confirmed but device-unverified**. The Android changes are type-checked and have a device checklist in `docs/android-test-checklist.md`, but also await a real-device run.

The review reports do not add another direct iOS code change. They still list Apple-specific findings: the colour matrix/range is guessed from the size instead of read from the format description, and planes are copied from raw surface dimensions without a `clean aperture` (`apple.rs:280-334`). The `cancelReading` asymmetry is no longer open in source: `apple.rs:154-162` now cancels before replacing the reader, matching `close` at `apple.rs:369-373`. Those remaining items are **documented findings, not fixes in this update**, and no iOS device run was performed. What would close the gap is an Apple counterpart to `docs/android-test-checklist.md`: probe an HDR and a padded-resolution source, log the reported colour metadata and display size, and seek repeatedly to check that the old reader no longer stalls the new one.

---

## Pixels: NV12, RGBA and the GPU

A hardware decoder produces 4:2:0 in NV12, and every GPU can sample it as two textures. Turning it into RGBA costs a pass over the whole picture, which is the one per-frame cost this crate controls. The layout a frame carries is therefore a choice the screen makes, and both answers are supported.

`VideoFrame::format()` states what arrived. An NV12 frame keeps its planes, and converts once — across every core, in fixed-point integer maths — the first time something asks for `rgba()` or `into_rgba()`. `luma()` and `chroma()` hand the planes over untouched.

| Who draws | Setting | What one 1080p picture costs |
| --- | --- | --- |
| A screen that uploads RGBA | default | one conversion on the decode thread, into a buffer from the pool |
| A screen that samples NV12 in a shader | `player.set_native_nv12(true)` | nothing: the planes are handed over as they are |
| A decoder that already produces RGBA | none | nothing: the frame arrives ready |

The conversion splits the rows of a picture across the cores only from 4K upward (`PARALLEL_MIN_PIXELS = 3840 * 2160`); 720p and 1080p stay on one core because the extra threads cost more CPU than they save (`CRANPOSE_VIDEO_WORKERS` pins the count for a measurement). The override is a *choice within* the safety ceiling, not a new ceiling: values are parsed, non-positive/garbage values ignored, and anything above `MAX_WORKERS = 8` is clamped down; automatic detection is clamped to `1..=8`. It writes into a buffer the slot recycles, so a long playback allocates no picture after the first second.

**[REPORT-ONLY]** Measured in an earlier session on a 4-core Windows host with release H.264 fixtures and the real Media Foundation decoder; the cited `probe-final-after-tables.log` is not present in this checkout. Each number is the average cost reported by one 6 s `stutter_probe` run, not a worst case and not a device result:

| Path | Average decode | Average pooled RGBA conversion |
| --- | --- | --- |
| 720p60, no native audio | 0.71 ms | 4.55 ms |
| 720p60, native audio | 0.67 ms | 5.12 ms |
| 1080p60, no native audio | 1.44 ms | 6.65 ms |
| 1080p60, native audio | 1.48 ms | 6.74 ms |
| NV12 handed over as planes, no conversion | — | no work |

**[REPORT-ONLY]** The earlier RGB32/video-processor path cost an order of magnitude more than decode, which is why Windows asks for NV12 first. A screen that uploads RGBA still pays the conversion, visible in `diagnostics().convert_time()`. The 1080p margin is thinner than 720p: the same final 1080p60 native run reported these averages but still had two long gaps, so average cost alone does not prove smooth presentation under host load.

**[REPORT-ONLY]** **GPU renderer note.** Decoding and audio do not require wgpu. The `player` feature enables the app-facing Cranpose renderer (`cranpose/renderer-wgpu` + `cranpose/desktop`), which is why an app using the ready-made widgets opens a GPU device; using the crate only for `VideoPlayer` frames does not. Cranpose 0.17.2 already used the `cranpose-wgpu` fork, so the 0.24.0 upgrade did not introduce wgpu itself. The startup failure seen on one legacy GLES host after upgrading to 0.24.0 comes from device limits (`65535` requested, `0` allowed) and a later GLES pipeline-compilation deadlock; it is diagnosed with source citations in `docs/final-report-ar.md` §6. That is an app/backend configuration issue, not a change to this crate's decoder path.

## HLS

A `.m3u8` address is a playlist of small fragments, not one file, and the native decoders open one file. So the crate reads a playlist itself: it fetches the master playlist, picks the tallest rendition the playlist offers (including 4K and 8K — there is no fixed resolution cap), reads the media playlist, and joins its initialization section and first fragmented-MP4 segment into a temporary file so decoding can start without waiting for a second segment. Later windows keep two segments buffered; MPEG-TS, which has no separate initialization section, keeps its existing two-segment startup window. When playback reaches the end of a window, it fetches the playlist again, joins the next window and reopens the decoder, so a live stream keeps playing as the playlist slides forward.

```rust,no_run
use cranpose_video::VideoPlayer;

fn watch(url: &str) -> Result<(), cranpose_video::VideoError> {
    let player = VideoPlayer::open(url)?;
    // A live stream states no length and cannot be seeked.
    if player.is_live() {
        assert_eq!(player.duration(), None);
        assert!(!player.is_seekable());
    }
    player.play()
}
```

The window is deliberately short. A live playlist drops fragments that are older than its window, so a decoder that holds the whole stream would stall on a fragment the server no longer has; joining a few segments and reopening keeps the reader inside the playlist's own window. A fragmented MP4 stream numbers its fragments from the start of the broadcast, and those numbers are rewritten as the window moves, so the platform decoder sees a file that starts at zero.

### Renditions follow the measured link

A master playlist offers several renditions with their stated bitrates. The crate measures the rate every fetched fragment actually arrived at, keeps a weighted estimate of it, and chooses the rendition again when a window ends. The end of a window is the one moment the decoder is closed anyway, so a switch costs no stall. The choice keeps a quarter of the measured rate spare for the container's overhead and the bursts inside a fragment, moves up only into that headroom, and stays on the rendition in play while it still fits, so a measurement that wobbles does not flicker between pictures. The slowest fragment of a window is the measurement used, and the estimate is fed by the background prefetch worker too, so a stream that is always decoded from cached bytes still adapts. A switch to a rendition whose playlist cannot be read leaves the stream on the one it has: a bad measurement never stops the picture. A fixed playlist keeps the exact position across a switch; a live stream rejoins at the live edge.

`is_live()` is true when the playlist states no `#EXT-X-ENDLIST` at open, and stays so for the life of the player. On such a stream `duration()` is `None`, `seek`, `seek_fraction` and `stop` report `VideoError::UnsupportedOperation`, and `play` cannot restart one that ended. A playlist with `#EXT-X-ENDLIST` is treated as a file: it is fully seekable, `duration()` is the sum of its segments, and a seek opens a window at the target segment.

A stream whose segments are encrypted with `#EXT-X-KEY` or `#EXT-X-SESSION-KEY` is refused at open with `VideoError::UnsupportedOperation`, because the crate carries no key handling. MPEG-TS segments without an initialization section are joined as one transport stream.

The fetch path uses at most four workers at a time, writes each result back to its playlist index so order never depends on which request finishes first, scales each fragment's deadline to four times its declared duration (clamped to 20–120 s), and prefetches the next window off-thread — including a separate audio rendition. The smaller first fragmented-MP4 window reduces the amount of media that must arrive before the first decode; the normal two-segment window and MPEG-TS startup are unchanged. Prefetch is generation-stamped, so a seek, a close or a new window invalidates stale bytes. A shutdown flag is checked at every blocking step, including retry sleeps, and body reads are chunked and cancel-aware.

In the browser, `open` keeps the address as it is and the element plays it. Safari and iOS play HLS on their own; a browser that cannot loads `hls.js` from jsDelivr once per page and points it at the element. The playlist and its fragments are fetched by the browser, so the server has to send `Access-Control-Allow-Origin` for a playlist on another host, which is why the element carries `crossorigin="anonymous"` for HLS.

---

## Sound

`set_muted` and `set_volume` always record the state so an application can use one control for every platform. What they act on depends on the build: with `native-audio` enabled, or on Windows where Media Foundation carries both tracks, they silence and set the volume of the player's own sound; on the web, where the browser plays the media in its own element, they mute and set the volume of that element. Without an audio backend they simply record the state, so an application can pair sound separately where needed.

### Native audio (opt-in feature)

The `native-audio` feature plays a source's own audio on native platforms with the same in-process backend the rest of the app uses:

```toml
cranpose-video = { path = "vendor/cranpose-video", features = ["native-audio"] }
```

On Windows, one Media Foundation source reader demuxes and decodes both tracks of the source, so the picture and its sound come from the same file, URL or HLS window. This covers every container and codec the system player supports, including MPEG-TS. The decoded PCM is rendered through `cpal` and both clocks are rebased to the same media timeline. **`carries_audio()` tells the player that this decoder already owns both tracks, so the paired path steps aside and the same sound is never played twice.**

On the other native platforms, `VideoPlayer::open` pairs the picture decoder with an audio player that reads the same bytes, and for HLS the audio follows the picture onto one timeline:

- When the stream carries its audio inside the fragmented MP4 segments, both decoders read the same joined window file.
- When a master playlist offers a separate audio rendition (`#EXT-X-MEDIA:TYPE=AUDIO`), the one named for the chosen video rendition is followed. Its init section and the segments covering the same span of the stream are joined into their own file, lined up by media sequence number, and played alongside the picture. This is what makes streams that ship video and audio on separate tracks audible.

**Pitch is never traded for sync.** The audio cursor advances by playback speed on an integer-nanosecond timeline; PCM samples are never stretched or resampled to close a gap. Small drift is absorbed by holding or releasing frames in the MF clock, and short fades remove clicks at those boundaries. On platforms whose audio backend cannot report a position, the audio head is estimated from wall time and only re-anchored when the gap exceeds 250 ms, with a 2 s cooldown — no per-frame seeking, no pitch change, no pulsing.

**Default output changes (Windows MF + `cpal`).** Plugging in headphones or removing the active output makes `cpal` report `StreamInvalidated` or `DeviceNotAvailable` in the stream error callback. That callback only sets an `Arc<AtomicBool>`; no COM object or media state crosses threads. On the next video read the decode thread checks the flag and rebuilds the `cpal` output stream on the new default device. When the new endpoint reports the same channels, rate and sample format, it keeps the same `Arc<AudioCore>` — reader, decoded PCM, play head and clock — so Media Foundation is **not** renegotiated and no audio is retimed. When it reports another rate or channel count, which is the usual case when switching between a headset and the built-in speakers, the reader is asked for float PCM in the new layout and the sound moves to a fresh core that carries the play head, mute and volume across; a different sample format alone only rebuilds the stream, because the PCM is already float. The first block after a renegotiation is mapped onto the play head, exactly like a fragment join, so the sound does not wait for a timestamp that restarted at zero. The old stream is released on a thread of its own, so an endpoint that blocks while it is torn down cannot stall the decode thread or the picture. Refusing a device whose layout differs used to leave the stream bound to the invalidated endpoint and silence both devices until the source changed; that refusal is gone. Failed rebinds are retried after a 500 ms cooldown, which covers a device that appears slightly after the old one disappears; every failure is swallowed so audio trouble never breaks the picture, and a failing run logs one warning instead of one per retry. See `src/platform/windows/mf_audio.rs` (`is_device_change`, `rebind`, `renegotiate`, `pump`). The layout decision and the play-head hand-over are unit-tested; an audible switch on real hardware is still unmeasured here. Web audio is routed by the browser's own element, and Android route changes remain OS/platform behaviour that has not been measured here.

Sound is best effort: a source with no audio track, a playlist whose audio cannot be fetched, or a machine with no output device simply plays no sound and never fails the picture or `open`. Where the paired audio backend is used it reads fragmented MP4, so MPEG-TS audio there stays silent; on Windows the Media Foundation path above handles it. The feature pulls in `cranpose-media` and `cranpose-services`, so a build that only draws pictures can leave it off and pay nothing for an audio stack. It has no effect on the web, where the element supplies sound itself.

Native audio starts unmuted at full volume. Web still starts muted: browsers do not start sound without a person's interaction, so sound there is opt-in. Call `set_muted(false)` from a click or key handler, or the browser may refuse to play it. Setting the volume does not unmute by itself.

```rust,no_run
use cranpose_video::VideoPlayer;

fn start_sound(player: &VideoPlayer) -> Result<(), cranpose_video::VideoError> {
    // Call from a click or key handler on the web.
    player.set_muted(false)?;
    player.set_volume(0.8)
}
```

`is_muted()` and `volume()` read the current state. `VideoPlayerHandle` from the `widget` feature exposes the same four methods, so a composable can offer a mute control without holding the player itself.

---

## Level 2 — opening and controlling a player directly

```rust,no_run
use cranpose_video::{VideoPlayer, VideoState};
use std::time::Duration;

fn play(path: &str) -> Result<(), cranpose_video::VideoError> {
    let mut player = VideoPlayer::open(path)?;
    player.play()?;

    let info = player.info();
    println!("{}x{} at {:?}", info.width, info.height, info.frame_rate);

    player.seek(Duration::from_secs(30))?;
    player.set_speed(0.5)?;
    player.toggle()?;
    assert_eq!(player.state(), VideoState::Paused);

    player.close();
    Ok(())
}
```

`open` accepts a path, a URL, or a byte buffer. `VideoSource::{path, url, bytes}` are the explicit constructors, and `&str`, `String`, `&Path`, `PathBuf`, `Vec<u8>` and `Arc<[u8]>` all convert into it. A byte buffer reaches a backend that reads only files through a temporary file, removed when the player closes. `VideoPlayer::with_decoder` takes an application's own `VideoDecoder` instead, which is how a format the platform lacks is read, and how Linux decodes without the GStreamer feature.

Every control returns `Result<(), VideoError>`: `play`, `pause`, `toggle`, `stop`, `seek`, `seek_fraction`, `set_speed`, `set_muted`, `set_volume`. `[SOURCE-VERIFIED]` `close(&mut self)` raises the shutdown flag, signals the worker and detaches its thread; it does not join. The worker closes the decoder it owns when it finally returns, and `Drop` calls `close` for you. The caller never reclaims or drops that decoder.

---

## Level 3 — pull-based frames and recycling

The player is pull-based. A decode thread decodes ahead of the clock and publishes the newest picture into a slot. The screen asks for it while it draws, then hands it back so the buffer decodes the next frame.

```rust,no_run
# use cranpose_video::{VideoError, VideoPlayer};
# use std::time::{Duration, Instant};
# fn upload(_: u32, _: u32, _: &[u8]) {}
# fn present() {}
fn render_loop() -> Result<(), VideoError> {
    let mut player = VideoPlayer::open("clip.mp4")?;
    player.play()?;

    let frame_time = Duration::from_millis(16);
    let mut next = Instant::now();
    loop {
        if player.state().is_ended() {
            break;
        }
        if let Some(frame) = player.take_frame() {
            upload(frame.width(), frame.height(), frame.rgba());
            present();
            player.recycle(frame);
        }
        next += frame_time;
        let now = Instant::now();
        if next < now {
            next = now;
        }
        std::thread::sleep(next - now);
    }
    player.close();
    Ok(())
}
```

A 60 Hz screen never waits on a decoder, and a picture is allocated once instead of once per frame. A paused player still decodes one picture after opening and one after each seek, so a screen shows the frame it is paused on. Dropping a frame instead of recycling it also works; it only costs the buffer.

---

## Rotation

A phone records landscape frames and states the turn in the track. `VideoInfo::rotation` reports that turn, `VideoInfo::display_size` gives the size a viewer sees, and the frames stay as the decoder produced them. Windows and Linux apply the turn themselves, because their own pipelines do it, and report `Rotation::None` there.

---

## Position and duration

`position()` moves between published pictures at the playback speed, so it is smooth rather than a step per frame. It never runs past the picture on screen, so a decoder that falls behind holds the clock back. `duration()` returns the length the container states, or `None` for a live stream.

Late frames from a network source no longer cause a stale picture: a frame that arrives after its media time re-anchors the clock and is shown. The re-anchor is recorded as a `Reanchored` event and counted in `reanchors()`, so a diagnostic build can tell an occasional network correction from a decoder that cannot keep up.

---

## Level 4 — observability and diagnostics

The update adds a pull-based telemetry layer. It never logs globally, never spawns a thread, and never changes `VideoState`. Events are kept in a bounded 256-record ring; numbers keep increasing after old records are dropped, so a gap tells you how much was missed. Frames are counted, not logged.

```rust,no_run
use cranpose_video::{PlaybackEvent, VideoPlayer};

fn report(player: &VideoPlayer) {
    let d = player.diagnostics();
    println!(
        "{} state={:?} position={:?} frames={} reanchors={} errors={} seeks={} uptime={:?}",
        d.session(),
        d.state(),
        d.position(),
        d.frames_published(),
        d.reanchors(),
        d.errors(),
        d.seeks(),
        d.uptime(),
    );

    for record in player.events() {
        match record.event() {
            PlaybackEvent::Reanchored { position } => {
                println!("#{} at {:?}: re-anchor to {:?}",
                    record.sequence(), record.at(), position);
            }
            PlaybackEvent::Failed { error } => {
                println!("#{} failed: {:?}", record.sequence(), error);
            }
            _ => {}
        }
    }
}
```

Other events are `Opened`, `Playing`, `Paused`, `Seeking { position }`, `Ended`, `InfoReady { width, height }`, `SpeedChanged { speed }`, `Muted { muted }`, `VolumeChanged { volume }` and `Closed`. `session()` identifies the player across recreations. `PlaybackDiagnostics` also exposes `duration`, `speed`, `is_live`, `is_seekable`, `is_muted`, `volume`, `info`, `plays` and `event_count`.

### One file per session: the unified playback log

Everything the player swallows — a busy output device, a source with no audio track, an HLS request that answered 403 — is invisible to a caller that only watches `state()`. `start_log_file` installs a sink that writes one line per event, stamped in UTC and tagged with the thread that produced it, so a field report can be attached instead of described:

```rust,no_run
use cranpose_video::{log_note, start_log_file, stop_log_file};

fn session() -> std::io::Result<()> {
    start_log_file("playback.log")?;
    log_note("INFO", "app.screen", "the reels feed opened clip 12");
    // Open and play; the crate writes what it did, in order.
    stop_log_file();
    Ok(())
}
```

A real session on Windows (`examples/av_probe.rs`, 1080p30 with sound) writes:

```text
2026-11-20T12:52:24.475Z INFO  player.source      [main] opening file target/bench/b1080p30a.mp4
2026-11-20T12:52:24.548Z INFO  player.open        [main] decoder=Media Foundation 1920x1080 live=false seekable=true format=Nv12 muted=false volume=1
2026-11-20T12:52:24.641Z INFO  mf_audio.setup     [cranpose-video] audio track open: 2 channel(s) at 48000 Hz
2026-11-20T12:52:24.646Z INFO  mf_audio.sample    [cranpose-video] the first decoded block of 2160 float sample(s) was buffered at 0ns
2026-11-20T12:52:24.650Z INFO  mf_audio.render    [cpal_wasapi_out] the output device started pulling PCM
2026-11-20T12:52:24.958Z INFO  player.video       [cranpose-video] the first picture was published
2026-11-20T12:52:29.555Z INFO  player.close       [main] frames=58 errors=0 seeks=0 reanchors=31 position=1.9892577s state=Playing
```

Those six lines answer the questions a report asks: the video sample arrived (`player.video`), the audio track was opened (`mf_audio.setup`), a decoded PCM block existed (`mf_audio.sample`), the device actually pulled it (`mf_audio.render`), and the session's counters are in `player.close`. An HLS session adds the playlist it chose and every window it joined, including whether the sound came with the window or from a separate audio playlist:

```text
2026-11-20T12:54:53.555Z INFO  hls.open   [cranpose-video] playlist https://test-streams.mux.dev/…/193039199_mp4_h264_aac_fhd_7.m3u8 holds 64 segment(s): live=false separate_audio=false start_index=0, chosen in 323.1214ms
2026-11-20T12:55:01.613Z INFO  hls.window [cranpose-video] joined 2 segment(s) from index 0 at 0ns (18496568 bytes), audio=decoder-owned, ready in 8.0577048s
```

`log_note` writes the application's own markers into the same file, `log_path` and `is_logging` read the state, and `stop_log_file` closes it and removes the sink. Lines from the library and from the application share one stamp, thread tag and URL redaction: a token in a live address never reaches the file. Logging is opt-in and additive — with no file open the crate writes nothing and behaves exactly as before. It replaces any sink the application installed with `set_diagnostic_sink`; install yours first if you want both, or forward from `log_note` instead.

**Source-verified, not a device measurement:** the `Playing` *event* is gated on the first published frame, but `state()` is not. `play()` sets `VideoState::Playing` immediately (`src/player.rs:354-357`); when no frame exists yet it defers the event through `playing_pending` (`src/player.rs:358-362`) and releases it in `note_frame()` once a real picture is published (`src/player.rs:457-468`). `tests/first_play.rs:82-99` locks this in: the clock/state are `Playing` at once while the `Playing` event is absent until the first picture. A screen that follows the *event* therefore never announces a blank frame as playing; a screen that follows `state()` sees `Playing` right away. This is source-level behaviour confirmed by a unit test, not a runtime observation on a device.

---

## Level 5 — native audio with mute and volume

Enable the feature, then drive the same controls on every platform. On web, call `set_muted(false)` from a user gesture.

```rust,no_run
use cranpose_video::{VideoPlayer, VideoState};
use std::time::Duration;

fn play_with_sound(path: &str) -> Result<(), cranpose_video::VideoError> {
    let mut player = VideoPlayer::open(path)?;
    println!("session {} — decoder {}", player.session(), cranpose_video::decoder_name());

    player.set_volume(0.75)?;
    player.set_muted(false)?;
    player.play()?;

    std::thread::sleep(Duration::from_millis(500));
    let d = player.diagnostics();
    if d.errors() > 0 || matches!(d.state(), VideoState::Failed(_)) {
        println!("audio may be unavailable; picture continues: {:?}", d.state());
    }

    player.close();
    Ok(())
}
```

---

## Level 6 — a custom `VideoDecoder`

`with_decoder` is the escape hatch for formats the platform does not read. Only `info`, `seek` and `read` are required. The four hooks added in this update all have defaults. `[SOURCE-VERIFIED]` `close()` detaches the decode thread and never joins it, so an uncooperative `read()` cannot block the caller; override `observe_shutdown` so a blocking decoder notices the shutdown flag and leaves its own read promptly, releasing its device and worker sooner. A read that returns after shutdown publishes nothing, and `Closed` stays the last event.

```rust,no_run
use std::{
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    time::Duration,
};
use cranpose_video::{
    DecodedFrame, FrameRate, Rotation, VideoDecoder, VideoError, VideoInfo, VideoPlayer,
};

struct SolidColor {
    info: VideoInfo,
    position: Duration,
    done: bool,
    shutdown: Arc<AtomicBool>,
}

impl SolidColor {
    fn new(width: u32, height: u32, frames: u32, fps: u32) -> Self {
        Self {
            info: VideoInfo {
                width,
                height,
                duration: Some(Duration::from_secs_f64(f64::from(frames) / f64::from(fps))),
                frame_rate: Some(FrameRate { numerator: fps, denominator: 1 }),
                rotation: Rotation::None,
            },
            position: Duration::ZERO,
            done: false,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl VideoDecoder for SolidColor {
    fn info(&self) -> VideoInfo {
        self.info
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        self.position = position.min(self.info.duration.unwrap_or(Duration::MAX));
        self.done = false;
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        if self.done || self.shutdown.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let (w, h) = (self.info.width, self.info.height);
        rgba.resize(w as usize * h as usize * 4, 0);
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[40, 120, 200, 255]);
        }
        let frame = DecodedFrame {
            pts: self.position,
            width: w,
            height: h,
        };
        self.position += self.info.frame_rate.unwrap().frame_duration().unwrap();
        if Some(self.position) >= self.info.duration {
            self.done = true;
        }
        Ok(Some(frame))
    }

    fn observe_shutdown(&mut self, flag: Arc<AtomicBool>) {
        self.shutdown = flag;
    }
}

fn main() -> Result<(), VideoError> {
    let decoder = SolidColor::new(64, 48, 30, 15);
    let player = VideoPlayer::with_decoder(decoder)?;
    player.play()
}
```

A decoder that renders both picture and sound from one clock should override `carries_audio() -> true`. One that owns the platform clock should override `sync_clock(position)`, and one with teardown order should override `close()`.

---

## Errors

A failure reaches the caller twice. The call that fails returns it, and `state()` holds `VideoState::Failed` with a copy, so a screen that only polls the state still shows why playback stopped. The decoder runs on its own thread, so a platform that refuses a seek reports it through the state rather than from `seek`. The same failure is recorded as a `Failed` event and counted in `errors()`.

---

## Problems solved in this update

| Problem | Fix |
| --- | --- |
| Android opened but showed nothing for ~30 s | `MediaCodec.BufferInfo` has **public fields**; the old `getInteger`/`getLong` calls threw, the exception was swallowed and the values became `0`. Fields are now read with `get_field`, and protocol-critical values propagate as `VideoError::Decode` instead of silently becoming zero. |
| Android leaked output buffers | The output buffer is now released even when copying the decoded picture fails. |
| Android left pending JNI exceptions | Checks and `dismiss` handling were added around JNI calls, with typed `int_value`/`long_value`/`optional<T>` helpers. |
| Android leaked input buffers | Sample read/growth, input preparation and return were split, with exactly one final `queueInputBuffer` per sample. |
| Duplicate audio | `carries_audio()` lets a decoder that demuxes both tracks opt out of the paired audio path. |
| Pitch changed when the clocks drifted | Drift is no longer absorbed by resampling PCM. The cursor advances at `speed` on integer nanoseconds; MF holds/releases frames with short declick fades. |
| Late network frames were skipped, leaving a stale picture | Late frames re-anchor the clock and are shown. `MAX_LATE_SKIPS`/`late_skips` no longer drop them. |
| HLS timestamps drifted across a window | One `tfdt` base per track and per-window rebasing, rather than one delta shared between tracks. Discontinuity resets the origin. |
| Encrypted HLS failed late or unclearly | `#EXT-X-KEY` / `#EXT-X-SESSION-KEY` streams are refused at open with `UnsupportedOperation`. |
| Playlists with a UTF-8 BOM, or relative URLs with only a query/fragment, parsed wrong | BOM stripping and a scheme-aware `resolve()`/`has_scheme()` pair. |
| Malformed `MEDIA-SEQUENCE` or zero-length byte range corrupted a window | The last valid sequence is retained, zero-length ranges are rejected, continuation uses `saturating_add`, and both `Content-Range` bounds are validated. |
| Slow networks stalled HLS | Parallel bounded fetch, index-preserving placement, adaptive deadlines, and next-window prefetch (including separate audio). |
| Prefetched bytes survived a seek or window change | Generation-stamped cache invalidated on seek, close and window rollover. |
| `close()` hung on a network retry | `[SOURCE-VERIFIED]` `close()` raises a shared shutdown flag, sends `Command::Shutdown` and detaches the worker instead of joining it; the flag is also handed to decoders and checked inside blocking reads and retry sleeps so a cooperative worker exits on its own. A read landing after shutdown returns before publication, clock work or events, and `Closed` is sealed as the last event. |
| MPEG-TS temp files were mislabeled `.mp4` | Container sniffing now checks sync bytes at offsets 0, 188 and 376 with a lead-in. |
| No way to explain a stutter after the fact | Pull-based events and counters via `events()` and `diagnostics()`, without touching `VideoState`. |
| **[REPORT-ONLY]** An earlier Windows RGB32/video-processor path spent 79 ms a frame | The decoder asks for NV12 first. The final 1080p60 probe reported 1.48 ms decode + 6.74 ms pooled RGBA conversion with native audio (1.44 + 6.65 ms without); RGB32 remains only as a rotation fallback. |
| A 1080p live HLS stream failed on its first picture | The HLS stream reported RGBA and forwarded the read to the window's NV12 decoder. It now forwards `output_format`, `read_nv12`, `nv12_color` and `carries_audio` from that decoder, so the picture arrives in the layout the decoder produces. |
| Apple converted every picture to BGRA inside the framework | The track output asks for `420v`, the decoder's own planar output, and keeps BGRA only as a fallback. |
| Android converted YUV to RGBA in a scalar loop on the decode thread, and refused the interleaved chroma plane some codecs hand out | The planes are copied into NV12 — three separate planes or one interleaved plane both work — and converted once, across every core, when the screen asks. |
| Linux asked the pipeline for RGBA, paying a conversion per picture | The sink's caps ask for NV12, and the rows are copied out of the buffer. |
| `cargo run -- <url>` had no binary to run | `src/main.rs`, behind the `player` feature, opens a window on the source and plays it with sound. |
| A converted picture allocated eight megabytes per frame | The decode thread converts into a pooled buffer, and both layouts return to the slot on `recycle`. |
| The conversion ran on the thread that draws | It runs on the decode thread, or not at all for a screen that samples the planes. |
| wasm build warnings from the new telemetry | `SessionId` uses `AtomicUsize`; the re-anchor hook is compiled only off wasm. |
| Windows failed on an odd-width clip with `a stride (896) narrower than a row (3416)` | The decoder keeps its NV12 layout (one byte per luma pixel, not four), reads the authoritative pitch from `Lock2D` — including a negative bottom-up stride — and checks the coded surface, display crop and buffer length before copying any row. The declared matrix and nominal range travel with the frame instead of falling back to a size guess; an odd-sized stream that cannot be NV12 switches to RGB32. |

### Identified in review — not yet fixed

The review reports found the issues below. They are kept **separate from the fixes above** because no code change or device measurement backs them yet. Severity is the reviewer's classification, not a measured user impact; every row is either a source-confirmed fact or an explicitly labelled hypothesis. The full analysis, including the ranked speed techniques and the phased plan, is in `docs/rust-code-architecture-review.md`; the native drawing gaps are detailed in the two integration reports.

| Ref | Finding | Where | Status |
| --- | --- | --- | --- |
| S1-1 | The default `read_nv12` returns `Ok(None)`, the same value as end-of-stream, so a backend that declares NV12 without overriding it can stop at the first frame with a silent black screen | `src/decoder.rs:68-71`, `src/player/threaded.rs:222-234` | Open — a distinct status or a default error is needed |
| S1-2 | No `cropRect`/clean-aperture/display-size query before a frame is built; coded size may carry padding (for example 1088 against 1080), so encoder padding can show as edges | `src/platform/android.rs:617-628`, `src/platform/apple.rs:280-281` | Open — carry crop explicitly, see the verdict below |
| S2-1 | Apple `open_at` drops `Playback` without `cancelReading`, unlike `close` | `src/platform/apple.rs:154-162` against `369-373` | Source fixed — `cancelReading()` is now called before replacing the reader on seek; iOS device proof still pending |
| S2-2 | `wait_until` keeps a full-frame `scratch`/`planes` pair alive while sleeping, delaying command response | `src/player/threaded.rs:392-434` | Open — internal change |
| S2-5 | A fresh `ImageBitmap` is created for every composited frame | `src/widget.rs:175` | Open — allocation is confirmed, its effect on stutter is a hypothesis until measured |
| S3-1 | Android assumes interleaved chroma in NV12 order (Cb then Cr); a codec returning NV21/Vu would swap the colour channels | `src/platform/android.rs:681-733` | Open — device-dependent hypothesis |
| S2-4 | HLS rendition ceiling is `u32::MAX` while fragment fetch is a fixed four workers | `src/hls/mod.rs:106-112`, `91-94` | Open — prefer an adaptive ceiling; four fetchers are an abuse guard, not a resolution cap |
| H-1 | FFmpeg and mpv both warn that GPU decode only pays when frames stay on the GPU and the output matches the decoder; every mpv hwdec has a copy-back twin. Our native-texture intake is untested, so a GPU path cannot yet be claimed as faster or lower-power | `docs/hardware-decode-reference.md`, `set_native_nv12` in `src/player.rs` | Open — keep NV12/RGBA paths as default; add opt-in texture intake only with device proof |

**Architecture verdict on normalisation.** The mobile audit recommends one normalisation layer before `VideoFrame::from_nv12`; the Rust architecture review rejects that exact design when it is mandatory and discards platform metadata. A layer that repacks everything into tight NV12 and drops stride, crop, colour and chroma order hides the very bugs above and adds a copy on platforms that already produce the right shape. The agreed design is the architecture review's **Option A**: add an explicit constructor that carries those fields, leaving `from_nv12` and every existing backend untouched. That is additive, and it keeps platform mistakes visible in the type instead of silent on a device. The mobile audit's broader recommendation is superseded by this verdict, and the corrections above (Android stride reads, the 1080p-and-above parallel threshold) are recorded in both reviews.

Hard safety limits remain deliberately fixed: playlist 16 MiB, media 256 MiB, reserve 8 MiB, 200 000 segments, 86 400 s per segment, Android sample 128 MiB, MF gap 10 s / hold 2 s, Android timeout 8 s / empty deadline 30 s, media deadline 20–120 s, and four parallel fetches. These are abuse guards, not quality caps — there is no ceiling on resolution or frame size.

---

## Linux

Linux ships no media framework of its own. The `linux-gstreamer` feature builds the GStreamer backend, which needs the development files and the plugins:

```sh
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-libav
```

Without the feature the crate builds, `is_supported()` returns false, and `VideoPlayer::open` returns `VideoError::Unsupported`. An application supplies its own `VideoDecoder` through `VideoPlayer::with_decoder` there.

---

## Using it from an app

Inside a Cranpose checkout the crate builds as a workspace member. Its Cranpose dependencies come from `[workspace.dependencies]`, so the checkout decides which Cranpose the widget compiles against, and updating Cranpose updates the widget with it.

An app that keeps the crate in a directory of its own names the Cranpose it builds against in the crate's manifest. Replace the two inherited lines in `[dependencies]` with versions:

```toml
cranpose-core = { version = "0.24.0", optional = true }
cranpose-ui = { version = "0.24.0", optional = true }
```

Then depend on the crate from the app:

```toml
cranpose-video = { path = "vendor/cranpose-video", features = ["widget"] }
```

The `widget` feature is on by default, so `features` only states it for a dependency that turns default features off.

---

## Tests and verification

The integration tests in `tests/` decode `tests/fixtures/clip.mp4`, a one second 64 by 48 clip at 15 frames per second made with:

```sh
ffmpeg -f lavfi -i testsrc=size=64x48:rate=15:duration=1 -c:v libx264 \
    -pix_fmt yuv420p -movflags +faststart clip.mp4
```

The tests that decode a picture skip themselves on a target whose build carries no decoder, which is Linux without the feature. The suite contains unit tests plus `tests/observability.rs`, `tests/playback.rs`, `tests/sources.rs` and `tests/pacing.rs`.

### Pacing gate and stutter probe

Two tools answer "is the stutter in processing or in presentation?" They measure frame *publication* cadence — the signal both video pacing and the audio clock follow — without needing a display, a GPU, a codec, a network or an audio device.

The automated gate, `tests/pacing.rs`, drives a real `VideoPlayer` with a synthetic `VideoDecoder` at two exact rates: 5 fps (200 ms) for a wide-period baseline, and **60 fps (16.666666 ms)** for the rate the library has to sustain. It samples 24 frames at 5 fps (21 watched after warm-up) and 48 at 60 fps (44 watched after 3 warm-up frames) and checks the interval histogram against the real `FrameRate`-derived period. A controlled 800 ms stall at frame 10 (5 fps) and a 200 ms stall at frame 20 (60 fps, twelve periods) prove the detector actually fires — a test that only ever reports "smooth" proves nothing. The five cases are serialized by `PACING_GATE` because parallel harness threads fabricated gaps at a 16.7 ms period. It also checks PTS continuity, that every published picture is announced to the audio clock via `sync_clock`, and that one window-seam-like stall produces exactly one bounded clock re-anchor with recovery.

```sh
cargo test --test pacing                        # 5 tests (5 fps + 60 fps), deterministic, any target
cargo test --test pacing -- --nocapture         # prints the interval histogram
cargo test --test pacing --no-default-features
cargo test --test pacing --features native-audio
```

Gate thresholds are unchanged and strict: 5 fps requires `p50 <= 1.25x`, `p95 <= 1.5x`, `max <= 2.25x` and at most one long gap (`> 1.5x`); 60 fps requires `p50 <= 1.35x`, **`p95 <= 1.75x`**, `max <= 2.5x` and at most two long gaps. Both require zero re-anchors and zero errors. **[REPORT-ONLY]** In 12 targeted 60 fps runs reported in an earlier session (log `pacing-hrtimer.log`, not present in this checkout) every run passed: `p50=1.00` throughout, `p95=1.02–1.49`, `max=1.02–1.91`, no re-anchors and no errors. Long gaps appeared only in run 1 (`[9,14]`) and run 12 (`[21,26]`), inside the two-gap allowance. The injected-stall cases require `max >= 3x`, a long gap and a re-anchor with clean intervals afterwards; they reported `max≈4.0` at 5 fps and `max≈12.03` at 60 fps, proving the detector fires. The seam case reported `max≈4.5` with one bounded re-anchor.

The diagnostic probe, `examples/stutter_probe.rs`, runs the same measurement against a real source and the actual platform decoder (Media Foundation / AVFoundation / MediaCodec / GStreamer), including HLS joins and the paired native-audio path. Pass the exact path or URL the application opens.

```sh
cargo run --example stutter_probe -- tests/fixtures/clip.mp4 10
cargo run --features native-audio --example stutter_probe -- https://host/index.m3u8 20
```

It prints the interval histogram, p50/p95/p99/max in multiples of the frame period, the counters (`frames_published`, `reanchors`, `errors`), and the retained events that explain a hitch, then a verdict: `SMOOTH` when `max <= 2x`, long gaps are at most 1% (minimum 1), re-anchors `<= 1` and errors are zero; otherwise `STUTTER`. `STUTTER` means the player itself is delivering pictures late — a processing-side signal, independent of what the screen does afterwards. **[REPORT-ONLY]** Verified against `tests/fixtures/clip.mp4` with and without `native-audio`: `VERDICT: SMOOTH`, `max≈1.13x`, `reanchors=0`, `errors=0`.

Always probe an optimized build: `cargo run --release --example stutter_probe -- <source> [seconds]`. **[REPORT-ONLY]** In an earlier after-tables run (`probe-final-after-tables.log`, absent from this checkout), 720p25 was `SMOOTH` at 25.15 fps (`p95=1.04`, `max=1.92`, 1 long gap, 0 re-anchors), 1080p25 was `SMOOTH` at 25.14 fps (`p95=1.08`, `max=1.29`, 0 long gaps, 0 re-anchors), and odd-width 854p was `SMOOTH` at 23.99 fps. The earlier pre-fix 1080p25 `STUTTER` with a 25-re-anchor storm is therefore not the final state: the re-anchor storm was a consequence of late frames, not their cause.

**[REPORT-ONLY]** At 60 fps the same final run was mixed. 720p60 with native audio was `SMOOTH` at 60.11 fps (`p95=1.08`, `max=1.19`, 0 long gaps, 0 re-anchors), but 720p60 without native audio was `STUTTER` at 59.33 fps (`p95=1.10`, `max=3.57/59.58 ms`, 2 long gaps, 1 re-anchor). 1080p60 without native audio was `STUTTER` at 59.67 fps (`p95=1.27`, `max=2.26/37.72 ms`, 6 long gaps, 1 re-anchor), and 1080p60 with native audio was `STUTTER` at 58.83 fps (`p95=1.12`, `max=5.89/98.24 ms`, 2 long gaps, 2 re-anchors). With `CRANPOSE_VIDEO_WORKERS=4` and no concurrent benchmarks, five isolated repeats each (`probe-repeat-final.log`, absent from this checkout) were all `SMOOTH`: 720p60 native worst `p95=1.09`, `max=1.26`, 0 gaps; 1080p60 native worst `p95=1.12`, `max=1.46`, 0 gaps; 1080p60 without audio worst `p95=1.08`, `max=1.34`, 0 gaps. Those repeats show the fix can sustain 60 fps, and the outliers show the margin is load-sensitive — not proof of universal smoothness.

**[REPORT-ONLY]** The AJA HLS source remains unresolved: native audio reported 23.67 fps, `p95=1.28`, `max=12.64/505.54 ms`, 3 re-anchors and 7 long gaps; without native audio it reported 24.27 fps, `p95=1.26`, `max=9.96/398.44 ms`, 2 re-anchors and 4 long gaps. HLS rollover is still an open, separate problem.

Limitations: these tools see only frame publication. They cannot see the compositor, GPU present or VSync (so a published frame can still be dropped by the display), cannot hear an actual audio-device underrun, and do not exercise the UI bitmap/upload path in `src/widget.rs`. They detect gross stutter (gaps beyond ~1.5 frame periods), not subtle jitter. The event ring keeps at most 256 records (`EVENT_CAP`), so counters cover a whole run but individual events only the recent window.

The full verification matrix recorded for the earlier update (**[REPORT-ONLY]** unless a current command is repeated in the test-round section below):

| Check | Result |
| --- | --- |
| `cargo check --features native-audio --all-targets` | clean |
| `cargo test --features native-audio --all-targets` | 100 library + 7 observability + 5 pacing + 10 playback + 5 sources, 0 failures (empty example groups skipped) |
| `cargo test --features native-audio --doc` | 11 passed |
| `cargo test --test pacing --no-default-features` | 5 passed (final rerun after the documentation edits; the cited `pacing-nodefault-final.log` is absent from this checkout) |
| `cargo test --test pacing --features native-audio` | 5 passed |
| `cargo check --target wasm32-unknown-unknown` | clean |
| Root `cargo build` from `D:\counter` | `Finished dev`, no errors |
| Android `aarch64-linux-android` type-check of the JNI scratch code | passed with ureq TLS removed |
| Static review documents | 8 completed: `technique-catalog.md`, `mobile-platform-audit.md`, `cross-platform-ui-video-review.md`, `rust-code-architecture-review.md`, `phone-integration-review.md`, `desktop-integration-review.md`, `upstream-decoder-comparison.md`, `hardware-decode-reference.md`. Review-only; no device run claimed. |

Still pending: a real-device Android run (`docs/android-test-checklist.md`), runtime HLS checks on Linux (WSL was fully removed, so no Linux device run exists), and an iOS device run for the `cancelReading` fix. Also open locally: the AJA HLS rollover stutter, the load-sensitive 1080p60 outliers, GPU/compositor presentation (the probe sees publication only), audible device underruns, and power measurement — this host has no external power meter. **[REPORT-ONLY]** CPU/memory evidence cited in an earlier report (`perf-final.log`, absent from this checkout): `pixel-bench` used 5094 ms CPU (127.4% of one core) with 113.09/113.12 MB peak working/private; 720p60 playback used 2859 ms (47.7%) with 44.41/33.57 MB; four 720p25 players used 6093 ms (53.0%) with 159.5/138.41 MB; four 720p60 players used 12266 ms (102.2%) with 147.31/142.09 MB. The review documents add device-dependent questions that only a real device can close: Android chroma order and BT.2020/PQ/HLG colour, Apple clean aperture and colour metadata, and the thread/memory budget when many reels play at once. The full local Android target build is blocked here by a missing NDK (`ANDROID_NDK_HOME`). Linux GStreamer was absent in the earlier report and WSL was subsequently fully removed; there is no completed Linux run or Linux setup log in this checkout. Measured slots for Linux/Reels remain empty and are marked `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]`; they may be filled only from a captured run with its exact command, feature set, environment and output. A pre-existing failure in `D:\counter\examples\probe.rs` (`use mp4::Mp4Reader`) affects `cargo check --all-targets` at the workspace root only; the plain root build is green.

Suggested stream checks: `https://samplelib.com/mp4/sample-30s-720p.mp4`, `https://live-hls-web-aja.getaj.net/AJA/index.m3u8`, `https://live.i-news.tv/hls/stream.m3u8`, and a local `assets/videos/bunny.mp4`.

**Live channel catalogue (demo app).** The root demo's `src/app_model.rs` also carries `LIVE_CHANNELS`: **42 unique playable HLS URLs** transcribed from the Free-TV/IPTV community playlists `playlist_zz_news_ar.m3u8` (Arabic news) and `playlist_united_arab_emirates.m3u8` (UAE), fetched 2026-11-19 from `raw.githubusercontent.com`. The source lists contain 45 entries; one YouTube channel page is not a media playlist, and two URLs (Sky News Arabia, Ajman TV) are duplicates, leaving 42. The list is kept out of `CLIPS`/`decode_cases()` so offline tests never depend on a server. Availability and codec support of third-party streams are **not** verified here; the provenance check is an exact string comparison against the fetched playlists, not a playback test.

---

## Finalization status of this documentation round

This round edited **both source and documentation**: `src/player.rs`, `src/player/threaded.rs`, `src/platform/windows/mf_audio.rs`, `src/audio.rs`, `src/hls/mod.rs`, `src/frame.rs`, `src/widget.rs`, plus `README.md` and `docs/*.md`. Nothing was staged, committed, pushed, merged or PR'd. No new `[MEASURED]` performance number was introduced: WSL was fully removed, no Linux/Reels device run has completed, and the reserved slots remain empty and explicitly marked `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]` with instructions not to invent values. Lifecycle, audio and UI facts below are `[SOURCE-VERIFIED]`; runtime and device effects are `[HYPOTHESIS]`. Stale version/path citations were corrected against the current `Cargo.lock`/source, and historical log numbers were kept verbatim but marked `[REPORT-ONLY]` instead of being presented as current.

- **[SOURCE-VERIFIED] Close is non-blocking and `Closed` is last.** `VideoPlayer::close()` raises the shutdown flag, sends `Command::Shutdown` and drops the `JoinHandle` to detach the worker (`src/player/threaded.rs:89-100`) instead of joining it; the worker still owns and closes the `Box<dyn VideoDecoder>` when it returns. A read landing after shutdown returns before publication, clock writes or events (`src/player/threaded.rs:239-255`), and `Shared::emit` seals every event after shutdown except `Closed` (`src/player.rs:219-230`), so `Closed` remains the last event. The RED test `close_does_not_wait_on_a_decoder_stuck_in_read` was reported green earlier `[REPORT-ONLY]`; no device timing was measured.
- **[SOURCE-VERIFIED] Audio EOF and prime deadline.** The 500 ms prime deadline (`PRIME_TIMEOUT` / `retire_expired_wait`, `src/platform/windows/mf_audio.rs:77`, `:264-269`) was already present. Only `MF_SOURCE_READERF_ENDOFSTREAM` latches `ended`; an HRESULT failure or `MF_SOURCE_READERF_ERROR` returns retryably to the next pump. `Audio::play()` clears `ended`, and `sync_to()` via `start_ready()` starts on a full `START_LEAD` or a non-empty end-of-stream tail, never on an empty buffer. Audible kickoff and recovery on a device are `[HYPOTHESIS]` and unmeasured.
- **[SOURCE-VERIFIED] Reels demo additions (not a device run).** The root demo app carries `LIVE_CHANNELS` (42 unique HLS URLs; provenance above) and `decode_cases()`. Its Reels UI clears the previous source's picture and resolution at the start of the source-keyed pump (`D:\counter\src\app_ui\reels.rs:99-105`), then polls `decoded_size()` at `:142`; both the frame and resolution pumps break on `is_closed()` (`:113-117`, `:137-141`). The in-app decode console is root `src/app_ui/tests_console.rs`. These are code-level additions; no multi-reel device playback, thermal or visual result was measured here.
- **[SOURCE-VERIFIED] `decoded_size()`.** Exposed on `VideoPlayer` at `src/player.rs:774` and re-exported on the widget at `src/widget.rs:147-148`, so a UI can read the decoder's real picture size and follow dynamic resolution changes instead of guessing from the container. No runtime size value is claimed.
- **[SOURCE-VERIFIED] In-app decode tests (run earlier in this work).** `rustc --edition 2021 --test src/app_model.rs` compiled and ran the root demo model tests: **8 passed, 0 failed** (including `live_channels_are_unique_opt_in_hls_sources` and `decode_cases_cover_local_files_only`). This covers the demo model's catalogue and decode-case logic, not a platform decoder on a device.
- **[SOURCE-VERIFIED] vs [HYPOTHESIS] — platform audio device-switch status.** Windows Media Foundation rebinds its audio sink on device change (source path plus unit tests); the generic Linux/macOS cpal path now polls the system default output from the decode thread and reopens the stream only when its PCM layout matches exactly (`src/audio.rs:49-97`, `src/audio.rs:242-301`, with focused tests at `src/audio.rs:727-747`); Android's AAudio disconnect flag is internal and not exposed by this crate; the web backend is browser-routed. The 500 ms prime deadline and the true-EOF-only latch above are also source-verified. Only the source facts are verified — audible recovery or kickoff on any real device is unmeasured and remains `[HYPOTHESIS]`.
- **[SOURCE-VERIFIED] Playback event vs state.** The `Playing` *event* is gated on the first published frame, while `state()` reports `Playing` immediately; see the observability section and `tests/first_play.rs:82-99`. This is source behaviour confirmed by a unit test, not a runtime observation.
- **[HYPOTHESIS] Still unresolved / pending device proof.** Real-device Android proof (`docs/android-test-checklist.md`), an iOS device run for the `cancelReading` fix, Linux runtime HLS checks (WSL was fully removed; no Linux run exists), GPU/compositor presentation, audible device underruns or audio kickoff, and power/thermal measurement. The legacy GLES-host startup diagnosis in `docs/final-report-ar.md` §6 was **not** re-tested after the current `Cargo.lock` upgrade; its version-specific claims are now marked historical `[REPORT-ONLY]`.

---

## Links

- [API documentation](https://docs.rs/cranpose-video/latest/cranpose_video/)
- [Source](https://github.com/samoylenkodmitry/Cranpose/tree/main/crates/cranpose-video)
- [Android device test checklist](docs/android-test-checklist.md)
- [Technique catalog — Makepad / FFmpeg / GStreamer / mpv / VLC](docs/technique-catalog.md)
- [Mobile platform audit](docs/mobile-platform-audit.md)
- [Cross-platform native UI/video review](docs/cross-platform-ui-video-review.md)
- [Rust code and architecture review](docs/rust-code-architecture-review.md)
- [Phone native integration review](docs/phone-integration-review.md)
- [Desktop native integration review](docs/desktop-integration-review.md)
- [Upstream decoder comparison](docs/upstream-decoder-comparison.md)
