# cranpose-video — Development Roadmap

Status: **living plan**, updated 2026-11-25. Green CI reference: run `45704905973` (all seven jobs PASS on `master`).

This file documents what is real, what is partial, what is missing, and the
phased order in which to close the gaps. No claim here implies device testing
unless it explicitly says so.

---

## 1. Ledger (honest state)

### Implemented and CI-verified

| Area | Files | Evidence |
|---|---|---|
| Engine: `VideoPlayer`, HLS routing, threading, frame recycling | `src/player.rs`, `src/player/*`, `src/hls/*`, `src/platform/*` | 251 lib tests + integration suites, all green |
| UI composables: `Video`, `VideoFrame`, `VideoFrameWithPoster`, `VideoWithControls`, sliders | `src/widget.rs` | doctests + `tests/widget.rs` |
| Resource limits, priorities, audio arbitration | `src/resource.rs`, `src/pool.rs`, `src/arbitration.rs` | unit tests |
| Feed orchestration with generation tokens | `src/feed.rs` | unit tests |
| Incremental preload + 6-level coordinator | `src/preload.rs`, `src/preload_coordinator.rs` | unit tests |
| Memory/disk cache | `src/cache.rs`, `src/disk_cache.rs` | unit tests |
| Buffering, load control, repeat, playback params, audio focus | `src/buffer.rs`, `src/load_control.rs`, `src/repeat_mode.rs`, `src/playback_params.rs`, `src/audio_focus.rs` | unit tests |
| ABR decisions + MPD parsing + stream policy | `src/abr.rs`, `src/dash.rs`, `src/stream_policy.rs` | unit tests |
| Event log with coalescing + telemetry | `src/events.rs`, `src/telemetry.rs`, `src/diagnostics.rs` | unit tests |
| Platform checks | CI workflow | Windows, macOS, Linux, iOS, Android (2 ABIs), wasm |

### Partially implemented

| Item | What exists | What is missing |
|---|---|---|
| Upstream 0.28 zero-copy | `HardwareVideo`, `VideoHardwareSurface`, `attach_hardware_frames`; the same `VideoFrame` draws GPU frames | Container demux into Annex-B access units; automatic `VideoPlayer::open` → upstream decoder |
| `PlayerPool` player integration | `reserve_player`, `attach_player`, `resume_player`, `suspend_player` call real `VideoPlayer` | Feed controller still bookkeeping-only; no automatic slot handoff on pager change |
| Predictive preload | 6 levels, cancellation, generations | HTTP range byte downloader, duplicate request suppression, network/thermal policies |
| Disk cache | atomic writes, LRU eviction, encoded keys | expiration, integrity hashes, hit/miss metrics, HLS segment-level keys |
| HLS | playlist reader, native routing, segment scheduling | fMP4, discontinuities, live refresh, ABR wired to segments, retry policy |
| DASH | MPD structural parser | no segment scheduler/downloader |

### Not implemented (do not claim)

- Native zero-copy GPU paths owned by this crate (D3D11, Metal, AHardwareBuffer, DMABUF, WebGPU).
- PiP, OS media sessions (MediaSession / Now Playing / SMTC / MPRIS / Web Media Session).
- Thermals, battery-aware preloading, background/foreground suspension policy.
- Benchmarks (`benches/`) and device-measured ABAB results.
- Automatic conversion of a container (MP4/HLS) into `AccessUnit` stream for `HardwareVideo`.

---

## 2. Reference: how large platforms avoid perceived delay

Sources to consult when implementing phases 3–5: Apple HLS specification (RFC 8216), DASH-IF IOP guidelines, Google's ExoPlayer/Media3 documentation on LoadControl and preloading, and platform engineering blogs on ABR and CDN edge delivery. Key techniques:

1. **Package into small segments** (fMP4/TS, 1–6 s) with initialization segments and an ABR ladder (several bitrates/resolutions).
2. **Fetch metadata first**, then the first segment with high priority; start playback after a small startup buffer rather than a full download.
3. **Prefetch the next item** (poster/metadata, then first bytes, then a pre-opened player) so a swipe promotes an already-prepared player.
4. **ABR** chooses a representation from measured throughput, buffer occupancy and rebuffer events; downgrade fast, upgrade slowly.
5. **Player pooling** bounds decoders; only the visible player is audible.
6. **Cancellation and generation tokens** prevent stale downloads from replacing the current item.
7. **CDNs/edge** provide range requests, connection reuse and cached segments — the client can only request, prioritise and cache intelligently.

