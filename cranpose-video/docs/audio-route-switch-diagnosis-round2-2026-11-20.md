# تبديل الصوت بعد فصل السماعة — تشخيص الجولة الثانية وإصلاحها

> التاريخ: 2026-11-20 (الجولة الثانية بعد بلاغ المستخدم بأن الصوت يعود بعد دقائق لا لحظياً).
> المصادر: `D:\counter\diagnostics.log` (ذيل السجل الجديد) + قراءة cpal 0.18.2 + اختبارات فعلية.
> الوسوم: **`[SOURCE-VERIFIED]`** مقروء مباشرة؛ **`[HYPOTHESIS]`** يحتاج إثباتاً على جهازك.
> كل التعديلات غير مُودَعة.

## 1. الدليل الحاسم من السجل الجديد `[SOURCE-VERIFIED]`

```
20:13:18.989 [WARN] [mf_audio.rebind] default output config unavailable
20:13:23.370 .. 20:17:28.435  (كل ~20 ثانية، نافذة HLS جديدة)
   [WARN] [mf_audio.setup] default output config unavailable
   [WARN] [mf_audio.setup.deferred] ...
20:17:49.019 [INFO] [mf_audio.setup] audio track open: 2 channel(s) at 48000 Hz
```
أي أن `default_output_config()` ظل يفشل **من 20:13:23 إلى 20:17:28 — أكثر من 4 دقائق**، وكل مقطع/نافذة جديدة خلالها بلا صوت، ثم عاد الصوت فجأة. الفيديو وفك التشفير يعملان طوال الوقت (`frames` و`reanchors` سليمان). هذا يطابق وصفك بدقة: «الصوت يختفي على كل المقاطع ويعود بعد مدة طويلة لا في نفس اللحظة».

## 2. سبب الجولة الثانية: المقبض الافتراضي لـ cpal `[SOURCE-VERIFIED]`

- `cpal::default_output_device()` (WASAPI) يُرجع جهازاً من نوع `DeviceHandle::DefaultOutput`، أي **مقبضاً افتراضياً**، ويُفعّل `IAudioClient` عبر `DEVINTERFACE_AUDIO_RENDER` ثم **يُخزّنه في `future_audio_client` داخل مقبض الجهاز** (`cpal-0.18.2/src/host/wasapi/device.rs:559-597`).
- `default_output_config()` → `default_format()` → `GetMixFormat()` (`device.rs:818-828`, `:755-768`). بعد فصل السماعة يمكن لواجهة `DEVINTERFACE_AUDIO_RENDER` أن تُحلّ إلى نقطة النهاية الميتة لدقائق، فيفشل `GetMixFormat` بينما السماعة الداخلية مسموعة فعلياً.
- كان الكود يسأل **هذا المقبض وحده** (`default_output_device()` ثم `default_output_config()`) في `setup_checked` و`rebind`، فلا يوجد أي بديل حقيقي — ومن هنا الانتظار الطويل.

## 3. الإصلاحات الجديدة

| # | المكان | ما تغيّر |
|---|---|---|
| 1 | `mf_audio.rs:119-137` | دالة `output_device_with_config()`: تجرب المقبض الافتراضي أولاً، وعند فشله **تُعدّد كل أجهزة الإخراج الحقيقية** (`host.devices()`) وتأخذ أول جهاز يستطيع قراءة `default_output_config()`. هذا هو «فحص هل مصدر الصوت صحيح» واختيار بديل فعلي فوراً. |
| 2 | `mf_audio.rs:892`، `:1013-1014` | `setup_checked` و`rebind` يستخدمان الدالة الجديدة بدل المقبض الافتراضي وحده. |
| 3 | `mf_audio.rs:1140-1141` | الفحص الاستباقي للمسار يقارن بالمخرجات التي سيختارها الكود فعلاً، فلا يبقى على جهاز ميت ولا يرتد إلى مقبض فاسد. |
| 4 | `windows.rs:707`، `:757-762` | `setup_audio(report_failure)`: مصدر واحد فقط يُبلّغ الجلسة بالفشل. كان `setup_audio` و`maybe_recover` يُبلّغان معاً، فيتضاعف العدّ ويصل الـ backoff إلى 4 ثوانٍ بعد محاولتين. |

