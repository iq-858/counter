# Hardening audit & execution plan — 2026-11-19

Labels used throughout:

- `[REPORT-ONLY]` — historical observation; not proof of the current source state.
- `[SOURCE-VERIFIED]` — read directly from the cited source, optionally with a unit test; **not** a device measurement.
- `[HYPOTHESIS]` — runtime/device consequence inferred from source or tester reasoning; not measured.
- `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]` — intentionally empty; no number may be invented.

---

## 1. الحكم التنفيذي / Executive verdict

**Arabic — الحكم.** المشكلة لم تكن "كثرة خيوط" بل ثلاثة عيوب: **حدود إلغاء غير مكتملة عند الإغلاق**، و**فقدان الصوت بعد التبديل** بسبب فشل صامت في `Audio::setup()` مع ربط احتياطي لمرة واحدة يعود مبكرًا فيبقى الصوت مفقودًا مدى حياة المشغّل، و**Latch خاطئ في الصوت** يعتبر أي خطأ قراءة نهايةً نهائية. بعد التعديل: الإغلاق يُرسل الإشارة ويتخلى عن الخيط بدل انتظاره، والربط الاحتياطي الفاشل يُعاد محاولته بتحويل زمني مع الإبقاء على عدم إعادة التجهيز لكل إطار في المصادر بلا صوت، والصوت يميّز بين نهاية حقيقية وخطأ قابل لإعادة المحاولة، وتمنع الإطارات القديمة من الظهور بعد تغيير المصدر. هذا **تحسين مصدرِي مُتحقَّق منه**، وليس إثباتًا سمعيًا على جهاز.

**English — verdict.** The dominant defects were not thread count: (1) an **incomplete cancellation boundary** — `close()` waited without bound on a decode thread stuck in `read()`; (2) **audio loss after a switch** — a silent `Audio::setup()` failure combined with a one-shot fallback attach that early-returned forever, making silence permanent for that player; (3) an **over-broad audio EOF latch** that treated a transient read error as terminal. The applied fixes detach instead of joining, seal events after shutdown, retry a failed fallback attach under a throttle while preserving no-per-frame prepare for no-audio sources, make audio EOF terminal only for true `ENDOFSTREAM`, refuse to start a silent clock on an empty buffer, and clear stale UI pictures on source change. This is **source-verified hardening**, not a device-level audible proof.

---

## 2. What was applied

| # | Change | Exact anchor | Mechanism | Status |
|---|---|---|---|---|
| A1 | Non-blocking close | `src/player/threaded.rs:95-106` | Sends `Command::Shutdown`, then drops the `JoinHandle`. The worker still owns `Box<dyn VideoDecoder>` (`:111`) and closes it on exit (`:223`); no reclaim from the caller, so no UAF/double-drop path is introduced. | `[SOURCE-VERIFIED]`; RED test green (`src/player.rs:1466`) |
| A2 | Both shutdown signals retained | `src/player.rs:259-272`, `src/player/threaded.rs:104`, `:155`, `:271` | `AtomicBool` covers a decoder observing shutdown inside a read; the command covers a thread between reads. Removing either would reopen a missed-wakeup path. | `[SOURCE-VERIFIED]` |
| A3 | Post-read shutdown guard | `src/player/threaded.rs:316-325` | If shutdown lands while `read()` was in flight, scratch/planes are returned and the worker exits **before** `finish()`, `fail()`, publication, position writes, or `sync_clock()`. | `[SOURCE-VERIFIED]` |
| A4 | Event sealing | `src/player.rs:237-248` | Once the flag is set, only `PlaybackEvent::Closed` may be emitted. `close()` sets the flag before `emit_closed()` at `src/player.rs:1294-1312`, so `Closed` remains last despite a detached worker. | `[SOURCE-VERIFIED]`; guarded by `tests/observability.rs:285-292` |
| A5 | Prime deadline | `src/platform/windows/mf_audio.rs:77`, `:264-269` | A fresh start with no PCM stops waiting after 500 ms and re-anchors; a unit test backdates the wall clock instead of sleeping (`src/platform/windows/mf_audio.rs:1402`). | `[SOURCE-VERIFIED]`; audible kickoff `[HYPOTHESIS]` |
| A6 | Terminal vs recoverable audio flags | `src/platform/windows/mf_audio.rs:120-129`, `:1009-1012` | Only `MF_SOURCE_READERF_ENDOFSTREAM` sets `ended`. `HRESULT` failure and `MF_SOURCE_READERF_ERROR` return retryably to the next video read inside the existing `PUMP_BUDGET`. | `[SOURCE-VERIFIED]`; regression test `:1304` |
| A7 | Empty-buffer start guard | `src/platform/windows/mf_audio.rs:133-138`, `:1084` | `start_ready()` starts on a full lead; a true EOF tail may start with non-empty PCM; an empty buffer never starts a running silent clock. | `[SOURCE-VERIFIED]`; regression test `:1285` |
| A8 | Resume clears EOF | `src/platform/windows/mf_audio.rs:1046-1051` | `Audio::play(&mut self)` clears `ended`; a truly exhausted reader reports EOF again on the next pump. | `[SOURCE-VERIFIED]` |
| A9 | Stale UI frame clear | `D:\counter\src\app_ui\reels.rs:99-105` | The source-keyed pump clears `picture` and `resolution` before `play()`/`take_frame()`; a slow or failed new source now shows a placeholder instead of the previous clip. | `[SOURCE-VERIFIED]` |
| A10 | Closed-player pump guards | `D:\counter\src\app_ui\reels.rs:113-117`, `:137-141` | Both the frame and resolution pumps break on `is_closed()`. | `[SOURCE-VERIFIED]` |
| A11 | Retryable fallback audio attach | `src/audio.rs:63`, `:129`, `:227-288`, `:567-575` | A failed `prepare()` no longer latches `attached=true` forever: `ensure_attached` keeps the wrapper unattached until `Audio::is_ready()`, and `Audio::attach` retries a failed source only after `ATTACH_RETRY_COOLDOWN` (500 ms). A ready source and a source inside its cooldown are no-ops; `detach()` clears the retry state. This preserves the no-per-frame `prepare()` behaviour for sources with no audio track while letting a device briefly held by a retiring switch recover. | `[SOURCE-VERIFIED]`; 2 device-free unit tests (`:893`, `:931`) |

