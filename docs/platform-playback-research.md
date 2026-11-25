# Technical Report: How Global Platforms Play Video

A technical research report on video playback pipelines in global platforms — YouTube, TikTok, Instagram/Meta — focused on what matters for building a Rust video playback library.

## 1. The Playback Pipeline, Stage by Stage

Stages from the play tap to the first rendered frame:

1. **Source acquisition:** the player calls the Playback API, which checks entitlements/region rules and returns a short-lived signed URL to a manifest. The manifest describes available renditions and segment URLs.
2. **Manifest fetch:** download the MPD (DASH) or M3U8 (HLS). Small (kilobytes) but a hard prerequisite for everything after.
3. **Initial rendition selection:** players typically start low — a low-quality segment arrives fast — then climb as throughput is measured.
4. **Segment fetch:** sequential HTTP requests for 2–10 s segments from the nearest CDN edge, with Range support.
5. **Demuxing:** split audio/video/text tracks out of the container (MP4/CMAF/WebM) into samples with timestamps.
6. **Decoding:** decompress samples to raw frames via hardware decoders where available, software otherwise.
7. **Rendering:** push frames to the display surface, synchronized with audio.

Server-side note: modern transcoding pipelines split video at keyframes into short chunks encoded in parallel; the encoded chunks are already the streaming segments, and publishing is just writing manifests. A 5-minute video can be playable ~30–40 s after upload.

## 2. Adaptive Streaming Protocols: DASH and HLS

- **MPEG-DASH:** MPEG standard. An MPD manifest lists representations (bitrate variants) and segment URLs; the client switches between them based on network conditions.
- **HLS (Apple):** M3U8 playlists with TS or fMP4 segments. Dominant on iOS/Safari.
- **CMAF:** unified fMP4 segment format that lets DASH and HLS share the same media segments — one stored copy serves every player. This is what large platforms use.
- **Segment length:** 2–6 s typically. Shorter segments = faster adaptation but more requests and overhead. Short-video platforms favor short segments (2–4 s) to cut time-to-first-frame.
- YouTube adopted DASH with H.264 and VP9 via Media Source Extensions (MSE) in browsers, delivering over 25 billion hours of MPEG-DASH VP9 in the first year after its 2015 HTML5 switch.

## 3. Codecs

| Codec | Efficiency | HW decode | Platform usage |
|---|---|---|---|
| H.264/AVC | baseline | nearly universal | compatibility tier on every platform |
| HEVC/H.265 | ~30–50% better than H.264 | widespread (esp. iOS) | iOS/Apple, HDR, 4K |
| VP9 | ~30% better than H.264 | recent Android + browsers | YouTube workhorse for years |
| AV1 | ~30% better than VP9/HEVC | limited but growing | Meta/Reels, YouTube high tiers |

- **Meta/Reels:** deployed AV1 for Facebook/Instagram Reels in 2022. AV1 cut bitrate ~65% vs AVC and ~48% vs VP9 at equal quality. Decoding is mostly software via dav1d (real-time 720p30 on most devices, 1080p30 on mid/high-end), with hardware decode on newer devices (Pixel 6 Pro, Galaxy S21, ...).
- **YouTube:** H.264 for compatibility, VP9 for efficiency (up to 50% bitrate reduction, >50% less rebuffering globally), AV1 for high tiers. Google built the Argos VCU custom transcoding chip.
- **Common strategy:** an encoding ladder with multiple renditions per video; advanced encodings only for high-projected-watch-time videos (Meta: two-stage hybrid hardware/software ABR encoding with convex-hull dynamic optimizer — e.g. 7 resolutions × 5 CRFs = 35 encodings, then pick the convex hull).
- **HDR:** Meta produces HDR and SDR renditions with server-side tone mapping; uploads from iOS use HEVC Main10, delivery uses AV1 Profile 10.4 (via FFmpeg/libsvtav1) with Dolby Vision metadata carried through.

## 4. Containers

- **MP4 (fMP4/CMAF):** the primary streaming container; fMP4 segments with `moof`/`mdat` per segment.
- **Progressive MP4:** for short videos, a whole MP4 served with Range requests. TikTok/Instagram rely on it heavily for short clips — simpler than DASH/HLS.
- **WebM:** VP9/AV1 alternative container; less used by large platforms now.
- **MPEG-TS:** legacy HLS segments, declining in favor of fMP4.
- **MP4 structure:** `ftyp` (file type), `moov` (track index: tracks, codecs, timing, byte offset of every sample), `mdat` (media data). `moov` placement determines instant-playability (see §8).

## 5. CDN and Delivery

