# Android playback test checklist

> **Status: PENDING DEVICE RUN.** Local verification is limited to source review,
> unit tests and `cargo check --target aarch64-linux-android`; no Android device
> was available on this host (NDK 29.0.13846066 is missing). Nothing below has
> been marked as passed. The performance thresholds and measurements in
> `docs/final-report-ar.md` are Windows host results and do not substitute for
> these device checks.

These checks target the MediaCodec/JNI fixes in `src/platform/android.rs` that
were verified by type-check but need a real device at runtime. Build the app
with this crate and exercise each case with `logcat` attached.

```
adb logcat | grep -E "cranpose|BufferInfo|MediaCodec|the decoder|cannot read"
```

The strings below are this crate's own `VideoError::Decode` messages, so a grep
hit for one is a concrete failure signal, not noise.

## 1. BufferInfo fields (the stub bug)

Before the fix, `flags`, `size` and `presentationTimeUs` were read with
`getInteger`/`getLong` method calls. `MediaCodec.BufferInfo` has **public fields**,
so those calls threw, the values were silently replaced with `0`, no frame was
ever copied, and `read_picture()` span until the 30 s empty deadline.

- [ ] Open a local MP4 (`assets/videos/bunny.mp4`). The **first frame appears in
      well under a second**, not after a ~30 s pause.
- [ ] Picture time advances smoothly; seeking lands on the expected frame
      (proves `presentationTimeUs` is a real PTS, not `0`).
- [ ] No `cannot read the picture's flags` / `...size` / `...time` in logcat.

## 2. Output buffer always released

A conversion error used to leave the output index unreleased, exhausting the
codec and stalling every later dequeue until the deadline.

- [ ] Play a stream that forces a format change mid-playback (resolution switch,
      e.g. an ABR HLS variant). Decoding continues afterwards.
- [ ] Play a short/clipped or malformed source. No `the decoder stopped` loop,
      no multi-second freeze at the point of the bad frame.
- [ ] No `the decoder kept its picture` in logcat.

## 3. Optional MediaFormat lookups clear exceptions

Absent optional `MediaFormat` keys throw in Java as a matter of course. Those
exceptions are now cleared immediately so they cannot poison the next JNI call.

- [ ] Open several different codecs in one session (H.264, HEVC, VP9, AV1 if the
      device supports them). All initialise without a crash or a stale-exception
      abort on the first frame.
- [ ] No `Fatal signal` / `JNI DETECTED ERROR` / `pending exception` in logcat.

## 4. Input buffer leak on failure (feed refactor)

Sample reading and growth now happen **before** `dequeueInputBuffer`, and any
preparation failure returns the index empty via `queueInputBuffer`. Previously a
failure leaked the index and every later feed stalled.

- [ ] Seek rapidly and repeatedly (scrub back and forth) for ~30 s. Playback
      keeps up; no growing lag or late freeze (would indicate leaked indices).
- [ ] Feed a sample larger than the initial buffer so the growth path runs.
      Decoding continues normally.
- [ ] Play a truncated/corrupt tail. The decoder reaches EOS cleanly instead of
      hanging, with no `the decoder takes no sample` / `the decoder refused the
      sample` spam.

## 5. Long-run stability (leak / ref pressure)

- [ ] Loop a 10-minute video (or leave a live HLS stream running) for 15+ min.
      Frame rate does not degrade over time.
- [ ] Memory use stays roughly flat (watch `dumpsys meminfo <pkg>`), no steady
      climb that would indicate an unreleased buffer or local-ref buildup.

## Reporting

For any failure, capture `adb logcat -d` from just before the case plus the exact
media URL/file and device model + Android version. The fixes are all in
`src/platform/android.rs`; a logcat line naming one of the strings above tells
us exactly which path to revisit.