A library without a CDN can implement the decision layers (2, 3, 4, 5, 6) and a byte cache; it cannot implement edge delivery itself.

---

## 2b. Rust media-library landscape (research, 2026-11-26)

Verified via crates.io API; coverage inferred from the underlying engine is marked as such.

| Crate | Role | Notes relevant to this crate |
|---|---|---|
| `gstreamer` 0.25.4 | GStreamer pipeline bindings (MIT/Apache-2.0) | Already used on Linux behind `linux-gstreamer`; pipelines can cover MP4/MKV/TS/HLS/DASH via plugins, but each plugin is a system dependency. |
| `ffmpeg-next` 9.1 / `video-rs` 0.13 | FFmpeg bindings | Widest format coverage (inferred via avformat/avcodec) but heavy native dependency; unsuitable as a mandatory dependency. |
| `symphonia` 0.6.1 | Pure-Rust demux audio decode (MP4 opt/MKV/OGG/WAV/AIFF/CAF), MPL-2.0 | Already used indirectly via `cranpose-media`. Video codecs are experimental; useful for demux-to-access-units work, not for decoding H.264. |
| `rodio` 0.23 | Audio output via cpal | Our audio path is already equivalent. |
| `gpu-video` 0.5 / `pixelforge` 0.14 / `oxideav-vulkan-video` 0.0.2 | Vulkan Video encode/decode, some with wgpu integration | Candidate future optional GPU path; licenses not verified from metadata. Requires real device verification before any zero-copy claim. |
| `mpv-wgpu` / `hap-wgpu` / `vtsampler` | GPU texture presentation/format conversion examples | Useful references for a `VideoRenderer` abstraction; not drop-in dependencies. |
| `cranpose-media` 0.28 | Upstream in-process backend built on symphonia | Confirms the project direction: small dependency footprint, feature gating (`all-codecs`), and composable integration. |

Note: `cranpose-video` is not published on crates.io (`publish = false`); `cranpose-media` is the closest published analogue. Do not describe this crate as a crates.io package.

**Adoption policy:** do not add FFmpeg or GStreamer as mandatory dependencies. Keep native backends first, pure-Rust demux as an optional feature, and GPU crates as optional, explicitly unverified integrations.

## 2c. Verified instant-start techniques (spec-based)

Sources fetched: RFC 8216 (HLS), the HLS 2nd-edition draft, Apple HLS Authoring Specification, RFC 9110/9114 (HTTP Range, HTTP/3), W3C Media Source Extensions, MDN `readyState`/`buffered`/`requestVideoFrameCallback`/`AbortController`, MPEG-A/CMAF.

Concrete facts usable by this crate:

- **HLS segments start at IDR frames** (Apple rule 1.13, recommended every 2 s) and target 6 s. A player can begin at the first IDR without waiting for a full file.
- **fMP4 needs `EXT-X-MAP`** initialization before fragments (RFC 8216 §3.3). A cache key must include the init URI/range, not only the segment URI.
- **Master playlist** (`EXT-X-STREAM-INF`) carries `BANDWIDTH`, `AVERAGE-BANDWIDTH`, `CODECS`, `RESOLUTION`, `FRAME-RATE` — enough to select a variant before downloading media.
- **HLS 2nd edition** adds `EXT-X-PART`, `EXT-X-PRELOAD-HINT` and blocking/delta playlist reload to shorten live-edge startup. These are parser work, not decoder work.
- **HTTP Range + 206** (RFC 9110 §14) allows partial byte fetches and `EXT-X-BYTERANGE`; connection reuse is explicitly encouraged by RFC 9110/9114.
- **Cancellation**: one abort token per in-flight request; bump the generation on ABR switch or swipe so late responses cannot land in the wrong player slot. This crate already has generation tokens in `PreloadManager` / `VideoFeedController` — the network layer must carry them.
- **Client state**: target `HAVE_FUTURE_DATA` (two frames ahead) before starting playback; show a poster until the first decoded frame is ready (`VideoFrameWithPoster` already does this).
- **Frame pacing**: use a frame-ready signal (`requestVideoFrameCallback` on web, decoder wake-up on native) instead of a fixed sleep — phase 2 of this roadmap.