- **Tiering:** edge caches near users → regional mid-tier caches → origin shield in front of storage. Each tier collapses many misses into one upstream request, so the origin sees a tiny fraction of traffic.
- **ISP-embedded caches:** Google Global Cache and Netflix Open Connect place cache servers inside ISP networks, so bytes never cross the expensive transit part of the internet.
- **Access control:** short-lived signed URLs/cookies per video path.
- **Scale:** YouTube pushes hundreds of Gbps at peak; the overwhelming majority of bytes leave the edge, not the origin.
- **Smart segment caching:** the "DASHing YouTube" study proposed placing the first 50 of segments of each video in primary caches, the middle 50–75% in secondary, and the tail in tertiary — since most views never reach the end.

## 6. Adaptive Bitrate (ABR)

- **Principle:** the player measures download throughput and buffer level, then picks the next segment's rendition accordingly.
- **Start low:** the first segment is low-quality so playback starts within ~2 s; then the player climbs.
- **Algorithms:** throughput-based (moving average estimates), buffer-based (BBA), and hybrid (BOLA, MPC). Known problems: repeated over-estimation of available bandwidth and unfair bitrate selection when multiple players compete (Ayad et al., 2018).
- **Switching:** only at segment boundaries, requiring keyframe alignment across renditions (every rendition starts each segment on a keyframe at the same instant).
- **QUIC:** YouTube uses QUIC in browsers; measurements show it is less sensitive to RTT variation than classic TCP-based players.

## 7. Reducing Time-to-First-Frame

Key factors, roughly in order of impact:

1. **Low-quality first segment** — start low, climb later.
2. **faststart / moov placement** — see §8.
3. **Short segments** — a 2 s first segment arrives faster than a 10 s one.
4. **Prefetch/preload before the request** — see §13.
5. **Decoder warm-up** — initialize the decoder before it is needed.
6. **Nearby CDN edge** — low round-trip latency.
7. **Avoiding excess pre-play requests** — don't download more manifest/data than needed.

Reference measurement: a documented case cut mobile p75 time-to-`playing` from 14.8 s to 1.6 s using faststart + Range support (206) + `preload="metadata"`.

## 8. Faststart and moov Placement

