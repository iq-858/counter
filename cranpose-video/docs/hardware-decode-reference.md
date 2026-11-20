# Hardware decode reference — vendor sources and what they mean here

This document records what the **official vendor documentation** actually says about GPU, CPU and hybrid video decoding, and maps each statement to a decision in `cranpose-video`. It is a **static** document: the URLs were read, no device or benchmark was run for it. The performance numbers it refers to are the ones already measured on this host and recorded in `README.md`. **Later update:** `cancelReading` in Apple's `open_at` is now source-fixed but device-unverified; the pixel threshold is `1920 * 1080`; and the final local 1080p60 numbers are in the table in §1 and in `README.md`.

Sources, read on 2026-11-18:

| Source | URL |
| --- | --- |
| FFmpeg `ffmpeg(1)`, `-hwaccel` | <https://ffmpeg.org/ffmpeg.html#Hardware-acceleration> |
| mpv `options(1)`, `--hwdec` | <https://raw.githubusercontent.com/mpv-player/mpv/master/DOCS/man/options.rst> |
| Android `MediaCodec` | <https://developer.android.com/reference/android/media/MediaCodec> |
| Android `ImageReader` | <https://developer.android.com/reference/android/media/ImageReader> |
| Android NDK `AHardwareBuffer` | <https://developer.android.com/ndk/reference/group/a-hardware-buffer> |
| Apple `VTDecompressionSession` | <https://developer.apple.com/documentation/videotoolbox/vtdecompressionsession> (JSON: `.../vtdecompressionsession.json`) |
| Apple `AVAssetReader` | <https://developer.apple.com/documentation/avfoundation/avassetreader> (JSON: `.../avassetreader.json`) |

---

## 1. The central caveat: decoding on the GPU is not automatically faster

FFmpeg's own manual says, verbatim, under `-hwaccel`:

> "Note that most acceleration methods are intended for playback and will not be faster than software decoding on modern CPUs. Additionally, ffmpeg will usually need to copy the decoded frames from the GPU memory into the system memory, resulting in further performance loss. This option is thus mainly useful for testing."

The accepted values are `none` (default), `auto`, `vdpau`, `dxva2`, `d3d11va`, `vaapi`, `qsv` and `videotoolbox`. `qsv` is called out as *not* accelerated decoding but accelerated transcoding "without copying the frames into the system memory" — the distinction is the copy, not the silicon.

**What this means here.** A GPU decoder only wins when the decoded picture never comes back to system RAM. Our measured path already separates those two costs:

| **[REPORT-ONLY]** Earlier local path (average from a 6 s 1080p60 `stutter_probe`; cited log absent from this checkout) | Decode | Pooled RGBA conversion |
| --- | --- | --- |
| 1080p60, no native audio | 1.44 ms | 6.65 ms |
| 1080p60, native audio | 1.48 ms | 6.74 ms |
| Earlier RGB32 decoded by MF with the video processor converting | — | ~79 ms (historical, not the final path) |
| NV12 handed over as planes with no conversion | — | no work |

**[REPORT-ONLY]** The older 79 ms case is the FFmpeg warning in concrete form: a conversion/copy-back path costing an order of magnitude more than decode. It is why the Windows backend asks for NV12 first, not a current path. Adding a GPU decoder that hands frames back as CPU RGBA would reintroduce that loss, so it is **not** proposed. The only GPU path worth measuring is decoder → GPU texture → compositor with no readback. These averages do not prove smooth presentation: the same final native-audio 1080p60 run still had two long gaps.

## 2. mpv: hardware decode is opt-in, paired with a matching output, and has an explicit copy-back mode

mpv's `--hwdec` defaults to `no`. The manual's rationale:

> "Hardware decoding is not enabled by default, to keep the out-of-the-box configuration as reliable as possible. However, when using modern hardware, hardware video decoding should work correctly, offering reduced CPU usage, and possibly lower power consumption. On older systems, it may be necessary to use hardware decoding due to insufficient CPU resources; and even on modern systems, sufficiently complex content (eg: 4K60 AV1) may require it."