### 2.1 Switch-audio root cause — `[SOURCE-VERIFIED]` chain

The switch-specific symptom (sound disappears after switching videos while video continues, even after further switching) traces to a silent failure, not to the EOF latch:

```text
setup() fails  (mf_audio.rs:748 / build_output_stream_raw :821 / play() :1046)
  -> carries_audio() == false  (windows.rs:949-955)
  -> wrap_direct fallback      (audio.rs:527-540)
  -> one-shot ensure_attached  (audio.rs:567-575, pre-fix attached=true unconditionally)
  -> Audio::attach early-return on same source with ready=false (audio.rs:227-288, pre-fix)
  -> permanent silence for that player
```

A6/A8 remain valid for a **fresh transient latch** on a newly created player, but an `ended` flag cannot be inherited across players (`Audio` is per-`DirectAudioDecoder`, `audio.rs:548`), so the latch fix is **not** the switch-specific cure. A11 addresses the dominant switch cause. The reason `setup()` fails after a switch is still `[HYPOTHESIS]`: a detached native worker can keep its MF reader/cpal stream because native `Reader` has no `observe_shutdown` override (`decoder.rs:174` default; `platform/windows.rs` reader). A11 lets a later read recover without needing to identify that cause.

### 2.2 Adversarial findings and rejected options

| Proposal | Verdict | Reason |
|---|---|---|
| Exit-token / deferred-open handshake so exactly one decoder exists at every instant | **Rejected** | `cranpose-core-0.24.0/src/hooks.rs:41-42` runs the new `init` before the old value is dropped, so overlap predates the detach change; a strict exit wait can strand the UI on a stuck decoder (reopens F1). One-at-a-time remains a composition-level invariant, not a measured runtime fact. |
| Reclaim/drop the decoder from `close()` | **Rejected** | The detached worker is the sole owner of `Box<dyn VideoDecoder>` (`threaded.rs:111`); caller-side reclaim is a use-after-free / double-drop path. |
| Arbitrary timeout or reaper for a stuck decode shutdown | **Rejected** | Ownership race and arbitrary behaviour; no fixed deadline was added. A11's 500 ms is a **retry throttle**, not a shutdown deadline. |
| Root wgpu `with_gpu_requirements(...)` device-creation fix | **Reverted (out of scope)** | Source-verified but unrelated to this hardening scope; Windows device creation still fails at `requested 65535, allowed 0` `[REPORT-ONLY]`. |

### 2.3 Residual review findings recorded, not claimed fixed

These were raised in adversarial review and are **not** presented as resolved. They stay as honest residuals:

| Finding | Label | Position |
|---|---|---|
| `start_ready` can hold a fresh start until `PRIME_TIMEOUT` | `[SOURCE-VERIFIED]` behaviour, audible effect `[HYPOTHESIS]` | Accepted tradeoff: starting on an empty buffer would run a silent clock (A7). The 500 ms deadline (`mf_audio.rs:77`, `:264-269`) bounds it; no unbounded wait exists. |
| Stale-frame repaint race between A9 clear and the new source's first frame | `[HYPOTHESIS]` | A9 clears at pump start (`reels.rs:99-105`); a late frame from the retiring decoder published in that window is not gated by the UI clear. One-decoder overlap (F7) is the same unresolved boundary. |
| Telemetry counters can advance after `Closed` | `[SOURCE-VERIFIED]` mechanism | Event sealing (`player.rs:219-230`) governs `PlaybackEvent`, not raw counters; a detached worker's late metric write is not filtered. No public field/variant change was permitted to expose or gate it. |
| Final audio sample may be dropped on EOF/error path | `[SOURCE-VERIFIED]` code shape, audible effect `[HYPOTHESIS]` | The tail-start guard (A7) requires non-empty PCM; a trailing sample arriving after the start decision is not re-evaluated. Not changed: altering it risks the silent-clock regression A7 prevents. |

No pacing gate was relaxed. The gates remain `FPS_60=60`, `PERIOD_60=16_666_666 ns`, `SAMPLES_60=48`, `WARMUP_60=3`, n=44, `p50<=1.35`, `p95<=1.75`, `max<=2.5`, `long_gaps<=2`; the real-source probe remains `max<=2.0`, `long_gaps<=max(1,1%)`, `reanchors<=1`, `errors==0`. No public signature, enum variant, or public struct field changed. No arbitrary resolution cap was added.

---

## 3. State machine / آلة الحالة

### 3.1 Player lifecycle

