# cranpose-video — Full Library Review (code-level)

Date: 2026-11-26. Every finding below was verified by reading the actual source
in `src/` (60 files, ~31k lines) and running searches across the tree. No
finding is inferred from documentation alone.

## 1. Architecture summary

The crate has two layers that share one decoder per platform:

- **Engine layer** — `VideoPlayer` (`src/player.rs`, 2031 lines) opens a
  `VideoSource`, drives a `VideoDecoder` (`src/decoder.rs` trait), and exposes
  transport, tracks, events, telemetry.
- **UI layer** — `src/widget.rs` (623 lines): `rememberVideoPlayer`,
  `VideoPlayerHandle`, `Video`, `VideoFrame`, `VideoWithControls`. The handle
  wraps the same player; no second player exists.

Platform decoders (`src/platform/`): Media Foundation (Windows, incl.
`mf_audio.rs` 2161 lines for PCM), AVFoundation + AVPlayer (Apple),
MediaExtractor/MediaCodec via JNI (Android), GStreamer behind
`linux-gstreamer` (Linux), HTML5 `<video>` (wasm). `platform/unsupported.rs`
returns `VideoError::Unsupported` elsewhere.

Streaming: `src/hls/` (2530 + 1086 + 314 lines) fetches playlists, joins
fragment windows into temp files, handles live rollover, separate audio
renditions, `EXT-X-MAP` init sections, and TFDT rebasing. `src/dash.rs` is a
structural MPD parser only.

Support systems: `feed.rs` (feed orchestration with generation tokens),
`pool.rs` (player pool), `preload.rs` + `preload_coordinator.rs` (6-level
incremental preparation), `buffer.rs`, `load_control.rs`, `abr.rs`,
`stream_policy.rs`, `cache.rs` (RAM) + `disk_cache.rs`, `resource.rs`,
`session.rs`, `events.rs`, `telemetry.rs`, `diagnostics.rs`, `logfile.rs`,
`tracks.rs`, `subtitle.rs`, `audio.rs` (1138 lines, cpal output + route
switching), `pixels.rs` (NV12→RGBA), `frame.rs` (RGBA/NV12 frames with color
info), `capabilities.rs`, `source.rs`, `upstream.rs` (Cranpose 0.28 hardware
decoder bridge).

Code quality is high: **zero TODO/FIXME/unimplemented markers** in the entire
tree, and errors are typed (`VideoError` with `is_transient`, redaction).

## 2. Verified gaps (file by file)

### Streaming

| Area | State | Evidence |
|---|---|---|
| HLS VOD + live | Implemented: playlist parse, window join, live rollover, separate audio, `EXT-X-MAP`, TFDT rebase | `src/hls/mod.rs`, `src/hls/playlist.rs`, `src/hls/tfdt.rs` |
| HLS low-latency (LL-HLS) | **Missing**: no `EXT-X-PART`, no `EXT-X-PRELOAD-HINT`, no blocking/delta playlist reload | grep over `src/hls/` |
| DASH | **Parser only.** `parse_mpd` exists and is exported, but nothing consumes `DashManifest` — no segment scheduler/downloader; `platform/mod.rs::open` routes only `source.is_hls()` | grep: `parse_mpd` referenced only in `dash.rs` and `lib.rs` |
| ABR wiring | `AbrController::sample` exists with tests, but is not wired into the HLS segment scheduler | `src/abr.rs` vs `src/hls/mod.rs` |
| RTSP | Missing entirely | no match in `src/` |
| DRM (Widevine/FairPlay/PlayReady/CENC) | Missing entirely | grep: 0 matches |

### Subtitles

`src/subtitle.rs` parses SRT + WebVTT into `SubtitleCue`s and is exported
(`parse_subtitles`, `active_at`). But:

- The player exposes only backend track selection (`set_subtitle_track` →
  `VideoDecoder::set_subtitle_track`, default no-op in `decoder.rs:186`).