## 4. مراحل التحقق (كلها `[SOURCE-VERIFIED]` — شُغّلت فعلياً)

### المرحلة A — اختبارات الوحدة
- `cargo test --release --no-default-features --features native-audio --lib` → **147 passed; 0 failed; 1 ignored** (من 148).
- اختباران جديدان يحرسان السببين:
  - `session::tests::cooldown_grows_one_step_per_failure_and_caps` (`session.rs:539`) — يثبت أن كل فشل يزيد الخطوة **واحدة** (400/800/1600/3200/4000/4000 ms)، لا ضعفين.
  - `mf_audio::tests::a_headset_layout_mismatch_stays_retryable` — يثبت أن عدم تطابق تنسيق السماعة لا يُصنَّف صمتاً.

### المرحلة B — اختبارات التكامل متعددة السيناريوهات
الملف `tests/audio_route_switch.rs` (جديد)، **4 سيناريوهات، 4 passed; 0 failed**:
1. `recovery_restores_sound_after_several_failed_attempts` (سطر 133): 3 إخفاقات ثم نجاح → حدث `Recovered`، حالة `Playing`، وإطارات متولّدة.
2. `repeated_failures_never_give_up_and_never_fail_the_picture` (سطر 156): 4 محاولات متتالية فاشلة → لا استسلام، لا `Failed` للفيديو، والإطارات تستمر أثناء تعافي الصوت.
3. `a_route_change_arms_exactly_one_verification` (سطر 188): تغيير مسار واحد → محاولة تحقق واحدة بالضبط، بلا حلقة إعادة ربط.
4. `rapid_clip_switches_do_not_inherit_a_stuck_session` (سطر 219): 8 تبديلات سريعة أثناء فشل → كل إغلاق < 1 ثانية، و`Closed` آخر حدث دائماً.

### المرحلة C — تشغيل حقيقي على الجهاز
- `cargo build --release --no-default-features --features native-audio` → نظيف.
- `av_probe bunny.mp4 6` → `errors=0`, `reanchors=1`, `audio track open: 2ch 48000Hz`، **بلا أي تحذير `mf_audio` وبلا إعادة ربط عابرة** (`target/av_probe_after_fallback.log`).
- مجموعة التكامل الكاملة: `pacing` 5 + `first_play` 1 + `observability` 7 + `panic_boundary` 3 + `shutdown_blocked` 2 + `malformed_source` 2 + `audio_route_switch` 4 → كلها خضراء.

### المرحلة D — ما لا أستطيع إثباته هنا `[HYPOTHESIS]`
- الفصل الفعلي لسماعة USB ثم التمرير بين المقاطع على جهازك. المنطق الجديد يختار جهازاً حقيقياً بدل المقبض الافتراضي المتجمّد، ثم يعود إلى جهازك المختار تلقائياً حين يصبح متاحاً (`on_default_device` يقارن كل 500 ms).
- زمن الانتقال المتوقع الآن: **أقل من 500 ms** (دورة الفحص الاستباقي)، بدل دقائق. لكن هذا تقدير منطقي؛ أطلب منك قياسه على جهازك.

## 5. خلاصة صريحة
- الجولة الأولى أصلحت التوقف النهائي (تصنيف الخطأ + الحفاظ على التيار + فحص المسار).
- الجولة الثانية أصلحت **السبب المتبقي للانتظار الطويل**: المقبض الافتراضي المتجمّد الذي لا يُعدّد أجهزة حقيقية، وتسريع backoff المزدوج.
- لا أزعم أن المشكلة انتهت على جهازك قبل أن تختبرها أنت؛ الاختبارات تثبت أن **المنطق** سليم وأن الصوت يعمل فعلياً هنا، لكن إثبات زمن التبديل لحظياً يتطلب جهازك.
