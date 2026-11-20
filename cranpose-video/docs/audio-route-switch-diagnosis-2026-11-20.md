# لماذا يتوقف الصوت عند التبديل بين سماعة الرأس وسماعة الجهاز

> التاريخ: 2026-11-20. المصادر: `D:\counter\diagnostics.log` وقراءة مباشرة لشيفرة `cranpose-video`.
> **`[SOURCE-VERIFIED]`** = مقروء من السجل/المصدر برقم سطر. **`[HYPOTHESIS]`** = استنتاج يحتاج إثباتاً على جهازك.
> كل التعديلات غير مُودَعة (no commit / stage / push).

## 1. ما يثبته ملف الأخطاء `[SOURCE-VERIFIED]`

- الأسطر **375-376**، **415-416**، **894-895**، **897-898**، **904-905**، **906-907**:
  ```
  [WARN] [mf_audio.setup] default output config unavailable
  [WARN] [mf_audio.setup.deferred] audio device unavailable; recovery will retry on the decode thread
  ```
  أي أن فتح مسار الصوت يفشل في لحظة تبديل الجهاز لأن `default_output_config()` تُخفق.
- البحث في السجل كاملاً لا يُظهر **أي سطر `mf_audio.rebind` ناجح** في لحظة التبديل — آلية التحويل المباشر لم تعمل.
- الأسطر **705-744**: `[mf_audio.setup] source has no selectable audio stream` يتكرر، ثم `[audio.prepare] prepare failed …: the item has no audio track` (716، 720، 740، 744) — أي أن مسار الصوت البديل فشل هو أيضاً، فصمت الجهازان معاً.

## 2. الأسباب الجذرية الثلاثة

### السبب 1 (الرئيسي): خطأ في الجهاز يُصنَّف خطأً كـ «المصدر بلا صوت»، فيُوقف الاستعادة نهائياً

- قبل الإصلاح كان `setup_checked` يُرجع `SetupError::NoAudio` عندما لا يقبل Media Foundation تنسيق PCM المطلوب للجهاز (كان `mf_audio.rs:863-867`). التنسيق **يُطلب من الجهاز الافتراضي الحالي** (`request_float(reader, channels, rate)`، `mf_audio.rs:1233`)، لا من المصدر. سماعة رأس بمعدل 44.1 kHz مقابل سماعة 48 kHz كافية لإخفاق التفاوض.
- `SetupError::NoAudio` تعني `retryable() == false` (`mf_audio.rs:776-778`)، فكانت تضبط `source_has_audio = false` في `windows.rs:705`.
- ثم `recover()` كان يرى `!source_has_audio` ويرجع `true` بوصفه «نجاحاً» (`windows.rs:1048-1049`)، فيستدعي العامل `session.recovered()` (`threaded.rs:279-280`) الذي يُصفّر الـ backoff و**يلغي كل المحاولات القادمة** (`session.rs:341-356`).
- النتيجة: سكون دائم على الجهازين، بلا إعادة تشغيل مباشرة أو مؤجلة. هذا يطابق وصفك: «من محاولة أول إلى ثلاثة بعدها لا يكشف المصدر… يتوقف في المصدرين ولا يعيد التشغيل».

### السبب 2: لا يوجد فحص دوري لصحة جهاز الإخراج `[SOURCE-VERIFIED]`

- آلية التحويل الوحيدة كانت علم `device_changed` الذي يضبطه callback جهاز `cpal` فقط (`mf_audio.rs:880-888` و`1016-1020`). إذا لم يُرسل Windows حدث `StreamInvalidated` — أو أرسله لجهاز اختفى بعد لحظة — يبقى العلم `false` ولا يحدث تحويل، ويبقى التيار على نقطة نهاية لا يسمعها أحد.
- النمط الصحيح كان موجوداً أصلاً لكن **للغلاف العام فقط**: `audio.rs:355-405` (`observe_output_route` + `current_output_route` + `ROUTE_POLL_COOLDOWN=500ms`)، لكنه لم يُطبَّق على المسار الأصلي Windows الذي يملك مصدراً واحداً للصوت والفيديو.

### السبب 3: إسقاط التيار القديم عند أي فشل `[SOURCE-VERIFIED]`

- `setup_audio` كان ينفّذ `self.audio = setup.audio` بلا شرط (`windows.rs:705-706`)، أي `None` عند الفشل. هذا يُسقط التيار الوحيد الذي ربما كان لا يزال يعمل على الجهاز القديم، فيصمت الجهازان معاً بدل أن يبقى أحدهما قادراً على العودة.

## 3. الإصلاحات المطبَّقة

