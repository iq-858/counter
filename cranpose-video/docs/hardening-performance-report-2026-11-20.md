# تقرير التدقيق الشامل والأداء بعد الإصلاح — `cranpose-video`

> تاريخ الجلسة: 2026-11-20. كل التعديلات **غير مُودَعة** (no commit / stage / push / PR / merge).
>
> ## معنى الوسوم (تُطبَّق على كل سطر أدناه)
>
> - **`[SOURCE-VERIFIED]`** — حقيقة قرأناها مباشرةً من ملف مصدر أو وثيقة، مع رقم سطر أو رابط، دون قياس على جهاز.
> - **`[VERIFIED]`** — سلوك أكّدناه بتشغيل اختبار فعلي على هذا المضيف (Windows، 4 أنوية منطقية).
> - **`[REPORT-ONLY]`** — قول محفوظ من جلسة/سجل سابق لم نعد نملك سجله الخام؛ لا يُعدّ دليلاً قابلاً لإعادة الإنتاج.
> - **`[HYPOTHESIS]`** — استنتاج وقت تشغيل/جهاز من المصدر لم يُقَس بعد.
> - **`[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]`** — خانة محجوزة للإثبات على جهاز/منصة لم نصلها؛ لا تُكتب فيها أرقام مختلقة.
>
> القياسات أدناه أُعيد تشغيلها **بعد** إصلاح نواة البكسل SIMD، على جهاز واحد:
> `Intel HD Graphics 4000 + NVIDIA NVS 5200M`، `1366×768 @ 59 Hz`، 4 أنوية منطقية.
> المقاطع: `bunny.mp4` 853×480@25، `clip1080p60.mp4` 20s@1080p60، `b720p25.mp4` / `b1080p30a.mp4` 6s.

---

## 1. البحث عبر مكتبات أخرى + ما استُنسخ وما رُفض

الهدف لم يكن «نقل كود» بل فهم آليات التحكم في التفريغ/الانتظار/الربط، ثم إعادة إنتاج المفيد داخل حدود واجهة `cranpose-video` الحالية.

| المصدر | الحقيقة المقراءة | الوسم |
| --- | --- | --- |
| `gstreamer-rs` README (raw GitHub) | ربط آمن يُولَّد بأداة `gir`؛ يتطلب GStreamer `>=1.14`؛ دورة الإصدار مرتبطة بـ GTK/GNOME. | `[SOURCE-VERIFIED]` |
| `rust-ffmpeg` / `ffmpeg-next` README (raw GitHub) | وضع الصيانة (maintenance mode)؛ يستهدف FFmpeg `3.4`–`8.0`؛ تعيين إصدارات غير قياسي. | `[SOURCE-VERIFIED]` |
| GStreamer `queue` docs (raw GitHub) | `queue` يحجب الدفع عند `max-size-buffers` / `max-size-bytes` / `max-size-time`، ويُنشئ خيطاً جديداً على الـ source pad. | `[SOURCE-VERIFIED]` |
| FFmpeg codec docs (raw GitHub) | فصل فك التشفير عن العرض عبر حدود/طوابير مدعومة بالـ codec، لا عبر «تسريع التشغيل». | `[SOURCE-VERIFIED]` |
| `docs/upstream-decoder-comparison.md:1-56` | upstream `cranpose-services` (commit `3e53a1b8`) يفصل feed/drain، طوابير محدودة، انتظار بالـ waker، ومسارات zero-copy (Surface/IOSurface/AHardwareBuffer)، بلا سقف ثابت للدقة. | `[SOURCE-VERIFIED]` |

**ما أُعيد إنتاجه فعلياً هنا** — كل نقطة بمرجع مصدر:

- طابور محدود/إشارة بدل الالتفاف: الخيط ينتظر في `inbox.recv()`، و`Command::Shutdown` يُرسل قبل أي محاولة `join()` (`src/player/threaded.rs:116-120`). `[SOURCE-VERIFIED]`
- انتظار نوم بدل spin طوال الإطار: `SPIN_TAIL = 300µs`، والبقية تُنام (`src/time_wait.rs:21,112-114`). `[VERIFIED]` — الاختبار `time_wait::tests::the_tail_is_spun_but_the_bulk_is_slept` يمر.
- فصل shutting-down عن الذيل: `shutdown` يُشار إليه بـ `AtomicBool`، والخيط يُفصل (detach) فلا يحجب `close()` حتى لو كان داخل `read()` (`src/player/threaded.rs:120`, `tests/shutdown_blocked.rs`). `[VERIFIED]`
- حدود panic حول `read`/`seek`/`close`: `catch_unwind` في `Worker::run` يحوّل الذعر إلى `VideoError::Decode` ويُبقي `Closed` آخر حالة (`src/player/threaded.rs:167-179`, `tests/panic_boundary.rs`). `[VERIFIED]`
- لا zero-copy هنا: المسار يبقى NV12→RGBA؛ لم نضف سطحاً أصلياً لأن الواجهة العامة لا تعرضه. `[SOURCE-VERIFIED]`