Special values: `no`, `auto`, `auto-unsafe`, `yes`/`auto-safe` (same as `auto`), and `auto-copy`. `auto` restricts itself to a whitelist of actively supported methods; `auto-unsafe` tries everything and is for debugging. Hardware decoding is also codec-whitelisted (`--hwdec-codecs` widens it), and mpv recommends testing with the real content before enabling it by default, with a runtime toggle (`Ctrl+h`).

Actively supported methods and their **required output**:

| Method | Requirement | Copy-back variant |
| --- | --- | --- |
| `d3d11va` | `--vo=gpu`/`gpu-next` with `--gpu-context=d3d11` or `angle` (Windows 8+) | `d3d11va-copy` |
| `videotoolbox` | `--vo=gpu`/`gpu-next` (macOS) or `--vo=libmpv` (iOS 9+) | `videotoolbox-copy` |
| `vaapi` | `--vo=gpu`, `gpu-next`, `vaapi` or `dmabuf-wayland` (Linux) | `vaapi-copy` |
| `nvdec` | any platform with CUDA | `nvdec-copy` |
| `drm` | `--vo=gpu`/`gpu-next` (Linux) | `drm-copy` |
| `vulkan` | `--vo=gpu-next` with Vulkan Video Decoding | `vulkan-copy` |

Other methods, "only use if you know you have to": `dxva2`, `vdpau`, **`mediacodec` (requires `--vo=gpu --gpu-context=android` or `--vo=mediacodec_embed`)**, `mediacodec-copy`, `cuda`, `crystalhd`, `rkmpp`.

**What this means here.** Three design rules are directly transferable:

1. **The zero-copy path is a pair, not a decoder.** mpv requires the *output* to match the decode API. Our equivalent is `player.set_native_nv12(true)` plus a screen that samples two textures; enabling the decoder flag alone cannot help. This is why the README states the flag is for "a screen that samples NV12 in a shader".
2. **Keep a copy-back mode.** Every mpv hwdec has a `-copy` twin for when surfaces cannot be shared. Our RGB32 request is that twin: the current fallback for a track the planar path cannot serve. Do not delete it.
3. **Opt-in with a safe default and a runtime escape.** mpv ships `hwdec=no` and a whitelist. Our default RGBA upload plus additive opt-in follows the same shape. Any future GPU-texture intake must be opt-in and must fall back to the CPU path when the surface cannot be shared — never a hard requirement.

## 3. Android: `PRIVATE` surface for display, `YUV_420_888` for CPU, `AHardwareBuffer` to join them

The `ImageReader` manual states that direct application access to image data rendered into a `Surface` is available, and that `PRIVATE` formats are **more efficient** when application access is unnecessary, compared with a reader using a format such as `YUV_420_888`. Not all format/usage combinations are supported.

The NDK `AHardwareBuffer` page states that buffers "can be accessed by various hardware components", that passing one between processes is **zero-copy** (a shared view of the same memory), and that a buffer can be bound to EGL/OpenGL via `eglGetNativeClientBufferANDROID` → `eglCreateImageKHR`, or to Vulkan via `VK_ANDROID_external_memory_android_hardware_buffer`.

`MediaCodec` is documented as the low-level codec component, normally used with `MediaExtractor` and `MediaSync`.

**What this means here.** These are exactly the two paths in `docs/phone-integration-review.md`:

- **Target (GPU):** `MediaCodec` output configured for a `Surface`/`AHardwareBuffer`, imported into a Makepad native texture through EGL/Vulkan. No CPU row read. This is the mpv `mediacodec` + `gpu-context=android` pairing.
- **Near term (hybrid):** `YUV_420_888` → normalised NV12 planes → two GPU textures. This keeps a CPU read, but avoids the far worse RGB32 conversion. Our `android.rs` already reads `rowStride`/`pixelStride` (`android.rs:921-928`, `951-956`) and colour data (`android.rs:830-836`); the open questions are chroma order, clean aperture and BT.2020/PQ/HLG, which need device proof.

The choice must be made per device, not globally. A phone that can share the buffer takes the GPU path; one that cannot keeps the hybrid path. Both remain behind the same non-breaking decoder API.

## 4. Apple: `AVAssetReader` and `VTDecompressionSession` are both available on iOS