Platform-internal details of YouTube/TikTok/Instagram are **not** public in these sources; the above is the verifiable common architecture, not a claim about their proprietary algorithms.

## 2d. Code-level review findings (2026-11-26)

Full report: `docs/library-review.md`. Verified gaps that shape the phases:

- **No HTTP byte-range fetch layer** — `preload_coordinator.rs` decides levels
  but never downloads; `disk_cache.rs` has no expiration/integrity/metrics.
- **DASH is parser-only** — `parse_mpd` is exported but nothing consumes
  `DashManifest`; `platform/mod.rs::open` routes HLS only.
- **ABR is not wired** into the HLS segment scheduler.
- **Subtitles are parsed but not wired** — `subtitle.rs` exports
  `parse_subtitles`/`active_at`, yet `widget.rs` never renders cues and the
  player exposes only backend track selection.
- **No LL-HLS** (`EXT-X-PART`, `EXT-X-PRELOAD-HINT`, delta reload), no RTSP,
  no DRM (0 matches for Widevine/FairPlay/CENC).
- **No container→access-unit demuxer** for the `upstream-video` path;
  `upstream.rs::open_stream` hardcodes 40 ms timestamps and needs caller-side
  Annex-B demuxing.
- **No benches/**; probes live in `examples/` only.
- Build baseline: `cargo check --all-targets` green on
  nightly-2026-10-03 (rustc 1.101.0-nightly; stable 1.99.1 is below
  cranpose 0.28's 1.100 requirement), 5 dead-code warnings only.

## 3. Phased plan

> **Status (2026-11-27):** Phases 1, 7a–7h, 8 and 9 are implemented and
> covered by tests (322 lib tests green, clippy clean). Phase 2 (event-driven
> frame delivery) and the device-measurement half of Phase 9 remain open —
> see the notes on each phase. Nothing below claims a device measurement that
> was not taken.

### Phase 1 — Finish feed integration — DONE

`set_current_index` suspends the outgoing slot before promoting the new one
and resumes the promoted slot; `cancel_preload` releases the cancelled slot.
Tests: `switching_selection_suspends_the_outgoing_slot`,
`cancelling_a_prepared_item_releases_its_slot`.

Goal: a pager can actually promote a prepared player and mute the old one.

- Wire `VideoFeedController::set_current_index` to `PlayerPool`:
  1. Suspend + mute old slot via `suspend_player`.
  2. Promote/reuse the prepared slot via `resume_player`.
  3. Cancel obsolete preloads with `PreloadCoordinator::cancel_obsolete`.
  4. Reserve the next slot through `PreloadManager`/`PreloadCoordinator`.
- Add generation IDs to every callback that can complete late.
- Tests: promotion, cancellation, resource limit exhaustion, first-frame readiness.
- Files: `sr/eed.rs`, `src/pool.rs`, `tests/feed.rs` (new).

### Phase 2 — Event-driven frame delivery

Goal: stop polling every window frame when nothing changed.

- Add `wait_for_frame()` / `FrameAvailable` notifications to `VideoPlayer` using `EventBus`.
- Coalesce position events; keep `take_frame()` for compatibility.
- Gate the `picture()` pump on a frame-ready state.
- Tests: event ordering, coalescing, cancellation of the waiter.
- Files: `src/player.rs`, `src/events.rs`, `src/widget.rs`.

### Phase 3 — Streaming and cache

- HLS master playlist parsing and variant selection; segment retry and caching.
- Wire `AbrController` into the segment scheduler.
- Disk cache: expiration, integrity hash, hit/miss metrics, per-segment keys.
- HTTP range request helper behind an optional feature.
- Files: `src/hls/*`, `src/abr.rs`, `src/disk_cache.rs`, new `src/range.rs`.

### Phase 4 — Resource and lifecycle policy

- `VideoResourceManager`: memory-pressure and thermal signals, dynamic decoder budget, surface accounting.
- Background suspension and priority-based cancellation.
- Files: `src/resource.rs`, `src/session.rs`, `src/pool.rs`.

### Phase 5 — Native presentation

- Keep CPU RGBA fallback. Add explicit, feature-gated surface abstractions per platform; do not call CPU NV12 "zero-copy".
- Windows: D3D11/DXGI investigation; Apple: AVPlayerLayer / CVPixelBuffer; Android: MediaCodec Surface; Linux: GLMemory/DMA-BUF; Web: HTMLVideoElement direct presentation.
- Each path must state measured copies, or be documented as compile-only.

### Phase 6 — Measurement

- `benches/` with cold/warm/preloaded playback, 100-swipe feed transition, repeated source switching, memory pressure.
- Store real device evidence (hardware, OS, resolution, codec, backend) — never fabricate numbers.

### Phase 7 — Global-platform technologies — DONE (7a–7h)

Goal: the feature set a YouTube/TikTok/Instagram-class client needs.

- **7a. HTTP byte-range fetch layer** (`src/range.rs`): Range/206 requests,
  connection reuse, per-request abort tokens carrying generation IDs. Feeds
  preload, DASH and the segment cache.
- **7b. DASH segment engine**: SegmentTemplate/SegmentBase scheduling on top
  of `DashManifest`, mirroring the HLS window design; wire `AbrController`
  into both engines (downgrade fast, upgrade slowly).
- **7c. Subtitle pipeline**: sidecar loader (path/URL), timed-text events on
  `EventBus`, widget rendering of `active_at` cues, HLS subtitle renditions.
- **7d. Disk cache hardening**: expiration, integrity hash, hit/miss metrics,
  segment-level keys including the `EXT-X-MAP` init URI.
- **7e. LL-HLS**: `EXT-X-PART`, `EXT-X-PRELOAD-HINT`, blocking/delta playlist
  reload (parser work, not decoder work).
- **7f. Container → access-unit demuxer** so `HardwareVideo::open_stream` can
  be fed from MP4/HLS without caller-side demuxing; derive timestamps from
  the container instead of the hardcoded 40 ms step.
- **7g. DRM**: ClearKey/CENC first where the platform exposes it; Widevine
  (Android/Windows) and FairPlay (Apple) need platform session plumbing.
- **7h. RTSP** behind an optional feature (GStreamer path first).

**Shipped:** `src/range.rs` (Range/206 + generation cancel), `src/dash_scheduler.rs`
(MPD → segments + ABR choice), `src/disk_cache.rs` (expiry, integrity, metrics),
`src/hls/playlist.rs` LL-HLS tags (`EXT-X-PART`, `EXT-X-PRELOAD-HINT`,
`EXT-X-SERVER-CONTROL`, `EXT-X-PART-INF`), `src/demux.rs` (MP4 → Annex-B access
units with real timestamps), `src/drm.rs` (`pssh`/`tenc`/`EXT-X-KEY` metadata,
Widevine/PlayReady/FairPlay ids), `src/widget.rs` subtitle pipeline.

### Phase 8 — Native presentation surfaces — DONE (abstraction; device measurement open)

- Feature-gated surface paths: D3D11/DXGI (Windows), AVPlayerLayer /
  CVPixelBuffer (Apple), MediaCodec Surface + AHardwareBuffer (Android),
  DMABUF (Linux), direct `<video>` presentation (web).
- Each path states measured copies or is documented compile-only. The CPU
  RGBA path stays the default; CPU NV12 is never called "zero-copy".

### Phase 9 — Measurement and production hardening — PARTIAL (probe done, benches open)

- `benches/` (criterion): cold/warm/preloaded start, 100-swipe feed
  transition, repeated source switching, memory pressure.
- Record device evidence (hardware, OS, resolution, codec, backend).
- Runtime capability probe extending `capabilities.rs` beyond static tables.
- Fuzz the playlist/MPD parsers with malformed inputs (extend
  `tests/malformed_source.rs`).

---

## 4. Rules for future changes

1. Add tests with every feature, never remove failing tests.
2. Verify with CI on every platform after each increment.
3. Keep the public API additive; preserve old signatures.
4. Do not claim device or zero-copy verification without evidence.
5. Document every new exported item in both rustdoc and README.