**ما رُفض صراحةً (وسببه)** — `[REPORT-ONLY]` للقرار، `[SOURCE-VERIFIED]` للموقع إن وُجد:

- `3→1` تقليل المخازن: قياسات متكررة أظهرت أن تقليل المخزن لا يمنح فائدة بدء؛ مرفوض.
- سقف دقة ثابت / فرض fan-out عند 1080p: القياس لا يدعمه (انظر §4)؛ `PARALLEL_MIN_PIXELS` يبقى `3840*2160` (`src/pixels.rs:33`).
- استرداد المُفكِّك من `close()` (UAF/double-drop)، ومخطط exit-token/تأخير الفتح (`hooks.rs:41-42`)، ومهلة إغلاق تعسفية/reaper: كلها مرفوضة.
- نقل `with_gpu_requirements` من جذر wgpu: خارج نطاق المكتبة.

---

## 2. تدقيق دورة حياة كل دالة/خيط (متى يتوقف، وماذا يفعل عند الفشل)

| الخيط/المكوّن | بداية | توقف طبيعي | عند الفشل/الذعر | الوسم |
| --- | --- | --- | --- | --- |
| decode worker (`cranpose-video`) | `VideoPlayer::open` | `Command::Shutdown` + detach؛ `Closed` آخر حالة | `catch_unwind` → `VideoError::Decode("the decoder panicked")` + log `decode.panic` + `decoder.close()` تحت `catch_unwind` ثانٍ (`src/player/threaded.rs:167-179`) | `[VERIFIED]` |
| cpal device callback | أول `read()` يهيّئ الصوت (مؤجَّل، على خيط الفك لا خيط الواجهة) | إغلاق الجهاز | أخطاء reader قابلة للاستئناف؛ خطأ `ENDOFSTREAM` وحده يُلغي الspan (`mf_audio.rs:1439-1455`) | `[SOURCE-VERIFIED]` |
| `cranpose-video-retire` (مرة لكل rebind) | عند إبطال مسار صوتي | يُسقط الـ stream فوراً | لا يؤثر على خيط الفك؛ يمنع WASAPI teardown من تعطيله (`mf_audio.rs:982-989`) | `[SOURCE-VERIFIED]` |
| `hls-prefetch` (لكل نافذة) | عند فتح نافذة HLS | أفضل جهد + detach | تحقّق الـ generation يمنع كتابة نافذة قديمة (`hls/mod.rs:742-778`)؛ الفتح كسول، `Stop` تعاوني | `[SOURCE-VERIFIED]` |
| `fetch_many` (scope) | جلب مقاطع متعددة | نهاية النطاق | فشل رابط صوتي منفصل يُعتبر best-effort ولا يكسر الفيديو (`hls/mod.rs:618`) | `[SOURCE-VERIFIED]` |

**سلوك مع فيديوهات مستلمة عشوائياً / أخطاء مكتبة غير متوقعة:**

- بايتات عشوائية/MP4 فارغ: اختبار جديد `tests/malformed_source.rs` ي opening إما يُرفض مباشرة (`Source`/`Decode`/`NoVideoTrack`) أو يستقر إلى `Failed`/`Ended`/`Paused` within 6s، و`close()` خلال <3s، و`is_closed()` صحيح. `[VERIFIED]` — `2 passed; 0 failed`.
- COM poisoning على Windows: `Apartment::join_for_process()` (`src/platform/windows.rs:80,1375`). `[SOURCE-VERIFIED]`
- Apple: فيديو فقط، `wrap_direct` محكوم بـ `carries_audio()` افتراضياً `false`، والـ liveness محدود بـ `MAX_UNUSABLE_SAMPLES=8` بلا مهلة زمنية (`src/platform/apple.rs:51-52,474`). `[SOURCE-VERIFIED]`
- Android: `StopFlag` + `close` + `recover` (`src/platform/android.rs:113-149`). `[SOURCE-VERIFIED]`
- Linux: **لا تحليل ولا تغيير ولا تطوير** وفق القيد. `[REPORT-ONLY]`