```text
OPENED
  └─(first published frame)─> PLAYING ──(pause)──> PAUSED
                                │                    │
                                ├─(seek)──> SEEKING ─┤
                                ├─(EOF)───> ENDED    │
                                └─(error)─> FAILED   │
                                                       v
close() on any state:  begin_shutdown() -> Command::Shutdown -> detach
                       -> clear slot -> emit Closed (last event)
```

Invariants `[SOURCE-VERIFIED]`:

1. `Playing` is released on the first published picture (`src/player.rs:465-476`), while `state()` still reports `Playing` immediately on `play()` by design.
2. After shutdown, no non-`Closed` event may be appended, even if a detached worker is still unwinding.
3. The worker, not the caller, owns decoder destruction.
4. A read returning after shutdown produces no publication, clock write, re-anchor, or event.

### 3.2 Windows audio span

```text
play() clears ended
   -> pump() reads PCM on the video decode thread
      ├─ ENDOFSTREAM -> ended = true; tail may start if PCM exists
      ├─ ERROR / HRESULT failure -> retry next pump (no latch)
      └─ sample -> push into contiguous PCM buffer
   -> sync_to(picture pts):
      ├─ filled >= pts + START_LEAD -> start
      ├─ ended && filled > pts       -> start with the tail
      └─ otherwise                   -> stay waiting; 500 ms prime deadline releases a starved start
```

A/V invariants `[SOURCE-VERIFIED]`:

- Picture PTS remains the master clock; audio `sync_to()` follows it (`src/player/threaded.rs:428-438`, `src/platform/windows/mf_audio.rs:977-992`).
- No sample-rate, channel-count, or sample-format conversion is allowed on route rebind; an exact PCM match is required.
- Speed changes the clock, never sample playback rate or pitch.
- Fading occurs only at silence/gap edges via `DECLICK`, not as a response to clock drift.

---

## 4. Failure taxonomy / تصنيف الأعطال

| ID | Failure | Anchor | Label | Severity before | Residual after A1–A10 |
|---|---|---|---|---|---|
| F1 | Unbounded `join()` on a stuck read | `threaded.rs:91-92` (pre-fix) | `[SOURCE-VERIFIED]` | Critical | Removed by A1 |
| F2 | Shutdown flag unobservable inside a blocking read | `decoder.rs:194-202` default, `windows.rs:649-679` | `[SOURCE-VERIFIED]` mechanism; indefinite blocking `[HYPOTHESIS]` | Critical | Close no longer waits; cooperative decoders still exit promptly |
| F3 | Audio error latched as permanent EOF | `mf_audio.rs:888-893` (pre-fix) | `[SOURCE-VERIFIED]` | Critical | Removed by A6/A7/A8 for a fresh latch; not the switch cause (see F11) |
| F4 | False `Ended` on close-mid-read | `threaded.rs:242-263` + `player.rs:406-409` | `[SOURCE-VERIFIED]` mechanism | High | Removed by A3 + A4 |
| F5 | Stale previous-source picture | `reels.rs:92` (pre-fix) | `[SOURCE-VERIFIED]` | High | Removed by A9 |
| F6 | Old audio device stranded on switch | `audio.rs:485-493`, `threaded.rs:183` | `[SOURCE-VERIFIED]` code path; audible effect `[HYPOTHESIS]` | High | Reduced by A11: a later read retries the prepare after 500 ms; a permanently stuck uncooperative decoder can still hold its device until it returns |
| F7 | Two decoders transiently overlap during switch | `reels.rs:87-88`, `hooks.rs:41-42` | `[SOURCE-VERIFIED]` new-init-before-old-drop; audible effect `[HYPOTHESIS]` | Medium | Composition-level gate still one-at-a-time; overlap predates detach. Handshake fix rejected (§2.2), so transition overlap remains, not measurable from this tree |
| F11 | Silent `setup()` failure + one-shot fallback latch permanent silence | `windows.rs:886-887`, `audio.rs:445-460`, `:485-493`, `mf_audio.rs:713` | `[SOURCE-VERIFIED]` chain; device cause `[HYPOTHESIS]` | Critical | Removed by A11: failed attach stays unattached and is throttled-retried on later reads |
| F8 | Audio loss has no event/counter | `telemetry.rs:88-139`, `hls/mod.rs:525-526` | `[SOURCE-VERIFIED]` | Medium | Unchanged: no public enum variant was allowed |
| F9 | Failure evidence lost when app drops player | `reels.rs:87-88`, `telemetry.rs:365-386` | `[SOURCE-VERIFIED]` | Medium | Unchanged; needs a caller-side snapshot if desired |
| F10 | Live rendition switch drops up to 3 buffered segments | `hls/mod.rs:43`, `:324-328`, `:882-886` | `[SOURCE-VERIFIED]` | Medium by design | Unchanged; explicit live-edge policy, not a bug fix |

---

## 5. Probability matrix / مصفوفة الاحتمالات

All numbers in this section are `[HYPOTHESIS]` tester estimates from source reading, **not measured rates**. They are included because the user asked for many probabilities from user/tester perspective. "Before" is the original code; "after" is reasoned residual after A1–A11. A11 (§2.1) is expected to lower the "permanent audio silence after switch" row further than shown, because a failed attach is now retried on later reads instead of latching for the player's lifetime; the table is not re-measured and no number is invented.