- An MP4 is a sequence of boxes: `ftyp`, `moov` (the index a player must read before decoding anything), `mdat` (the data).
- Recorders write `mdat` as they go and append `moov` at the end (they don't know the final sample table until recording stops). Fine for local playback; bad over HTTP — the index is the last thing to arrive.
- **Fix 1 — faststart:** remux so `moov` precedes `mdat` (no re-encode): `ffmpeg -i in.mp4 -c copy -movflags +faststart out.mp4`. The first response contains the index; playback starts as soon as the first seconds of `mdat` arrive.
- **Fix 2 — Range requests:** the server must answer `206 Partial Content` with `Content-Range` so the player can seek to the tail for `moov`, then back to the front. If the server ignores Range and returns 200 with the whole body, the browser must download everything in order.
- **Extra pitfall (ExoPlayer):** non-faststart files force scatter reads (tail then front); recreating the DataSource on every seek cancels in-flight downloads so the buffer never fills — the fix is a chunk-download session that survives seek-driven data-source recreation.

## 9. Hardware Decode and Surface Rendering

- **MediaCodec (Android):** the hardware decode interface. Two output modes:
  - **Surface mode:** hand a Surface to the decoder; frames go from hardware memory straight to display with no CPU copy. Fastest and most power-efficient, but pixels are not readable.
  - **ByteBuffer mode:** frames come out as CPU bytes — slower, but allows processing/filters.
- **Render chain:** decoder → SurfaceTexture (external texture) → GL context → display. Surface changes require teardown/rebuild of the chain — a common source of bugs. The fork between GPU frames (with a native window) and CPU frames (without) is fixed at open time.
- **Frame release timing:** ExoPlayer's VideoFrameReleaseControl decides per frame: release immediately, schedule on a future VSYNC, drop (too late), or try again later. VideoFrameReleaseHelper smooths timestamps against VSYNC intervals and sets `Surface.setFrameRate` from the media frame rate.
- **iOS:** AVSampleBufferDisplayLayer renders decoded frames, with HDR via Dolby Vision metadata consumed alongside each frame.
- **Software decode:** dav1d for AV1 (integrated by Meta in both iOS and Android apps), libgav1 in the Android SDK. Synchronous mode (one frame at a time) minimizes latency; asynchronous mode raises throughput. Meta found 2–4 dav1d threads suffice on iOS; more wastes memory/power.

## 10. Audio/Video Synchronization

- **Reference clock:** audio is usually the master clock (AudioTrack/AudioUnit clock); video follows — each frame is rendered when its PTS matches the audio clock.
- **MediaSync (Android):** a dedicated A/V sync class: audio via AudioTrack, video via `releaseOutputBuffer(idx, presentationTimeNs)` on the Surface, with playback-speed handling built in. Pre-filling buffers at speed 0.0 reduces initial underrun.
- **Drift handling:** if video lags audio, late frames are dropped rather than stalling audio; if video leads, rendering is briefly slowed.
- **Non-1x speed:** changing speed requires audio time-stretching and re-anchoring the video clock.
- **Startup:** pre-filling the audio buffer before starting playback avoids the first underrun.

## 11. Background Playback

- **Android:** playback continues via a foreground service with a MediaSession + MediaStyle notifiction. The player itself is unaffected, but the Surface is destroyed when the Activity leaves — detach and re-attach the surface on return without re-initializing the decoder (ExoPlayer handles this via `setVideoSurface` re-attachment).
- **Picture-in-Picture:** system mode allowing continued rendering in a small window.
- **iOS:** background audio via `AVAudioSession` category `playback`; video via PiP.
- **System constraints:** Doze mode may throttle background network — a sufficient buffer-ahead protects against stalls.
- **Audio focus:** request audio focus and respond to its loss (pause on calls).

## 12. Vertical Feed with Next-Video Preload

The TikTok/Reels pattern: full-screen vertical list, swipe to switch, autoplay.

- **UI structure:** ViewPager2 (page-by-page snapping) or RecyclerView + SnapHelper. ViewPager2 destroys off-screen pages by default, so preloading is the app's responsibility.
- **Player pool:** don't create an ExoPlayer per item — instances are expensive (codec + threads + buffer pools). Use a pool of 3 players (previous, current, next) and swap them across items while scrolling; release `position - 2` and beyond.
- **Preload the next item:** while the current video plays, prepare the next (prepare + buffer, don't play). On swipe, playback starts from the buffer instantly.
- **Fetch feed pages early:** if the next page loads only when the user reaches the last item, a stutter appears every N videos. Load the next page ~3 items before the end.
- **Media3 DefaultPreloadManager:** purpose-built for this pattern:
  - `PreloadConfiguration` for linear playlists: `player.preloadConfiguration = PreloadConfiguration(5_000_000L)` — preloads 5 s of the next item, and only when no media is being loaded for ongoing playback (no bandwidth competition).
  - `DefaultPreloadManager` for dynamic feeds: manages a pool of MediaSources ranked by proximity to the current item (`setCurrentPlayingIndex` + `invalidate`), querying a `TargetPreloadStatusControl` for how much to load per item (e.g. 5 s for next, 3 s for previous, tracks-selected at distance 2, source-prepared at distance ≤4, nothing beyond).
  - On play: `preloadManager.getMediaSource(mediaItem)` returns a MediaSource with preloaded data — a seamless preload→playback transition.
  - Caveat: the returned MediaSource must be played on an ExoPlayer built by the same `DefaultPreloadManager.Builder`.
  - Note: PreloadManager can preload both previous and next items; PreloadConfiguration only looks ahead.
- **Measuring success:** `onRenderedFirstFrame()` for time-to-first-frame; the STATE_BUFFERING → STATE_READY transition for join latency.

## 13. Instant-Playback Techniques

### 13.1 Preload the first seconds
Download the first 3–5 s of each upcoming video before the user reaches it. On TikTok, the first video in the feed is typically already downloaded and its first frame buffered before the user even enters the page. From the second video onward, downloading starts when the current video is selected. Don't over-preload — wasted bandwidth/battery on videos that may never play.

### 13.2 Faststart / moov placement
Detailed in §8. Platform practice: run `-movflags +faststart` on every upload, and guarantee Range (206) support in the server and CDN. Without it, time-to-first-frame grows linearly with file size.

### 13.3 Segment prefetch
- Request upcoming segments before they're needed (buffer ahead) — typically 20–60 s of buffer.
- In a vertical feed: prefetch the first segments of the next video while the current one plays.
- Meta: "we prefetched multiple Reels videos earlier, before they were played" to improve fast scrolling with software AV1 decode.
- Caveat: don't compete with primary playback for bandwidth — Media3 defers preloading until primary loading is idle.

### 13.4 Decoder warm-up
- Creating a MediaCodec/software decoder takes tens to hundreds of ms depending on hardware and codec. Initializing it during preload cuts time-to-first-frame.
- In a vertical feed: decoding the first frame of the next video during preload (decode-to-surface without rendering) makes swipe-to-play nearly instant.
- Meta documented the problem: memory allocation and thread creation for software AV1 decode delayed the first frames, prolonging startup and causing in-play stalls during fast Reels scrolling; the fix was earlier prefetch plus per-device benchmarking (Gaussian blur, memory copy, 3D rendering scores) to pick the deliverable quality tier.

### 13.5 Double buffering
- Two buffers: one plays while the other fills (data or decoded frames). Switching is instant with no wait.
- In players: the network/data-source buffer, the sample buffer, and the decoder output queue are separate stages; each absorbs jitter from the stage before it.
- In rendering: one frame is displayed while the next is decoded — a parallel pipeline (network → demux → decode → render).

### 13.6 Media3 PreloadManager and ExoPlayer
Detailed in §12. Takeaways for a Rust implementation:
- The PreloadManager concept is portable: a list of items ranked by proximity to thecurrent position, with a policy for "how much to load per item" and "in what order".
- Separate preparation (manifest/track selection — cheap) from data loading (expensive).
- Share network sources and decoder state between preload and playback to avoid reconnection costs.

## 14. Summary for a Rust Playback Library

1. **Core support:** progressive MP4 with Range + faststart covers most short-video cases with the simplest architecture.
2. **DASH/HLS later:** for long-form and adaptive streaming. CMAF unifies segments.
3. **Decoding:** hardware via platform APIs (MediaCodec on Android, VideoToolbox on macOS/iOS, DXVA/D3D11VA on Windows, VAAPI on Linux) with software fallback (dav1d/FFmpeg).
4. **Rendering:** direct surface/texture output from the decoder, no CPU copies.
5. **Sync:** audio as master clock; video follows with late-frame dropping.
6. **Vertical feed:** player pool (3) + 3–5 s preload of neighbors + decoder warm-up + early feed-page prefetch.
7. **Metrics:** time-to-first-frame, buffering→ready latency, stall count.

## Sources

1. SysPad — YouTube system design walkthrough (pipeline, ABR, CDN, ladder): https://syspad.io/interview-practice/youtube-pipeline
2. Breejesh Rathod — Design a YouTube-Scale Video Platform: https://breejeshrathod.com/blog/en/design-youtube-streaming
3. DASHing YouTube (IEEE LCN 2013): https://doi.org/10.1109/lcn.2013.6761273
4. YouTube HTML5/VP9 Case Study (Google Developers): https://developers.google.com/media/pdf/Youtube-HTML5-VP9-Case-Study.pdf
5. Meta Engineering — How Meta brought AV1 to Reels: https://engineering.fb.com/2023/02/21/video-engineering/av1-codec-facebook-instagram-reels/
6. Meta Engineering — Reducing Instagram's basic video compute time by 94%: https://engineering.fb.com/2022/11/04/video-engineering/instagram-video-processing-encoding-reduction/
7. Meta Engineering — Bringing HDR video to Reels: https://engineering.fb.com/2023/07/17/video-engineering/hdr-video-reels-meta/
8. Meta Engineering — Enhancing HDR on Instagram for iOS with Dolby Vision: https://engineering.fb.com/2025/11/17/ios/enhancing-hdr-on-instagram-for-ios-with-dolby-vision/
9. Android Developers Blog — Introducing preloading with Media3: https://developer.android.com/blog/posts/elevating-media-playback-introducing-preloading-with-media3-part-1
10. Android Developers — Preload manager concepts: https://developer.android.com/media/media3/exoplayer/preloading-media/preloadmanager/concepts
11. androidx/media issue #2804 — Benchmarking PreloadConfiguration: https://github.com/androidx/media/issues/2804
12. androidx/media issue #3244 — Zero inter-presentation delay in short-form feeds: https://github.com/androidx/media/issues/3244
13. bytevikas — Building a TikTok-Style Vertical Feed with ExoPlayer: https://bytevikas.com/blog/tiktok-feed.html
14. Johnson Lee — Short Video Preloading (Part 1): https://johnsonlee.io/2021/02/10/short-video-preloading-1.en/
15. Stack Overflow — How does TikTok stream videos in Android: https://stackoverflow.com/questions/57555351/
16. hibug.me — Fix Slow MP4 Start: moov Atom and Faststart: https://www.hibug.me/mp4-slow-start-moov-atom-faststart
17. NuvioTV PR #3102 — Non-faststart MP4 playback fix: https://github.com/NuvioMedia/NuvioTV/pull/3102
18. Android Developers — MediaSync API: https://developer.android.com/reference/android/media/MediaSync
19. DeepWiki — androidx/media Video Rendering Pipeline: https://deepwiki.com/androidx/media/3.3-video-rendering-pipeline
20. panvox — Android hardware decoding: MediaCodec and the Surface lifecycle: https://panvox.net/en/tech/build/ch3-hwdecode/
21. Ayad et al. — A Practical Evaluation of Rate Adaptation Algorithms in HAS (Computer Networks 2018): https://eric-keller.github.io/papers/2018/adaptive_streaming_comnet2018.pdf