---

## 3. تغطية الفشل: ما أُغلق هذه الجولة تحديداً

كانت طوابير HLS (~49 اختباراً) واختبارات الصوت (head/resync/seek/rebind) موجودة سلفاً. أُضيف **فقط** ما كان غائباً فعلاً:

1. **403/410 دائمة وليست قابلة لإعادة المحاولة** — تأكيد صريح في `transient_status` (`src/hls/mod.rs:1987`, الاختبار في نفس الملف): الرابط المنتهي/المحذوف يُعاد حلّه لا يُعاد طلبه. `[VERIFIED]`
2. **بايتات عشوائية/MP4 فارغ أمام المُفكِّك الحقيقي** — `tests/malformed_source.rs` (جديد). `[VERIFIED]`

ولم تتكرر اختبارات PCM الافتراضية ولا «فشل الصوت لا يكسر الفيديو» لأنها مغطاة أصلاً في `audio.rs` و`mf_audio.rs`. `[SOURCE-VERIFIED]`

**تحقّق شامل:** `cargo test --release --no-default-features --features native-audio --lib` → `146` اختباراً، `145 passed; 0 failed; 1 ignored`. `[VERIFIED]`

**قيود معروفة (لا تُخفى):**

- `[REPORT-ONLY]` اختبارات debug على Windows تفشل بـ `link.exe LNK1318 Unexpected PDB error; OK (0)`؛ release يعمل.
- `[REPORT-ONLY]` `websearch` يعيد HTTP 402؛ استُخدم raw GitHub `webfetch` كبديل.
- `[REPORT-ONLY]` `cargo check --all-targets` في الجذر يفشل في `examples/probe.rs` (`use mp4::Mp4Reader`) — خطأ سابق في مستودع الجذر، ليس في هذه المكتبة.
- `[HYPOTHESIS]` `cranpose-audio`/`cranpose-media` تبعيات registry مثبّتة (غير vendored)؛ لا يمكن إصلاح عيوبها من هنا.
- `[REPORT-ONLY]` `InterruptionReason::AudioFocusLoss` (`session.rs:67`) بلا مُستدعٍ؛ لا يُ claiming استعادة فقدان تركيز صوتي.
- `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]` إثبات audible على جهاز + «مُفكِّك واحد فقط في كل مرة» عبر إلغاء الإطار (framework cancellation). التداخل عند `hooks.rs:41-42` مرفوض بالتصميم ولم يُصلَح.

---

## 4. الأداء: قبل/بعد إصلاح SIMD، مع تحسّن/تراجع صريح

القيم «السابقة» من جلسات سابقة `[REPORT-ONLY]`؛ القيم «الآن» مقيسة هذه الجولة `[VERIFIED]`.when الجولة الواحدة ضجيجها واضح، لا يُ claim تحسّن إلا عند فارق متسق.

### 4.1 البكسل/SIMD — الإصلاح الجوهري

- `[SOURCE-VERIFIED]` العيبان في `convert_rows_sse41`/`convert_rows_tuned`: (1) V كان يُقرأ من بايتات الـ shuffle العليا الصفرية عبر `_mm_unpackhi_epi8(uv, zero)` — أُصلح بـ `_mm_unpacklo_epi8` ثم `_mm_unpackhi_epi16`/`_mm_unpacklo_epi16`؛ (2) مجاميع Q16 كانت تُحزم قبل `>>16` فتُقرّب 254→255 — أُصلح بـ `_mm_srai_epi32(...,16)` قبل الحزم. مع U/V محايدين كان الأخضر يتحول إلى `[0,255,0,255]`؛ مع U=90/V=240 كان R=254→255.
- `[VERIFIED]` اختبار انحدار `pixels::tests::the_vector_kernel_matches_the_scalar_oracle_for_non_neutral_chroma` (Bt601-limited و Bt709-full، U=90/V=240) يمر؛ كل اختبارات البكسل خضراء.