| # | الملف | ما تغيّر |
|---|---|---|
| 1 | `src/platform/windows/mf_audio.rs:799-810` | دالة نقية `negotiation_ok` تُصنّف عدم تطابق تنسيق PCM كـ `Unavailable` (**قابل لإعادة المحاولة**) لا `NoAudio`. تُستخدم في `setup_checked`. |
| 2 | `src/platform/windows/mf_audio.rs:889-893` | فشل `request_float` للجهاز الحالي أصبح `Unavailable` أيضاً، برسالة توضّح أنه مرتبط بالجهاز. |
| 3 | `src/platform/windows/mf_audio.rs:752-757`، `1119-1139` | حقول `bound_device` (`cpal::Device` من نوع Eq+Send)، `bound_layout`، `last_route_check`؛ ودالة `on_default_device()` تقارن الجهاز الافتراضي الحالي بما ارتبط به التيار. |
| 4 | `src/platform/windows/mf_audio.rs:1142-1156` | `pump()` يتحقق من المسار كل 500 ms ويضبط علم التحويل حتى لو لم يأتِ callback — هذا هو «الفحص هل المصدر صحيح». الجهاز الموجود لكن الذي لا يستطيع بعد إعلان تنسيقه يُعامل كمسار في طور التبديل، فتُعاد المحاولة. |
| 5 | `src/platform/windows/mf_audio.rs:1045-1047` | بعد نجاح الربط يُحدَّث `bound_device`/`bound_layout` حتى لا يرتد الصوت إلى الجهاز الذي غادره المستخدم. |
| 6 | `src/platform/windows.rs:673-675`، `114` | حقل `audio_ever_present`: بمجرد ظهور مسار صوت يبقى صحيحاً، فلا يُلغي فشل جهاز عابر الاستعادة. |
| 7 | `src/platform/windows.rs:707-760` | `setup_audio()` لا يُسقط التيار القديم عند الفشل؛ يستبدله فقط عند نجاح بناء تيار جديد، ويُقاعد القديم على خيط منفصل (`cranpose-video-retire`) لتفادي تجميد WASAPI. |
| 8 | `src/platform/windows.rs:1068-1079` | `recover()` يُرجع نجاحاً فقط عند بناء تيار فعلي، لا لمجرد وجود تيار قديم، فتبقى المحاولات مجدولة حتى يعود الصوت. |

القيود الأصلية محفوظة: لا resampling، لا تغيير طبقة/زمن، تطابق PCM تماماً، الصوت لا يُسقط الفيديو، ومصدر واحد يقرأ المسارين.

## 4. التحقق `[SOURCE-VERIFIED]`

- `cargo build --release --no-default-features --features native-audio` → نظيف بلا أخطاء أو تحذيرات.
- `cargo test --release --no-default-features --features native-audio --lib` → **146 passed; 0 failed; 1 ignored** (من 147؛ الاختبار الجديد `a_headset_layout_mismatch_stays_retryable`).
- اختبارات التكامل `pacing` (5)، `first_play` (1)، `observability` (7)، `panic_boundary` (3)، `shutdown_blocked` (2)، `malformed_source` (2) → كلها خضراء.
- `av_probe` على bunny.mp4 → `errors=0`، `reanchors=1`، بلا تحذيرات `mf_audio` وبلا إعادة ربط عابرة (`target/av_probe_after_switch_fix.log`).

## 5. ما يحتاج إثباتاً على جهازك `[HYPOTHESIS]`

- التبديل الفعلي سماعة↔سماعة أثناء التشغيل، والتحقق أن الصوت ينتقل خلال <500 ms دون انقطاع مسموع. لا يمكن محاكاة نقطة نهاية WASAPI فعلية هنا؛ الفحص الدوري الجديد يغطي منطقياً حالة غياب حدث `StreamInvalidated`.
- سماعة رأس بمعدل عيّنات مختلف (44.1 مقابل 48 kHz): يجب أن يبقى المسار قابلاً لإعادة التفاوض بدل أن يُصنَّف صمتاً.
- إذا بقي الصوت متوقفاً بعد هذا الإصلاح، أرفق سجلاً جديداً؛ سننظر تحديداً إلى سطور `mf_audio.rebind` و`audio.recover` لمعرفة أي مرحلة توقفت.

> ملاحظة unrelated من السجل: السطر 567 PANIC في `cranpose-wgpu-hal` أثناء تجميع الأنابيب، والأسطر 12/163 PANIC في HLS القديم. كلاهما خارج نطاق الصوت وقد عولج HLS لاحقاً؛ لا علاقة لهما بتوقف الصوت.