| Scenario | Before — normal | Before — 10 switches/s | Before — 30× | After — normal | After — 10 switches/s | After — 30× |
|---|---:|---:|---:|---:|---:|---:|
| `close()` hang / UI freeze | 10 | 45 | 60 | 2 | 5 | 10 |
| Permanent audio silence after switch | 15 | 45 | 45 | 3 | 8 | 12 |
| Stale previous frame visible | 2 | 95 | 99 | 1 | 5 | 10 |
| Event after `Closed` | 2 | 5 | 5 | 0 | 0 | 0 |
| A/V desync > 1 frame | 8 | 35 | 65 | 5 | 15 | 25 |
| Two decoders alive during transition | 5 | 40 | 65 | 3 | 15 | 30 |
| Error swallowed with no event | 30 | 55 | 70 | 30 | 55 | 70 |
| Crash needing restart | 2 | 10 | 20 | 2 | 6 | 12 |

Why the residual is not zero:

- Stalled network / driver reads that ignore the shutdown flag still occupy a detached worker until they return. We deliberately did **not** add a timeout or reaper: both were reviewed and rejected because they would create ownership races or arbitrary behavior.
- Audio-loss events remain invisible by construction because adding a `PlaybackEvent` variant is forbidden by the non-breaking API constraint.
- Framework cancellation timing around `rememberKeyed`/`LaunchedEffect` lives in `cranpose-core`, not this tree, so "one decoder at every instant" remains a composition-level invariant rather than a measured runtime counter.

---

## 6. Fast-switch and data-loss checks

1. **Open is now lazy, not a network wait.** `VideoPlayer::open_hls` / `open` → `platform::open` (`platform/mod.rs:121-132`) → `hls::open` (`hls/mod.rs:121-128`) → `Stream::new(url)` (`:338-397`) only stores the URL and builds a literal empty playlist with `loaded = false` via the additive `playlist::Media::empty()` (`playlist.rs:99-108`); the first playlist/window fetch happens in `ensure_loaded()` (`:405`), called only from `seek()` (`:1165`) and `next_picture()` (`:1316`) on the decode thread. `open()` never calls `info`/`output_format` network paths. This removes the former startup/switch latency amplifier `[SOURCE-VERIFIED placement]`; the latency win itself is `[HYPOTHESIS]` until a device run is captured.
2. **Settled-page gate retained.** Only the settled page composes an `ActiveReel` (`reels.rs:62-68`); Live uses the same helper. Reels and Live are separate root arms, so they cannot both compose a player `[SOURCE-VERIFIED]`.
3. **Frame queue is deliberately one newest picture** with two spares, not an unbounded backlog (`player.rs:48`, `:549-635`). Coalescing is intentional; no frame backlog data loss is introduced.
4. **Stale frames are filtered at the engine** by seek generation and pre-seek PTS checks (`threaded.rs:405-416`, `:528-549`) and at the UI by A9.
5. **Live rejoin is explicit media loss**, not accidental: `LIVE_BACK=3` rejoins at the live edge on initial live open and on ABR switch (`hls/mod.rs:413-420`, `:947-951`) `[SOURCE-VERIFIED]`.
6. **Cooperative HLS shutdown is bounded by checks**, not by a hard cancel: flag installed before the decoder moves to the worker (`player.rs:875`), adopted by `observe_shutdown` (`hls/mod.rs:1245-1248`), and observed in fetch sleeps and waits (`hls/mod.rs:1067`, `:1689`, `:1492`) `[SOURCE-VERIFIED]`.
7. **Detached workers terminate on the demo's local-clip path** `[SOURCE-VERIFIED]` mechanism, `[HYPOTHESIS]` timing: the worker rechecks `is_shutting_down()` at the loop top (`threaded.rs:155`), inside `step()` before the read (`:295`) and again after it (`:321`), then closes its decoder on exit (`:223`). Native `Reader::close()` drops the cpal stream and MF reader (`platform/windows.rs:975`). `ReadSample` (`platform/windows.rs:734`) is synchronous and has no `observe_shutdown` override (`decoder.rs:202` no-op), so a worker blocked in a **network** read stays there until MF returns; for local `CLIPS` the read returns promptly and the worker exits on the next check, so detached workers do not accumulate during ordinary Reels switches. A permanently stalled network read is the same residual already recorded in F6; no timeout/reaper was added (§2.2).

---

## 7. Observability gaps and proposed SLIs — not applied

No public struct field or event variant may be added, so these are **design recommendations only** `[HYPOTHESIS]`:

| Signal | Why | Current gap |
|---|---|---|
| `stalled_reads` | Distinguish a slow decode from a hung read | `threaded.rs:293-341` has no deadline observation |
| `audio_prime_timeouts` | Count 500 ms releases | `mf_audio.rs:264-269` mutates state silently |
| `ended_latches` | Separate true EOF from historical error latch | `Ended` folds no counter at `telemetry.rs:109`, `PlaybackDiagnostics` at `:365` |
| `stale_frames_dropped` | Detect a decoder that cannot keep up | `threaded.rs:405-416`, `:528-549` drops silently |
| `audio_rebind_attempts/successes/failures` | Detect route churn after switch | `mf_audio.rs:874-961` result is effectively ignored by caller |
| `audio_attach_retries` | Detect a failed fallback prepare recovering (A11) | `audio.rs:227-288` reschedules silently; the two unit tests prove logic, not runtime frequency |
| `events_after_closed` | Guard the sealing invariant itself | `tests/observability.rs:285-292` checks order, not count |
| `decoders_alive` / `decoder_reopens` | Make one-at-a-time measurable | No runtime counter exists |

Probe SLIs that already exist and must not be weakened: publication p50/p95/max, long-gap count, reanchor count, error count, and event order (`telemetry.rs:310`, `:450-455`, `tests/pacing.rs:723-838`).