| `pixel_bench` | السابق `[REPORT-ONLY]` | الآن `[VERIFIED]` | الحكم |
| --- | --- | --- | --- |
| 720p NV12→RGBA | 0.90 ms | 1.08 ms | ضمن الضجيج |
| 720p copy | 0.54 ms | 0.40 ms | تحسّن طفيف |
| 720p new buffer | 2.49 ms | 2.21 ms | تحسّن طفيف |
| 1080p NV12→RGBA | 2.16 ms | 2.41 ms | ضمن الضجيج |
| 1080p copy | 1.19 ms | 1.04 ms | تحسّن طفيف |
| 1080p new buffer | 5.21 ms | 4.78 ms | تحسّن طفيف |
| 4K NV12→RGBA | 5.79 ms | 5.12 ms | تحسّن طفيف |
| 4K copy | 5.50 ms | 4.13 ms | تحسّن |
| 4K new buffer | 17.78 ms | 16.55 ms | تحسّن طفيف |

**الحكم الصادق:** بعد تصحيح SIMD بقيت الأرقام في نفس نطاق الضجيج؛ **الصحة هي المكسب الحقيقي**، لا السرعة. لا يُستنتج تسريع من هذا الجدول.

### 4.2 زمن الانتظار (أكبر مكسب فعلي)

| `yield_until` | السابق `[REPORT-ONLY]` | الآن `[VERIFIED]` |
| --- | --- | --- |
| استهلاك المعالجة | حتى ~96% من نواة واحدة عند 60 fps (~48% عند 30 fps) لأن الإطار كاملاً كان يُستدار بـ spin | النوم يأخذ كامل المدة، والـ spin محدود بـ `SPIN_TAIL=300µs` (`src/time_wait.rs:21,112-114`) |

**تحسّن حقيقي ومتسق** في استهلاك المعالجة لكل إطار. `[VERIFIED]`

### 4.3 تشغيل حقيقي (`perf_probe play`)

| المقطع | السابق `[REPORT-ONLY]` | الآن `[VERIFIED]` | الحكم |
| --- | --- | --- | --- |
| bunny — first picture | 220.6 ms | 195.9 ms | تحسّن |
| bunny — fps / published | 24.36 / 195 | 24.49 / 196 | مكافئ |
| bunny — decode / convert | 831 / 451 µs | 807 / 512 µs | decode تحسّن، convert ضمن الضجيج |
| bunny — CPU machine | 3.1% | 2.9% | مكافئ |
| 1080p60 — first picture | 166.9 ms | 114.1 ms | تحسّن واضح |
| 1080p60 — fps / published | 59.10 / 591 | 59.40 / 594 | تحسّن طفيف |
| 1080p60 — decode / convert | 1452 / 2508 µs | 1284 / 2786 µs | decode تحسّن، convert تراجع طفيف (ضجيج) |
| 1080p60 — CPU one-core / machine | 129.5% / 32.4% | 128.1% / 32.0% | مكافئ |

`reanchors=1`, `errors=0` في الحالتين. `[VERIFIED]`

**الحكم:** تحسّن في زمن أول صورة وفك التشفير؛ استهلاك المعالجة عند 1080p60 لا يزال مرتفعاً (~1.28 نواة) — وهذا `[HYPOTHESIS]` يعود أساساً إلى تكلفة التحويل + العرض على معالج رسوميات ضعيف، لا إلى عدد الخيوط (الخيوط الخاملة محجوبة في `recv()`).

### 4.4 التبديل السريع (Reels)

| | السابق `[REPORT-ONLY]` | الآن `[VERIFIED]` |
| --- | --- | --- |
| `perf_probe reels 700 4` | switches=12 failures=0 pictures=208 seconds=8.43 | switches=12 failures=0 pictures=211 seconds=8.44 |
| أول صورة 720p | 53.7–111.3 ms | 45.8–109.8 ms |
| أول صورة 1080p30 | 130.6–193.2 ms | 115.5–174.8 ms |
| أول صورة bunny | 53.2–77.5 ms | 65.7–91.4 ms |
| `switch_mean_ms` | (مقتطع في السجل السابق) | 702.9 ms، worst 704.6 ms |
| CPU machine | غير مسجَّل | 13.7% |

`[VERIFIED]` `switch_probe`: 145–146 إطاراً، `open failures=0`، أبطأ تبديل ~504 ms، و`errors=4` في السجل هي تحذيرات `mf_audio.setup source has no selectable audio stream` (غير قاتلة؛ كل أسطر `player.close` تُظهر `errors=0`).

**الحكم:** تحسّن طفيف في 720p/1080p30، وتراجع طفيف في bunny — ضمن الضجيج. **لماذا ليس أسرع؟** `[HYPOTHESIS]` زمن التبديل (~700 ms) هو زمن الثبات المقصود (`hold_ms`) لا زمن الفتح؛ الفتح الفعلي مخفي خلف أول صورة (45–175 ms). إزالة الخيوط لا تُسرّع لأن الخيط الإضافي خامل ومحجوب.

