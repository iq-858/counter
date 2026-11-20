# Upstream `cranpose-services` decoder comparison

Read-only comparison against the current upstream Cranpose source tree (`main`, commit `3e53a1b8`). No upstream code was copied into this crate and no public API here was changed by this review.

## Exact upstream source

The decoder API is **not** in `cranpose-0.24.0` or `cranpose-media-0.24.0`. It lives in the workspace source at:

- `crates/cranpose-services/src/video_decoder.rs`
- `crates/cranpose-services/src/video_decoder/{config,codec,decoder,frame,frame_queue,stats,capabilities,admission,error}.rs`
- `crates/cranpose-services/src/video_decoder/{media_codec,image_reader,video_toolbox,decompression_session}.rs`
- `crates/cranpose/src/player_surface_intake.rs`

The examples that show how it is driven are `apps/player-demo/src/clip_feed.rs`, `apps/player-demo/src/feed_thread.rs`, `apps/xr-demo/src/clip_player.rs`, and `apps/xr-demo/src/decoder_limits.rs`.

## What upstream actually does

| Concern | Upstream behaviour | Evidence |
| --- | --- | --- |
| Output choice | `DecoderOutput::Surface(ExternalSurface)` sends pictures straight to a compositor with no copy; `DecoderOutput::Frames { depth }` returns bounded `VideoFrame`s that a renderer imports as GPU images. | `video_decoder/config.rs:26-40` |
| Feed/frame-loop split | `submit` blocks only the **feed thread** while it waits for decoder room. The frame loop reads `DecodedFrames` with short lock-only calls and never waits for decode. | `video_decoder/config.rs:8-24`, `video_decoder/decoder.rs:16-23,48-55` |
| Input wait | Default `INPUT_WAIT` is 30 ms. The source documents a measured client whose 3840x2160 decode averaged 5.9 ms and peaked at 15.9 ms; the old 4 ms wait caused refusals and lost reference chains. | `video_decoder/config.rs:8-24` |
| Bounded in-flight pictures | Demos use depth 8 for the player surface and depth 6 for the XR stream. The comments account for pictures held by the surface, three GPU-held frames, and decoder slack. | `apps/player-demo/src/clip_feed.rs`, `apps/xr-demo/src/clip_player.rs` |
| Backpressure | When the app-held queue reaches `depth`, the decoder drops the oldest frame and counts it. `latest_frame()` also drops older frames. | `video_decoder/frame_queue.rs:23-29,56-63`; `media_codec.rs:324-331`; `video_toolbox.rs:279-289` |
| Wakeups | `DecodedFrames::wait_for_frame()` uses a stored `Waker` instead of polling. The decoder wakes the waiter after queueing a frame. | `video_decoder/decoder.rs:86-92`; `video_decoder/frame_queue.rs:32-38,65-75` |
| Android path | One feed thread waits in `dequeueInputBuffer`; one drain thread waits in `dequeueOutputBuffer`. Neither sleeps. `operating-rate` is set to the maximum (`i16::MAX`). Output images are `AHardwareBuffer`s from `ImageReader` and are sampled by the renderer without a CPU colour conversion. | `media_codec.rs:4-5,42-58,206-210,244-355`; `image_reader.rs` |
| Apple path | VideoToolbox's callback queues pictures. The feed thread waits on a condition variable only while eight units are in flight. Output is NV12 in an IOSurface with `kCVPixelBufferMetalCompatibilityKey`; BGRA is explicitly avoided because it would force a conversion. | `video_toolbox.rs:1-5,40-55,224-235,246-290`; `decompression_session.rs:35-36,287-310` |
| Frame metadata | `FrameInfo` carries timestamp, app metadata and decode time. `VideoFrame` keeps buffer `size` separate from `CropRect`, so coded padding is not displayed as picture. | `video_decoder/frame.rs:12-48` |
| Capability query | `DecoderCapabilities` reports device/driver limits and `max_frame_rate(width, height)`. Android reads the codec list; VideoToolbox leaves unknown limits as `None` instead of inventing a ceiling. | `video_decoder/capabilities.rs:10-44` |
| Stream contract | The decoder requests no reordering; feeds must have no B-frames, an access-unit delimiter before each picture, and parameter sets before each keyframe. Rising timestamps and access-unit size are checked before admission. | `video_decoder/decoder.rs:16-23`; `video_decoder/admission.rs`; demo comments |
| Surface consumption | `PlayerSurfaceIntake` runs on the composition thread at most once per window frame. The app chooses which queued picture to show and which ones count as dropped. | `cranpose/src/player_surface_intake.rs` |

## Comparison with this crate

| Area | `cranpose-video` today | Consequence |
| --- | --- | --- |
| Thread model | One decode thread does read, pacing, conversion and publication; the UI only takes the published slot. | Simpler and deterministic, but a slow decode or a long `wait_until` also delays command handling and the next decoder read. This is the structural gap called `S2-2` in the architecture review. |
| Frame queue | One published `Slot`; no bounded decoder-output queue, no `latest_frame()`, no decoder-side dropped counter. | There is no queue-depth tuning knob. A late frame is re-anchored instead of being skipped in favour of a newer queued picture. |
| Copy path | Platform frames are copied into tight NV12 planes or RGBA before `VideoFrame` construction. | Correct and portable, but it is a CPU copy/ conversion where upstream lets the compositor sample `AHardwareBuffer` or IOSurface directly. This is the main phone/desktop zero-copy gap. |
| Geometry | `VideoFrame` carries width/height and colour matrix/range. The Windows fix now validates coded size, crop, pitch and buffer length before repacking. | Explicit clean-aperture/crop metadata is still missing from the public frame type; adding it must be additive because existing public structs cannot change. |
| Frame rate | `FrameRate` is an exact ratio and `frame_duration()` derives the period, including 60 fps and fractional rates. | No fixed 30/60 ceiling exists in the pacing path, which matches the requirement for adaptive sources. |
| Diagnostics | Publication intervals, re-anchors, decode/convert timing and events are exposed through the existing observability API. | Upstream additionally counts decoder refusals and drops; those counters are not exposed yet here. |
| Clock recovery | Late picture => re-anchor the clock and publish it; audio follows via `sync_clock`. | Upstream does not have this problem in the decoder layer because feed and presentation are decoupled. Here the observed 1080p re-anchor storm is still a symptom of late decode, not its cause. |

## Non-breaking implications

The upstream design cannot be copied wholesale without violating this task's API constraints. The useful next steps are add-only:

1. Keep `VideoFrame::from_nv12` and all existing public enums/structs unchanged. Add a padded-layout constructor carrying `stride`, `coded`, `crop` and `color` for backends that can expose them.
2. Consider an add-only diagnostics hook for decoder-side refusals/drops if a backend can produce those numbers, rather than adding fields to existing public structs.
3. Treat a future feed/drain split as an alternative decoder engine behind an add-only constructor, not as a change to `VideoPlayer`'s public signatures. The low-memory host used for this work cannot validate such a rewrite safely.
4. Do not add a fixed resolution or frame-rate cap. Use capability queries where the platform provides them, and let unknown limits stay unknown as upstream does.

## Status

This document itself is a source review only: no device run, benchmark, or code change is claimed *from it*. **[REPORT-ONLY]** Later implementation work in the crate changed the pixel threshold and other internals, and the final local Windows probes supersede the old baseline: 720p25 and 1080p25 are both smooth, 60 fps is achievable but load-sensitive (isolated repeats all smooth, one unisolated 1080p60 run stuttered), and AJA HLS remains unresolved. No device run is claimed.