---

## 8. Verification protocol

Do **not** run `cargo check --all-targets` from `D:\counter`; the historical example failure is outside this fix. Use library and named test targets only.

```powershell
# RED -> GREEN for the freeze
cargo test --lib close_does_not_wait_on_a_decoder_stuck_in_read -- --nocapture

# GREEN baselines for audio
cargo test --lib a_prime_with_no_buffer_stops_waiting_after_a_deadline -- --nocapture
cargo test --lib an_empty_ended_buffer_does_not_start_a_silent_clock -- --nocapture
cargo test --lib only_end_of_stream_is_terminal_for_audio -- --nocapture

# GREEN baselines for A11 fallback retry (device-free: a missing file fails in the
# media decoder before any output device is opened)
cargo test --lib audio::tests

# Regressions
cargo test --lib
cargo test --test observability
cargo test --test pacing
cargo test --test frame_matrix
cargo test --test first_play
cargo test --test widget_state
```

Evidence already captured:

- `close_does_not_wait_on_a_decoder_stuck_in_read` was **RED before** A1 (`close blocked on a decode thread stuck in read`) and **GREEN after** A1 (`1 passed; 0 failed`, `TEST_RC=0`) `[REPORT-ONLY run output, current source verified]`.
- `a_prime_with_no_buffer_stops_waiting_after_a_deadline` passed with `TEST_RC=0` before the latest helper edit `[REPORT-ONLY]`.
- A full `cargo test --lib` after the earlier audio-flag fix reported `110 passed; 0 failed`, `TEST_RC=0`, before the final `start_ready` helper edit `[REPORT-ONLY]`.

**Final post-edit host verification (Windows, this tree, 2026-11-19).** These are host test/compiler results, not device-level audible proof. Exact commands and return codes from the run:

| Command (workdir) | Result | RC |
|---|---|---|
| `cargo test --lib` (`cranpose-video`) | 137 passed; 0 failed; 0 ignored | 0 |
| `cargo test --lib hls` (`cranpose-video`) | 49 passed; 0 failed (HLS offline corpus, includes the startup no-panic regression) | 0 |
| `cargo test --lib audio::tests` (`cranpose-video`) | 33 passed; 0 failed (A11 retry tests included) | 0 |
| `cargo test --lib session` (`cranpose-video`) | 8 passed; 0 failed | 0 |
| `cargo test --lib diagnostics` (`cranpose-video`) | 3 passed; 0 failed | 0 |
| `cargo test --lib recovery` (`cranpose-video`) | 2 passed; 0 failed | 0 |
| `cargo test --test observability` | 7 passed; 0 failed | 0 |
| `cargo test --test pacing` | 5 passed; 0 failed; 0 ignored (17.70 s) | 0 |
| `cargo test --test frame_matrix` | 3 passed; 0 failed | 0 |
| `cargo test --test first_play` | 1 passed; 0 failed | 0 |
| `cargo test --test widget_state` | 2 passed; 0 failed | 0 |
| `cargo test --test sources` | 6 passed; 0 failed | 0 |
| `cargo test --test playback` | 10 passed; 0 failed | 0 |
| `cargo check --lib` (`cranpose-video`) | `Finished dev`, no warnings | 0 |
| `cargo check` (`D:\counter` root, no `--all-targets`) | `Finished dev`, no warnings | 0 |
| `rustc --edition 2021 --test src/app_model.rs` (`D:\counter`) | 8 passed; 0 failed | 0 |
| `rustc --edition 2021 --test src/app_diag.rs` (`D:\counter`) | 2 passed; 0 failed | 0 |