- **The parsed cues are never wired into the player or the widget**: grep shows
  no `subtitle`/`cue` reference in `src/widget.rs`. The app must poll
  `active_at` itself; there is no timed-text event, no sidecar-file loader, no
  HLS/WebVTT track extraction.

### Preload / instant start

- `preload_coordinator.rs` decides *how far* to prepare (6 levels) but
  **never downloads**: grep shows no `ureq`/range fetch in `preload*.rs`. The
  HTTP byte-range downloader the roadmap calls for does not exist yet.
- `cache.rs` (RAM) and `disk_cache.rs` exist, but `disk_cache.rs` has **no
  expiration, no integrity hash, no hit/miss metrics** (grep: 0 matches), and
  cache keys are not segment-level for HLS.
- `feed.rs` bookkeeping is complete, but the roadmap's own Phase 1 admits the
  controller is not wired to `PlayerPool` slot handoff on pager change.

### Presentation

- Default path is CPU RGBA (`pixels.rs` converts NV12→RGBA). Zero-copy exists
  only via the opt-in `upstream-video` feature bridging Cranpose 0.28's
  decoder; the crate itself owns no D3D11/Metal/AHardwareBuffer/DMABUF path
  (grep confirms).
- `upstream.rs::open_stream` hardcodes a 40 ms timestamp step and requires the
  caller to demux the container into Annex-B access units — no MP4/HLS →
  access-unit converter exists in the crate.

### Platform coverage

- Linux without `linux-gstreamer` builds but returns `VideoError::Unsupported`
  (`platform/unsupported.rs`).
- `capabilities.rs` reports codecs per platform statically; there is no runtime
  probe of e.g. AV1 support on a specific device.

### Measurement

- No `benches/` directory exists. `examples/perf_probe.rs`,
  `stutter_probe.rs`, `stress_probe.rs`, `pixel_bench.rs` are manual probes,
  not criterion benches. No device-measured numbers are stored.

## 3. What is alrady strong

- Generation tokens + cancellation discipline across `feed.rs`,
  `preload.rs`, `segment.rs` — the hard part of feed playback is already
  designed correctly.
- Live HLS with separate audio renditions and TFDT rebasing is rare at this
  level in Rust.
- Audio route switching (headset/Bluetooth) handled per platform
  (`audio.rs::observe_output_route`, iOS observer in `platform/`).
- Typed, redactable errors and a diagnostics/logfile layer.
- 251+ lib tests plus 16 integration test files.

## 4. Technologies to add (priority order)

1. **HTTP byte-range fetch layer** (`src/range.rs`): partial downloads with
   `Range`/206, connection reuse, per-request abort tokens carrying the
   existing generation IDs. Unlocks preload, DASH, and segment cache at once.
2. **DASH segment engine**: reuse the HLS window/scheduler design against
   `DashManifest` (SegmentTemplate/SegmentBase), then wire `AbrController`
   into both engines.
3. **Subtitle pipeline**: sidecar loader (file/URL), timed-text events on
   `EventBus`, widget rendering of `active_at` cues; HLS `EXT-X-MEDIA`
   subtitle renditions.
4. **Disk cache hardening**: expiration, integrity hash, hit/miss metrics,
   segment-level keys including the `EXT-X-MAP` init URI.
5. **LL-HLS parsing** (`EXT-X-PART`, `EXT-X-PRELOAD-HINT`, delta reload).
6. **Container → access-unit demuxer** so `HardwareVideo::open_stream` can be
   fed from an MP4/HLS source without caller-side demuxing.
7. **Native surface paths** (feature-gated): D3D11/DXGI (Windows),
   AVPlayerLayer/CVPixelBuffer (Apple), MediaCodec Surface (Android),
   DMABUF (Linux), direct `<video>` presentation (web) — each documented with
   measured copies or marked compile-only.
8. **DRM**: at minimum ClearKey/CENC on the platforms that expose it; Widevine
   and FairPlay need platform session plumbing.
9. **Runtime capability probe** extending `capabilities.rs` beyond static
   tables.
10. **Criterion benches** under `benches/` with recorded device evidence.