The Apple documentation metadata confirms the platforms:

- `VTDecompressionSession` (Video Toolbox): iOS 6.0+, iPadOS 6.0+, Mac Catalyst 13.0+, macOS 10.8+, tvOS 10.2+, visionOS 1.0+.
- `AVAssetReader` (AVFoundation): iOS 4.1+, iPadOS 4.1+, Mac Catalyst 13.1+, macOS 10.7+, tvOS 9.0+, visionOS 1.0+.

**What this means here.** `src/platform/apple.rs` uses `AVAssetReader` and requests `420v` (NV12). That is a supported, documented iOS path — no device run is claimed in this session, but the platform is not in doubt. The untested step is the GPU handoff: `CVPixelBuffer`/IOSurface → Metal texture owned by Makepad. As with mpv, that handoff is the part that decides whether the frames stay on the GPU.

Two correctness items must be proven before a GPU path is enabled on iOS, and both are already listed as device work:

- Read colour matrix/range and clean aperture from the format description instead of guessing from dimensions (`apple.rs:154-171`, `358-359`). Padded aperture and HDR sources are the test cases.
- `cancelReading` in `open_at` is now source-fixed as in `close` (`apple.rs:154-162`, `369-373`); device work remains: measure seek blocking time to confirm a reader is not left running.

## 5. Low-power under high load: what the sources support and what we can claim

Combining the three sources, the low-power recipe is consistent:

| Rule | Vendor basis | Our state |
| --- | --- | --- |
| Prefer decode into a planar format the compositor can sample | FFmpeg `-hwaccel`, mpv zero-copy hwdec | Done: NV12 on Windows/Apple/Android/GStreamer |
| Avoid decode→system-RGBA copy-back | FFmpeg's explicit warning, **[REPORT-ONLY]** 79 ms measurement | Done: RGB32 is fallback only |
| Pair decoder with output before claiming a GPU win | mpv `--vo`/`gpu-context` requirements | Partial: `set_native_nv12` exists, texture intake untested |
| Keep a software/fallback path | mpv `hwdec=no` default and `-copy` variants | Done: RGBA default, RGB32 fallback |
| Make acceleration opt-in and content-tested | mpv whitelist, `Ctrl+h`, `hwdec-codecs` | Done for the flag; device matrix pending |
| Do not impose a resolution cap | mpv cites 4K60 AV1 as a reason to enable hwdec | Done: no fixed cap; adaptive HLS rendition selection |

**Not claimed:** no vendor document here proves a specific watts-per-frame figure, and no power measurement was taken. Power reduction is a consequence of removing CPU work, which we *did* measure **[REPORT-ONLY]**: the historical RGB32 path cost ~79 ms versus the final 1080p60 native-audio probe at 1.48 ms decode + 6.74 ms pooled RGBA conversion, and 1080p RGBA conversion dropped from 11.63 ms (one worker) to 6.86 ms (two). The remaining proof is a device power probe, tracked as H-1 in [README.md#identified-in-review--not-yet-fixed](../README.md#identified-in-review--not-yet-fixed).

## 6. What would change in this crate, and what will not

No code change is made by this document. Any future change must remain additive and non-breaking:

- **Would change:** add an optional native-texture intake (AHardwareBuffer on Android, IOSurface/CVPixelBuffer on Apple) with explicit layout, crop and colour metadata; keep the existing CPU/RGBA path as the default.
- **Would not change:** public method signatures, `VideoState` variants, fields on existing public structs, the clock/resync rules, or the requirement that audio failure never breaks video.
- **Must be measured before merging:** per-frame publication intervals at 60 fps, `diagnostics().convert_time()`, decode time, memory high-water mark, and — on device — power/thermal. A path that only moves cost from CPU to a GPU readback is a regression, not an optimisation.
- **[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]:** reserved slot for the Linux GStreamer and multi-reel figures named above. Fill only from a captured run with its exact command, feature set, environment and output. Do not invent or copy numbers into this slot; until then it carries no measured value.

See `docs/technique-catalog.md` for the fence and per-frame metadata proposals, and `docs/upstream-decoder-comparison.md` for the bounded-queue/feed-thread pattern used by upstream Cranpose.