The root-app `cargo check` initially reported one `unused variable: scope` warning at `src/app_ui\reels.rs:132:41` (the resolution pump's `LaunchedEffect` closure does not use `scope`, unlike the frame pump which uses `scope.with_frame_nanos`); it was silenced by renaming the parameter to `_scope`, and the re-run is warning-free. No gate, public signature, or threshold was changed.

**Startup-crash regression found and fixed after the table above was first captured.** The user reported `thread 'main' panicked at cranpose-video\src\hls\mod.rs:342:14: a minimal empty playlist parses: Source("the HLS playlist names no segments")` with the app collapsing at startup/HLS compose and no sound. Root cause `[SOURCE-VERIFIED]`: `Stream::new()` parsed a fake segment-less `#EXTM3U … #EXT-X-ENDLIST` placeholder on the composition thread, but `playlist::media()` rejects a playlist with no segments (`playlist.rs:485-487`), so `.expect(...)` panicked before the decode thread could load the real playlist. Fix `[SOURCE-VERIFIED]`: added the additive `playlist::Media::empty()` (`playlist.rs:99-108`) and used it directly in `Stream::new()` (`hls/mod.rs:338-397`); no parsing and no panic path on startup. A regression test `opening_an_hls_address_never_panics_before_the_playlist_is_loaded` (`hls/mod.rs:2052`) covers it. Post-fix capture (Windows host, this tree):

| Command / observation | Result | RC |
|---|---|---|
| `cargo test --lib` (`cranpose-video`) | 137 passed; 0 failed | 0 |
| `cargo test --lib hls` (`cranpose-video`) | 49 passed; 0 failed | 0 |
| `rustfmt --edition 2024 src/hls/mod.rs src/hls/playlist.rs` | formatted, no diagnostics | 0 |
| `cargo build` (`D:\counter` root) | `Finished dev` | 0 |
| Launch `counter.exe`, idle smoke observation | process stayed alive >15 s; `reels.open … bunny.mp4: player ready`; no `[PANIC]` in the current run | n/a |

`D:\counter\diagnostics.log` still holds the two pre-fix crash records (pids 13332, 10628 at `hls/mod.rs:342`) alongside the current no-panic startup record (pid 5456); the smoke process was then ended with `taskkill /PID 5456 /F /T` (exit 1 from the kill, not a panic). A plain full `cargo test` (including doc tests) was attempted twice after the fix but both runs were cancelled by environment/server restarts, so it is **not** claimed as a post-fix pass here; the two `--lib` rows above are the captured post-fix evidence. Audible recovery on a device is still unmeasured (below).

- Device-level audible proof after rapid real Reels switching: `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]`.

False positives to reject:

- RED passing because `StuckDecoder` was changed to observe shutdown — that would mask the contract.
- Prime test passing because `waiting` was already false — the precondition asserts it is true.
- A pacing test passing after threshold edits — thresholds are fixed and must not move.
- Treating probe cadence as display-level SLO; `examples/stutter_probe.rs` explicitly measures processing-side publication, not the compositor.

Platform limits still blocking end-to-end proof `[REPORT-ONLY]`:

- Windows host app fails device creation with `max_compute_workgroups_per_dimension: requested 65535, allowed 0`.
- GLES shader pipeline compile deadlock on Intel HD 4000 / NVIDIA NVS 5200M remains report-only.
- WSL was fully removed; no Linux runtime test exists. Do not fill Linux measured slots.
- Android NDK is absent; iOS device unverified.

---

## 9. Final acceptance checklist

- [x] No commit, push, PR, or merge.
- [x] No public API signature/enum/struct-field change.
- [x] No gate relaxation.
- [x] No arbitrary resolution cap.
- [x] No pitch/time-stretch/resampling path added.
- [x] Close no longer waits on an uncooperative read.
- [x] `Closed` is sealed as the last event.
- [x] Only true EOF ends an audio span; empty buffer cannot start a silent clock.
- [x] Stale picture is cleared on source change.
- [x] A failed fallback audio attach is retried under a throttle (A11); a ready or cooling source is a no-op; no per-frame prepare for no-audio sources.
- [x] Adversarial blockers triaged: exit-token/deferred-open handshake rejected, decoder reclaim-from-close rejected, timeout/reaper rejected, root wgpu fix reverted (§2.2).
- [ ] Device-level audible proof after rapid real Reels switching: `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]`.
- [ ] One-decoder-at-a-time measured across framework cancellation: `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]` (overlap verified at `hooks.rs:41-42`; handshake fix rejected).
- [x] Additive playback-session/recovery layer on every native target (§10); no Linux change.
- [x] Explicit HLS open entry point; routing no longer depends on a `.m3u8` suffix (§10.4).
- [x] Startup/HLS compose no longer panics on an unloaded segment-less placeholder (§8 crash-fix regression).
- [x] Diagnostics sink with redaction of user info and query/fragment material (§10.5).

**Signoff status.** All source-verifiable items are addressed and triaged. There is deliberately **no complete/final signoff**: the two device-level rows above are unmeasured because WSL was removed and no Linux/Reels device run exists (§8). This document reports source-verified hardening and reasoned estimates only.

---

## 10. Second wave — playback sessions and cross-platform recovery

The first wave (A1–A11) fixed the shutdown/EOF/latch failures. This wave answers the remaining user asks: *"make the audio return immediately on any switch instead of failing completely, and handle errors intelligently across all the named systems"* and *"review the library's network handling — do network errors break the player, and is the URL still handed to it"*. It is **additive**: a new public `PlaybackSession` type, new **defaulted** decoder hooks, and a new `VideoPlayer::open_hls` entry point. No existing public signature, enum variant, or public struct field changed.

### 10.1 Session model — `src/session.rs` `[SOURCE-VERIFIED]`

| Property | Anchor | Mechanism |
|---|---|---|
| Process-local, not an OS media session | `session.rs:7-15` | The library records player state; it does not claim to own the platform's media key. |
| No terminal failure | `session.rs:31-42` | Repeated failures back off 400 ms → 4 s geometrically (`RECOVERY_COOLDOWN`, `RECOVERY_COOLDOWN_MAX`) and stay at the bound; an open player always has another attempt. |
| Bounded 64-event ring | `session.rs:44-46`, `:390-405` | A device that fails repeatedly cannot grow memory without bound. |
| `Closed` is last | `session.rs:269-292`, `:348-364`, `:390-400` | `failed()`/`interrupted()`/`route_changed()`/`begin_recovery()`/`recovered()` all return early once `Closed`; `close()` sets an atomic seal before appending `Closed`, and `push()` drops any late non-`Closed` event from a detached worker. Covered by `a_late_push_after_close_is_sealed`. |
| Redaction | `session.rs:269-272` | `failed()` passes every reason through `redact_for_report()` before storing or exposing it. |
| No polling when idle | `session.rs:204`, `player/threaded.rs:193-212` | `next_retry()` is `None` unless a retry is armed; the worker blocks on `recv()` and switches to `recv_timeout(≤50 ms)` only while a retry is pending. No shutdown timeout or reaper. |

**Additive decoder hooks** (`decoder.rs`, defaulted so existing decoders are unaffected): `carries_audio` (`:174`), `observe_shutdown` (`:202`), `attach_session` (`:212`), `recover` (`:222`). A private audio helper `needs_attach(attached, ready)` (`audio.rs:516`) lets a failed route rebind retry while `attached=true`. The worker calls `maybe_recover()` on its own thread (`player/threaded.rs:231-254`), so no device enumeration, route query, or stream rebuild runs on the composition/UI thread. A source failure is redacted before it is stored in the public state/event history (`error.rs:59-69`, `player.rs:491-508`).

### 10.2 Frame-gated recovery — `src/player.rs`, `src/player/threaded.rs` `[SOURCE-VERIFIED]`

`decoder.recover() == true` is **not** treated as proof that playback resumed. On success the worker only sets `refresh` (`threaded.rs:231-245`); the public `VideoState::Failed` is cleared and a `Playing` event emitted only in `resume_after_recovery` (`player.rs:305-314`), which `note_frame()` calls after a real picture is published (`player.rs:547-563`). A live source that rebuilds to an empty window re-arms the bounded retry instead of ending the feed (`threaded.rs:331-341`, test `a_live_recovery_without_a_picture_rearms_instead_of_ending` at `:1120`). The transient-decode test (`threaded.rs:1022`) verifies `Failed → Recovering → frame → Playing`.

### 10.3 Per-target recovery — `[SOURCE-VERIFIED]` source, `[HYPOTHESIS]` audible outcome

- **Windows** (`platform/windows.rs`, `platform/windows/mf_audio.rs`): MF audio setup is deferred from UI-thread `open()` to the decode thread's first `read()`, then retried through `Reader::recover()`; one source reader owns both tracks, exact `(channels, sample_rate, sample_format)` match is required for route rebind, a self-retiring `MirrorClock::waiting` prime with a 500 ms wall-clock deadline, and only true `MF_SOURCE_READERF_ENDOFSTREAM` latches `ended`. COM is kept alive by the first caller's apartment (`Apartment::join_for_process()`). No pitch, time-stretch, resampling, or route query on the UI thread.
- **Apple** (`platform/apple.rs`): video-only backend; `wrap_direct` is gated by `carries_audio()` (default `false`). Stream liveness uses a bounded unusable-sample counter (`MAX_UNUSABLE_SAMPLES=8`, reset per `pull()`), no wall-clock timeout, and checks shutdown between samples. Recovery/shutdown hooks report into the session.
- **Android** (`platform/android.rs`): lazy JNI/MediaCodec setup on the decode thread (`open()` at `:90` only clones the source; `with_vm()` at `:991`, `setup_media()` at `:1008`, `ensure_ready()` at `:257`, codec `start` at `:150`); `observe_shutdown` (`:1308`), `attach_session` (`:1313`), and `recover` (`:1318`) are wired, and setup failures are redacted and reported once per recovery generation (`:323`, `session.failed("android.recover", …)`). Audio-focus loss is **not** wired: `InterruptionReason::AudioFocusLoss` (`session.rs:67`) is defined but has no caller in this tree, so no audible-focus-loss recovery is claimed here. Audible device proof is absent (no NDK/device), so the outcome stays `[HYPOTHESIS]`.
- **Generic audio** (`audio.rs`): a failed `prepare()` is reported into the session once per source and stays unattached; the worker re-arms it via failed `recover()`. The 500 ms attach/route/rebind throttles remain; `Audio::retry()` bypasses the throttle only for a scheduled worker recovery. Audio failures never break video.

### 10.4 Network routing — `[SOURCE-VERIFIED]`

- `VideoPlayer::open_hls(...)` (`player.rs:808-844`) explicitly selects the HLS reader for a known playlist, skips the direct-audio wrapper (HLS carries its sound inside the joined media), and is cfg-gated to exactly the targets where the `hls` module exists (`lib.rs:10-17`). It is additive; on targets without HLS it delegates to `open`.
- Reels selects it for `ClipKind::Hls` (`D:\counter\src\app_ui\reels.rs:87-98`); shipped and future HLS URLs no longer depend on a `.m3u8` suffix.
- `VideoSource::is_hls()` (`source.rs`) remains a **fallback** only: it inspects the URL path (not query/fragment), decodes percent escapes, and is covered by `tests/sources.rs` (`hls_classification_uses_the_url_path_not_the_query_or_fragment`). All 42 shipped `LIVE_CHANNELS` still end in `.m3u8` `[SOURCE-VERIFIED]`, but explicit routing removes the fragility for arbitrary URLs.
- Network/live sources remain opt-in and outside the offline `decode_cases()` corpus; Reels and Live are separate tabs with one decoder open at a time.

### 10.5 Observability — `src/diagnostics.rs`, `D:\counter\src\app_diag.rs` `[SOURCE-VERIFIED]`

A process-local diagnostic sink writes `diagnostics.log` in the cwd (2 MiB rotation to `.old`, path printed to stderr). `diagnostics::diag()`, `PlaybackSession::failed()`, and the app logger all call `redact_for_report()`, which strips user-info and query/fragment/token material. Tests in `app_diag.rs` and `session.rs` assert no secret survives. No number is invented; runtime event counts remain `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]`.

### 10.6 Evidence and residuals for this wave

- Device-free session ordering, `Closed`-last, failed-prepare dedupe, recovery-only-after-frame, and live empty-window re-arm tests pass `[SOURCE-VERIFIED by unit test]`.
- No device-level audible proof of return-after-switch exists: `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]`.
- Blocked, not fixed here: Windows host device creation (`max_compute_workgroups_per_dimension 65535/0`) `[REPORT-ONLY]`, missing Android NDK and iOS clang toolchain, no Linux runtime (WSL removed), user HLS URL returned HTTP 403, upstream `cranpose-audio`/`cranpose-media` are pinned registry crates and cannot be patched in this workspace.

The second-wave verification table is filled from the exact command/RC capture described in §8.

### 10.7 HLS network path — `src/hls/**` `[SOURCE-VERIFIED]` source, `[HYPOTHESIS]` runtime outcome

This section answers the explicit ask: *do network errors break the player, and is the URL/data still handed to the decoder when the network fails?*

All anchors below are **post-format** (the crate is edition 2024, `Cargo.toml:4`; `rustfmt --edition 2024 --check src/hls/{mod,playlist,tfdt}.rs` RC=0 — the 2021 formatter disagrees only on import ordering).

| Property | Anchor | Mechanism |
|---|---|---|
| Lazy network open | `hls/mod.rs:121-128`, `:338-397`, `:405` | `open()` only stores the URL via `Stream::new`; `new()` builds a literal empty playlist with `loaded = false` via `playlist::Media::empty()` (`playlist.rs:99-108`), never parsing (no startup panic); the first playlist/fragment/window fetch happens in `ensure_loaded()`, called only from `seek()` (`:1165`) and `next_picture()` (`:1316`) on the decode thread. `info()` (`:1075`) and `output_format()` (`:1212`) stay network-free. |
| Cooperative cancellation, no timeout/reaper | `hls/mod.rs:1461`, `:1492`, `:1230`, `:1245` | A one-way `Stop { local, external }` is threaded through prefetch, fetch, retries and body reads; `close()` signals it first, `observe_shutdown()` adopts the player flag, and `sleep_or_stop()` wakes waits immediately. Cancellation is permanent. A single-request `media_deadline()` (`:1446`) bounds an HTTP hang; there is no stuck-decode shutdown timeout or reaper. |
| Session integration | `hls/mod.rs:1250`, `:1262`, `:1295`, `:1307` | `attach_session` stores the player session; `recover()` clears `loaded`, closes the window, resets init/audio/prefetch and returns `true` (HLS never calls `session.recovered()` itself); `report_failure()` dedupes by `session.generation()` and calls `session.failed("hls.open"\|"hls.window", redact)`. Throttling is owned by `session.rs`, not duplicated here. |
| Video network failure | `hls/mod.rs:1316-1339`, `:1165-1170`, `:344` | A failed playlist or window fetch is reported and returned as `Err(VideoError::Source)` to the worker; public state stays `Failed` until a real post-recovery frame is published (§10.2). The URL string stays in the `Stream` and a later `recover()` reloads it. The platform decoder receives joined bytes through a `TempFile` window, **not** the HLS URL itself. Transient failures are retried with an attempt/`Retry-After` cap; permanent failures are not. |
| Audio network failure | `hls/mod.rs:786-816` | An audio playlist/init fetch failure returns `Ok(None)` rather than `Err`, so a broken audio rendition never breaks the video path; audio is paired from local `TempFile` URIs and the generic audio backend reports its own failures into the session. |
| Content type + redaction | `hls/playlist.rs:151-209`, `hls/mod.rs:1600`, `:1419`, `:1797`, `:1740`, `:1894`, `:1908`, `:1952` | Each request declares a `ResourceKind`; `accepts_content_type()` (`playlist.rs:176`) accepts a missing/empty header but refuses HTML/JSON where media is expected before any body read, independent of the `.m3u8` suffix. Refusal text names only the kind; every fetch error uses `safe_url = redact_for_report(url)`. All HLS `diag` targets (`:1277`, `:1715`, `:1798`) are address-free. |

**Net answer to the user's question:** a network error is classified and reported into `PlaybackSession`; it does **not** silently tear the player down, but it is also **not** transparently survived by video — a failed playlist/window read returns `Err` and the public state stays `Failed` until the worker's bounded retry or a later `recover()` publishes a real frame. Audio-side fetch failures are non-fatal to video by design. The decoder is handed *media bytes*, never a half-fetched URL; the only URL that crosses the boundary is a **redacted** address inside `VideoError::Source`. Source placement/classification are `[SOURCE-VERIFIED]`; actual return-to-sound on a live failure is `[HYPOTHESIS]` — all 49 HLS offline tests avoid live network.

HLS tests added: `a_content_type_matches_the_resource_it_names` (`playlist.rs:813`), `a_truncated_tfdt_body_has_no_base` (`tfdt.rs:227`), `opening_an_hls_address_never_panics_before_the_playlist_is_loaded` (`mod.rs:2052`), `a_permanent_failure_is_not_retried` (`mod.rs:2296`), `a_transient_failure_retries_until_its_attempt_limit` (`:2306`), `a_close_cancels_a_retry_before_it_waits` (`:2324`), `a_direct_close_signals_the_players_flag` (`:2333`), `an_loaded_stream_does_not_touch_the_network_again` (`:2345`) `[SOURCE-VERIFIED by unit test — `cargo test --lib hls` RC=0, 49 passed, 0 failed]`.

**Out-of-scope residual (reported, not edited here):** `src/source.rs:61-74` still classifies HLS by `.m3u8` suffix in `is_hls()`, so a playlist URL without that suffix would not reach this path through plain `open()`. This is mitigated by the explicit `VideoPlayer::open_hls(...)` entry point (§10.4) used for `ClipKind::Hls`, which skips the suffix check; the fallback remains documented as a fallback (§10.4).
