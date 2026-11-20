# تخليق تدقيق الأسطول (30+ وكيل) — الحالة النهائية

> التاريخ: 2026-11-20. الأسطول: 31 وكيلاً (مستكشفون، نقّاد عدائيون، باحثون، مشرفو بنية، مدققو findings، كتاب خطط).
> القيود: لا commit/stage/push؛ لا عمل Linux؛ لا تغيير تواقيع عامة.
> الوسوم: **`[VERIFIED]`** مقروء من المصدر/سجل تشغيل؛ **`[HYPOTHESIS]`** استنتاج يحتاج جهازاً.

## 1. ما تعلّمه الأسطول فعلياً

الأسطول أنتج 4 وثائق خطط جديدة و12 تقرير مراجعة. أهم خلاصة: **المنصات الثلاث ليست متطابقة**.

| المنصة | حالة الصوت | كاشف تغيير المسار |hongθεση|
| --- | --- | --- | --- |
| Windows | مُفكِّك واحد يملك الصوت (`carries_audio=true`) | مُصلَح: `output_device_with_config` + فحص كل 500ms | إصلاح كامل هذه الجلسة `[VERIFIED]` |
| Android | فيديو فقط + مشغّل صوت عام مُزاوج | **لا يوجد كاشف داخل المكتبة** (`audio.rs:355-361` no-op) | يعتمد على AAudio الخارجي `[HYPOTHESIS]` |
| iOS | فيديو فقط + مشغّل صوت عام مُزاوج | cpal يرصد حدث التبديل لكنه **يُسجَّل كـ warn فقط** ولا يصل للجلسة | `[VERIFIED]` من cpal/dependency |

## 2. الإصلاحات التي طبّقتها هذه الجولة فعلياً `[VERIFIED]`

| # | الملف | الإصلاح | سببه |
|---|---|---|---|
| 1 | `src/player/threaded.rs:279-288` | `refresh=true` بعد أي `recover()` ناجح، لا فقط عند `Source` | كان iOS `Decode` يبقى `Failed` بلا قراءة جديدة → تجميد |
| 2 | `src/platform/apple.rs:174-199` | `open_at` يبني القارئ الجديد **قبل** إلغاء القديم | كان فشل الـ attach يُبقي `playback=None` → `Ok(None)` → إنهاء البث |
| 3 | `src/player.rs:316-330` + `src/player/threaded.rs:279-290` | `recovery_succeeded()` يضبط `recovering` عند نجاح التعافي؛ `resume_after_recovery` يمسح `Failed` عند أول إطار | فشل Apple/Android يُبلَّغ عبر `session.failed()` مع `Decode` غير transient، فتبقى الحالة `Failed` حتى بعد عودة الإطارات |
| 4 | `tests/audio_route_switch.rs` | اختبار جديد `a_session_reported_failure_resumes_to_playing_after_recovery` | يحرس الإصلاح 3 (الآن 5 سيناريوهات، كلها خضراء) |

مدقق الـ findings (V1) أعاد فحص 7 ادعاءات حرجة بعد هذه التعديلات فصنّف **6 منها `DISPROVED`** — أي أن الإصلاحات أزالت الأسباب فعلاً. الادعاع السابع (`hls live/seekable`) `DISPROVED` أيضاً لأن `Media::empty()` منع panic الافتتاح سلفاً.

## 3. ما تبقى مفتوحاً (بيّناه بصراحة)

- **كاشف مسار Android/iOS**: `PlaybackSession::route_changed()` (`session.rs:247-261`) **لا يوجد له مُستدعٍ إنتاجي** — فقط اختبار. `InterruptionReason::AudioFocusLoss` (`session.rs:67`) بلا مُستدعٍ. لا يمكن إصلاح هذا داخل المكتبة وحدها: يحتاج AVAudioSession/AudioManager من التطبيق، أو مُثبَّت cpal(y) يعيد توجيه الحدث للجلسة.
- **استعلام الموارد على الهاتف**: لا توجد أرقام موثوقة (R4)؛ القاعدة الصحيحة هي مُفكِّك واحد + استعلام `getMaxSupportedInstances` + تخفيض حراري عند `serious/critical`. كل الخانات `[MEASURED — TO BE FILLED]`.
- **إثبات جهازي**: لم يجرِ أي تشغيل على Android (NDK مفقود) أو iOS (لا جهاز). كل وعود «<500ms» و«الصوت يعود» `[HYPOTHESIS]`.
- **مُفكِّكان أثناء التمرير**: `rememberKeyed` يفتح الجديد قبل إسقاط القديم (`hooks.rs:41-42`)، و`close()` يفصل الخيط دون `join()` (`threaded.rs:121`). هذا سلوك مصمَّم ومُختبر (`shutdown_blocked.rs`)، خارج نطاق المكتبة.
- **واجهة عناصر التحكم**: الحالية تفتقر mute/volume/stop (`widget.rs:39-150`)؛ التصميم الجديد مقترح في `WIDGET-CONTROLS-DESIGN.md` كإضافات فقط، لم تُنفَّذ بعد.
- **تناقضات وثائقية**: وثائق التشخيص السابقة تشير إلى أرقام أسطر قديمة بعد الإصلاحات؛ يُفضَّل الرجوع لأسماء الرموز.

## 4. بوابات التحقق النهائية `[VERIFIED]`

- `cargo build --release --no-default-features --features native-audio` → نظيف.
- `cargo test --release ... --lib` → **147 passed; 0 failed; 1 ignored** (من 148).
- اختبارات التكامل: `pacing` 5 + `first_play` 1 + `observability` 7 + `panic_boundary` 3 + `shutdown_blocked` 2 + `malformed_source` 2 + `audio_route_switch` 5 → كلها خضراء.
- `cargo fmt --check` داخل المكتبة → exit 0.
- `git status` → `## No commits yet on master`؛ كل التعديلات غير مُودَعة.

## 5. الوثائق الجديدة من الأسطول

- `docs/PRODUCTION-MASTER-PLAN.md` — الخطة المرحلية (5 مراحل).
- `docs/WIDGET-CONTROLS-DESIGN.md` — تصميم عناصر تحكم بأسلوب iOS (إضافة فقط).
- `docs/MOBILE-RISK-REGISTER.md` — سجل مخاطر مُرتب بالخطورة.
- `docs/MOBILE-TEST-MATRIX.md` — 34 سيناريو اختبار (5 فئات + إدارة حرارية/ذاكرة).
- `docs/EXECUTIVE-DECISION-MEMO.md` — قرار NO-GO مشروط بـ 6 شروط توقيع.
- `docs/FIX-SEQUENCING.md` — ترتيب تنفيذ الإصلاحات المتبقية.

## 6. الحكم النهائي

**لا توقيع إنتاجي (signoff) قبل تشغيل جهازي حقيقي** على Android وiOS مع سجل أوامر/بيئة/مخرجات. الأسطول لم يُغلق الفجوة بين «مقروء من المصدر» و«يعمل على هاتف» — بل صنع قائمة P0 دقيقة من نفس القراءة الثابتة. الإصلاحات التي أمكن إثباتها محلياً نُفِّذت وتحققت؛ الباقي مُوثَّق بصراحة كـ `HYPOTHESIS` ومحجوز للقياس.