### 4.5 الإجهاد والصوت/الفيديو معاً

| | السابق `[REPORT-ONLY]` | الآن `[VERIFIED]` |
| --- | --- | --- |
| `stress_probe bunny 4 3` | play 287 / park 0 / seek 12 / 0 errors / close 0.00s | play 290 / park 0 / seek 12 / total 309 / 0 errors / close 0.00s |
| `av_probe bunny 6` | frames=148, decode=880µs, convert=462µs | frames=147, decode=758µs, convert=528µs, state=Playing, errors=0, reanchors=1 |

`[VERIFIED]` الصوت: 2 قنوات @ 48000 Hz، أول كتلة 2048 عيّنة float عند 0ns؛ ربط المسار يتطلب تطابق PCM تماماً (`src/audio.rs:103,384`; `audio.rs:872-880`) فلا resampling ولا تغيير طبقة ولا تقطيع.

**الحكم:** مكافئ مع تحسّن في decode؛ `park=0` و`close=0.00s` يستمران.

### 4.6 قرار حد التوازي (fan-out)

| تشغيل | 720p | 1080p | 4K |
| --- | --- | --- | --- |
| A/B/C السابقة `[REPORT-ONLY]` | 1.20×/0.74×/0.98× | 1.14×/0.86×/1.14× | 1.41×/1.37×/1.23× |
| D الآن `[VERIFIED]` | 0.69× (fanout=false) | 1.64× (fanout=false) | 1.10× (fanout=true) |

**القرار:** الإبقاء على `PARALLEL_MIN_PIXELS = 3840*2160` (`src/pixels.rs:33`). `[SOURCE-VERIFIED]` عيّنة اليوم عند 1080p تُظهر 1.64× لكنها عيّنة واحدة ضجيجها كبير (min 1.445 مقابل sequential min 2.281) وتتناقض مع A/B؛ 4K يبقى متسقاً >1.0 عبر كل التشغيلات. `[HYPOTHESIS]` الحاجة إلى أخذ عينات أكثر قبل أي تغيير؛ لا سقف تعسفي ولا فرض fan-out عند 1080p.

---

## 5. بوابات التحقق النهائية

| البوابة | النتيجة | الوسم |
| --- | --- | --- |
| `cargo fmt --check` | نظيف (exit 0) بعد توحيد تنسيق أمثلة probes | `[VERIFIED]` |
| `cargo build --release --no-default-features --features native-audio --example ...` | Finished بنجاح | `[VERIFIED]` |
| lib suite release | `145 passed; 0 failed; 1 ignored` (من 146) | `[VERIFIED]` |
| `tests/panic_boundary.rs` | `3 passed; 0 failed` | `[VERIFIED]` |
| `tests/shutdown_blocked.rs` | `2 passed; 0 failed` | `[VERIFIED]` |
| `tests/malformed_source.rs` | `2 passed; 0 failed` | `[VERIFIED]` |
| `tests/pacing.rs` | `5 passed` | `[VERIFIED]` |
| `tests/first_play.rs` | `1 passed` | `[VERIFIED]` |
| `tests/observability.rs` | `7 passed` | `[VERIFIED]` |
| HLS unit tests | 49 اختباراً (منها 403/410 الجديد) | `[VERIFIED]` |
| `git status --short --branch` | `## No commits yet on master` — كل الملفات غير مُودَعة | `[VERIFIED]` |

---

## 6. ما ينتظر فعلياً (لا يُغلق بدون إثبات)

- `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]` إثبات audible على جهاز حقيقي (سماعات/headset switching) أن لا pitch/distortion/pulse/cut/stutter ولا time-stretch/resampling.
- `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]` إثبات «مُفكِّك واحد فقط» عبر إلغاء الإطار في Reels الفعلي (لا في `switch_probe`).
- `[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]` قياس 60 fps على Linux (محذوف بالكامل من هذا المضيف) وعلى جهاز Reels حقيقي.
- `[HYPOTHESIS]` تقليل استهلاك الـ 1080p60 (~1.28 نواة) يتطلب تحسين مسار العرض/التحويل لا تقليل الخيوط؛ يُقاس قبل أي قرار.
- لا توقيع نهائي (signoff) قبل معالجة هذه البنود أو waive صريح من صاحب القرار.
