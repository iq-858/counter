# الخطة الرئيسية للإنتاج — `cranpose-video` (Production Master Plan)

> **التاريخ:** 2026-11-20. **الإصدار:** Rust edition 2024 (`Cargo.toml:1-8`).
> **الغرض:** خطة مرحلية للمراجعة الكاملة للمكتبة، والتقصي المكثّف لمنصّتي Android/iOS، والجاهزية للإنتاج، وإضافة عناصر تحكم بأسلوب iOS，**دون كسر أي واجهة عامة** (strict non-breaking API).
> **قيد النطاق لهذه المهمة:** كُتب **هذا الملف فقط**. لم تُعدّل أي شيفرة، ولم يُنشأ commit، ولم يُجرَ أي عمل على Linux. جميع التعديلات السابقة ما زالت **غير مُودَعة** (كما هو موثّق في `docs/final-report-ar.md:5,179`).
> **ملاحظة مستودعية [SOURCE-VERIFIED]:** لا يوجد `.git/HEAD` داخل `D:\counter\cranpose-video` ولا داخل `D:\counter` في هذه الشجرة؛ لذلك تعتمد المرحلة 0 على لقطة ملفات بدل افتراض وجود `git status`.

---

## 0. وسوم الأدلة (قواعد إلزامية لكل المراحل)

| الوسم | المعنى | قاعدة الاستخدام في هذه الخطة |
|---|---|---|
| **`[SOURCE-VERIFIED]`** | حقيقة مقروءة مباشرة من ملف مصدر/وثيقة موجودة في الشجرة، مع مرجع `file:line`. | يُستخدم لكل وصف شيفرة؛ لا يحتاج تشغيلاً. |
| **`[MEASURED]`** | رقم ناتج عن تشغيل فعلي **مذكور في مستند موجود**، مع ذكر المستند والسطر. | لا يُستحدث أي رقم جديد في هذه الخطة؛ الأرقام أدناه منقولة حرفياً وبإسنادها فقط. |
| **`[HYPOTHESIS]`** | نتيجة متوقعة على جهاز/شبكة/زمن تشغيل لم تُقَس هنا. | كل وعود الأداء على الأجهزة الحقيقية تُوسَم بهذا حتى يُقاس. |
| **`[MEASURED — TO BE FILLED]`** | خانة محجوزة لقياس مستقبلي. | تبقى **فارغة**؛ لا تُكتب فيها قيم مختلقة. |
| **`[REPORT-ONLY]`** | رقم تاريخي من سجلّات لم تعد موجودة في هذه الشجرة. | لا يُقدَّم كدليل حالي (انظر `docs/final-report-ar.md:7,11-22`). |

**قاعدة ذهبية:** لا يرقّى أي `[HYPOTHESIS]` إلى `[MEASURED]` إلا بأمر تشغيل + بيئة + مخرجات مسجَّلة في مستند المرحلة 5.

---

## 1. السياق: ما أُصلح للتو، وما الذي ينتظر هذه الخطة

| البند | الحقيقة | الوسم والمرجع |
|---|---|---|
| إصلاح المقبض الافتراضي (default-handle fallback) | `output_device_with_config()` تجرب المقبض الافتراضي ثم تُعدّد أجهزة الإخراج الحقيقية؛ تُستخدم في `setup_checked` و`rebind`، والفحص الاستباقي يقارن بالمخرجات التي سيختارها الكود فعلاً. | `[SOURCE-VERIFIED عبر docs/audio-route-switch-diagnosis-round2-2026-11-20.md:19-31]` — أسماء المسارات الحالية في `src/platform/windows/mf_audio.rs` كما وردت في ذلك المستند: `:119-137`, `:892`, `:1013-1014`, `:1140-1141`. |
| إصلاح الـ backoff المزدوج | مصدر واحد يُبلّغ الجلسة بالفشل عبر `setup_audio(report_failure)`؛ كان `setup_audio` و`maybe_recover` يُبلّغان معاً فيتضاعف العدّ. | `[SOURCE-VERIFIED عبر docs/audio-route-switch-diagnosis-round2-2026-11-20.md:32]` — `src/platform/windows/mf_audio.rs:707`, `:757-762`. الحارس الحالي في `src/session.rs:539-565`. |
| أساس الاختبار بعد الإصلاح | `147 passed; 0 failed; 1 ignored` (من 148) واختبارات تكامل `4 passed; 0 failed`. | `[MEASURED — تشغيل مضيف سابق، docs/audio-route-switch-diagnosis-round2-2026-11-20.md:37,43-47]`. |
| زمن التبديل المتوقع على جهاز المستخدم | «أقل من 500 ms» تقدير منطقي لدورة الفحص الاستباقي، بدل دقائق. | `[HYPOTHESIS — docs/audio-route-switch-diagnosis-round2-2026-11-20.md:54-56]` — لا يُقدَّم كرقم مُقاس. |
| إلغاء `cancelReading` على Apple | موجود في المصدر في `open_at` و`close`، لكنه غير مُختبَر على جهاز. | `[SOURCE-VERIFIED] src/platform/apple.rs:179-182`, `:396-402`; `[HYPOTHESIS] docs/mobile-platform-audit.md:36,88,202`. |
| فقدان تركيز الصوت على Android | `InterruptionReason::AudioFocusLoss` مُعرَّف لكنه **غير موصول بأي مُستدعٍ**. | `[SOURCE-VERIFIED] src/session.rs:65-70`; `docs/final-report-ar.md:196`. |

---

## 2. العقد غير القابل للكسر (non-breaking API contract)

هذه هي القيود التي تُراجَع في **كل** مرحلة؛ أي خرق يُقفل البوابة فوراً.

| # | الثابت | المرجع `[SOURCE-VERIFIED]` |
|---|---|---|
| C1 | `VideoPlayerHandle` يبقى `#[derive(Clone)]` بحقل خاص واحد `Rc<Result<VideoPlayer, VideoError>>`؛ لا حقول عامة جديدة. | `src/widget.rs:27-30` |
| C2 | تواقيع `play/pause/toggle/seek/seek_fraction/set_speed/set_looping/is_looping/set_muted/is_muted/set_volume/volume/position/duration/state/speed/info/decoded_size` لا تتغير. | `src/widget.rs:39-150` |
| C3 | تواقيع `rememberVideoPlayer`, `Video`, `VideoWithControls`, `VideoControls`, `VideoProgressSlider`, `VideoProgressSliderWith`, `progress_fraction_for` لا تتغير. | `src/widget.rs:156-166`, `172-175`, `180-183`, `190-272`, `274-299`, `301-356`, `358-370` |
| C4 | `VideoDecoder` يبقى مع نفس التواقيع؛ أي إضافة تكون hook جديداً **بقيمة افتراضية** فقط. نموذج موجود: `observe_shutdown`, `attach_session`, `recover`, `carries_audio`. | `src/android.rs:1307-1322`; `src/apple.rs:404-433`; `src/audio.rs:685-710`; `src/session.rs:1-5` |
| C5 | `PlaybackSession` ينمو بإضافات فقط؛ `RECOVERY_COOLDOWN`/`MAX` و`EVENT_CAPACITY` وتسلسل الأحداث تبقى كما هي. | `src/session.rs:31-46`, `152-440`, `402-418` |
| C6 | ميزات `Cargo.toml` وافتراضاتها (`default = ["widget","player"]`) لا تتغير؛ أي مسار جديد خلف اسم composable/feature **جديد**. | `Cargo.toml:15-40` |
| C7 | لا تغيير في `output_format()` ولا في `VideoFrame::from_nv12`؛ التطبيع يُضاف **قبلها** كطبقة اختيارية. | `src/android.rs:1291-1293`; `src/apple.rs:388-390`; `docs/mobile-platform-audit.md:188-192` |
| C8 | لا عمل Linux، ولا تعديل في `src/platform/linux.rs` أو ميزة `linux-gstreamer` ضمن هذه الخطة. | `Cargo.toml:30-35`; قيد المهمة |

**قاعدة الإضافة (additive-only):** كل عنصر جديد يحصل على **اسم جديد**؛ الدوال القديمة تستدعي المسار القديم حرفياً. لا إعادة تسمية، لا تغيير توقيع، لا حذف، لا تغيير للقيم الافتراضية.

---

## 3. المرحلة 0 — بوابات الأساس (Phase 0 — Gates)

**الهدف:** تثبيت خط الأساس قبل لمس أي شيفرة، حتى يمكن إثبات أن أي انحدار جاء من مرحلة لاحقة.

**DRI:** Master Planner (P1) يملك هذه الوثيقة؛ المالك (Owner) يمنح الإذن بالانتقال. أسماء الأشخاص يُحدّدها المالك؛ لا تُخترع هنا.

| # | الإجراء | المرجع/الأداة | الوسم |
|---|---|---|---|
| 0.1 | حفظ لقطة كاملة لشجرة `D:\counter\cranpose-video` (الملفات + تواريخها) قبل أي تعديل، لأن لا `.git` محلي. | لا `.git/HEAD` في المسارين `cranpose-video` و`counter` | `[SOURCE-VERIFIED]` |
| 0.2 | تسجيل الحالة الحالية للاختبارات كخط أساس **دون إعادة اختراع أرقام**: يُعاد تشغيل الأوامر المذكورة في الوثائق وتُسجَّل مخرجاتها الجديدة. الأرقام القديمة (`147/0/1`، `4/0`) تُوسَم تاريخياً فقط. | `docs/audio-route-switch-diagnosis-round2-2026-11-20.md:36-53` | `[MEASURED (historical)]` |
| 0.3 | بوابة الشكل: `rustfmt --check --edition 2024` على الملفات التي ستُلمَس لاحقاً. | `Cargo.toml:4` | `[SOURCE-VERIFIED]` |
| 0.4 | بوابة البناء/الاختبار المعتادة على مضيف Windows فقط: `--no-default-features --features native-audio --lib`، ثم `--all-targets`، ثم doc tests. لا يُدعى نجاح أي أمر لم يُشغَّل فعلياً. | `docs/audio-route-switch-diagnosis-round2-2026-11-20.md:36-52`, `docs/final-report-ar.md:11-22` | `[MEASURED — TO BE FILLED]` |
| 0.5 | مراجعة العقد C1–C8 وتوقيع Bootstrap: أي تعديل سابق خرق أحدها يُعالَج **قبل** Phase 1. | القسم 2 أعلاه | `[SOURCE-VERIFIED]` |
| 0.6 | تأكيد نطاق الجهاز: Windows متاح؛ Android/iOS غير متاحين محلياً (NDK غائب، لا جهاز iOS). | `docs/android-test-checklist.md:3-8`; `docs/final-report-ar.md:98,186` | `[MEASURED — TO BE FILLED]` |

**بوابة الخروج (Exit criteria):**
- لقطة Baseline محفوظة ومؤرخة. ✅
- كل أمر في 0.3–0.4 له مخرج مسجَّل أو خانة `[MEASURED — TO BE FILLED]` فارغة صراحةً. ✅
- توقيع Owner على C1–C8 وعلى «لا Linux work». ✅
- **لا تعديل مصدر في المرحلة 0.** ✅

---

## 4. المرحلة 1 — تقصية وتصلب Android (Phase 1 — Android hardening)

**DRI:** Android/MediaCodec owner (يُسمَّى من قِبل المالك). المراجع النهائية: `docs/mobile-platform-audit.md:92-101` (AND-1..5) و`:62-76` (H4-H8).

**الهدف:** إزالة أعلى خطر جوال — افتراضات ترتيب الطائرات/اللون والـ stride — **كطبقة تطبيع إضافية** قبل `VideoFrame::from_nv12`، مع فصل «صورة غير صالحة مؤقتاً» عن نهاية البث.

| # | المهمة | المكان `[SOURCE-VERIFIED]` | الوسم/الحالة |
|---|---|---|---|
| 1.1 | فحص `Image.getPlanes` وتحديد ترتيب الكروما فعلياً (NV12 مقابل NV21/Vu) قبل نسخ `planes_of`; إبقاء التفسير الحالي كافتراضي، بلا تغيير `output_format()`. | `src/platform/android.rs:743-755`, `791-845`, `1291-1293` | `[SOURCE-VERIFIED]` للخطر: `docs/mobile-platform-audit.md:96` (AND-1), H4 `:67` `[HYPOTHESIS]` |
| 1.2 | تطبيع اللون: توسيع خريطة `COLOR_STANDARD` (Bt709 عند `1` وإلا Bt601) لتغطية BT2020/PQ/HLG مع إبقاء `YuvColor` الحالي ك fallback؛ قراءة `COLOR_RANGE` كما هي. | `src/platform/android.rs:944-953`, `1295-1305` | `[SOURCE-VERIFIED]`؛ فقدان HDR `[HYPOTHESIS]` H6 `:72` |
| 1.3 | طبقة نسخ تحترم `getRowStride`/`getPixelStride` و`cropRect` وتُنتج NV12 محكماً؛ لا تغيير في `Plane` ولا في `from_nv12`. | `src/platform/android.rs:1353-1405`, `711-855`, `1485-1493` | `[SOURCE-VERIFIED]` AND-2 `:97`; `[HYPOTHESIS]` H7 `:73` |
| 1.4 | فصل `Ok(None)` المؤقت عن EOF: عدّ محاولات محدودة/تأجيل قصير قبل ضبط `finished`، بدل اعتباره نهاية فورية في Worker. | `src/platform/android.rs:363-396`, `378-393`; `src/player/threaded.rs:367-402` | `[SOURCE-VERIFIED]` AND-3 `:98`, THR-2 `:125`; `[HYPOTHESIS]` H5 `:71` |
| 1.5 | مسار خروج مبكر عند seek/close بدل انتظار `EMPTY_DEADLINE` (~30 s)؛ يبقى منطق EOS العادي كما هو. | `src/platform/android.rs:42-45`, `363-396`, `1324-1350`; `src/player/threaded.rs:111-122` | `[SOURCE-VERIFIED]` AND-4 `:99` |
| 1.6 | التحقق من تحديث `info` أثناء القراءة مقابل الوسيط بعد التغيير (لا استبدال إلا عند صفر الأبعاد حالياً). | `src/platform/android.rs:919-958`; `src/player/threaded.rs:438-444` | `[SOURCE-VERIFIED]` AND-5 `:100`; `docs/mobile-platform-audit.md:39` (F13) |
| 1.7 | توصيل `AudioFocusLoss` بمُستدعٍ فعلي عبر `PlaybackSession::interrupted(...)` — إضافة hook افتراضية فقط، بلا تغيير في `InterruptionReason`. | `src/session.rs:65-70`, `226-238`; `docs/final-report-ar.md:196` | `[SOURCE-VERIFIED]` للغياب؛ `[HYPOTHESIS]` للاستعادة المسموعة |
| 1.8 | تشغيل قائمة جهاز Android كما هي، مع تسجيل النتائج في الخانات الفارغة. | `docs/android-test-checklist.md:21-81` | `[MEASURED — TO BE FILLED]` |

**بوابة الخروج (Exit criteria):**
- كل مسار جديد خلف قيمة افتراضية/خيار جديد؛ C1–C8 سليمة (خاصة C7). ✅
- اختبارات device-free جديدة تغطي 1.1–1.6 ببيانات طائرات مُصنَّعة (بدون جهاز). ✅
- تقرير جهازي يملأ بنود `android-test-checklist.md`، أو يبقى موسوماً `[MEASURED — TO BE FILLED]` بلا أرقام مختلقة. ✅
- لا انحدار في بوابة المرحلة 0 على مضيف Windows. ✅

---

## 5. المرحلة 2 — تقصية وتصلب iOS (Phase 2 — iOS hardening)

**DRI:** Apple/AVFoundation owner (يُسمَّى من قِبل المالك). المراجع: `docs/mobile-platform-audit.md:82-91` (IOS-1..5) و`:133-139`.

**الهدف:** تقليل افتراضات اللون والهندسة، وتأكيد إلغاء `cancelReading` على الجهاز، كل ذلك بإضافات لا تغيّر `open_at/close/seek` ولا `output_format`.

| # | المهمة | المكان `[SOURCE-VERIFIED]` | الوسم/الحالة |
|---|---|---|---|
| 2.1 | قراءة بيانات اللون/النطاق من `CMFormatDescription`/attachments عند توفرها، مع الإبقاء على `YuvColor::for_size` كافتراضي عند الغياب. | `src/platform/apple.rs:392-394`, `549-565`; `docs/mobile-platform-audit.md:86` (IOS-1) | `[SOURCE-VERIFIED]`؛ الانزياح `[HYPOTHESIS]` H1 `:64` |
| 2.2 | حساب clean aperture/crop وإعادة حزم الطائرات إلى NV12 محكم قبل `copy_planes`، مع إبقاء المسار الحالي ك fallback. | `src/platform/apple.rs:302-368`, `65-79`; `docs/mobile-platform-audit.md:87` (IOS-2) | `[SOURCE-VERIFIED]`؛ التشويه `[HYPOTHESIS]` H2 `:65` |
| 2.3 | تحقّق فعلي من `CVPixelBufferGetPixelFormatType` قبل افتراض BGRA/NV12؛ الثوابت الحالية `BGRA=0x42475241`, `NV12=0x34323076`. | `src/platform/apple.rs:257-300`, `528-547` | `[SOURCE-VERIFIED]` IOS-4 `:89` |
| 2.4 | قياس زمن seek على جهاز لإثبات أن `cancelReading()` في `open_at` يمنع الإطارات القديمة؛ لا تغيير واجهة `Playback` مطلوب. | `src/platform/apple.rs:174-199`, `396-402` | `[SOURCE-VERIFIED]` للمصدر؛ `[HYPOTHESIS]` H3 `:68` و`IOS-3 :88` |
| 2.5 | مسار IOSurface/Metal اختياري خلف **feature flag جديد**؛ لا إلغاء للمسار الحالي. | `docs/mobile-platform-audit.md:90` (IOS-5) | `[HYPOTHESIS]`； C6 يمنع تغيير الافتراضات |
| 2.6 | فحص الحدود/الأنماط الحالية التي ترفض الأبعاد الفردية و`<2` طائرة وstride القصير — تُبقى كما هي دون تغيير. | `src/platform/apple.rs:318-340`; `docs/mobile-platform-audit.md:87` | `[SOURCE-VERIFIED]` F3 `:35` |

**بوابة الخروج (Exit criteria):**
- C7 وC4 سليمتان؛ لا تغيير في `output_format()` ولا في توقيع `open_at/close/seek`. ✅
- اختبارات device-free لمنطق الحزم/اللون الجديد (بدون جهاز iOS). ✅
- نتيجة 2.4 على جهاز iOS تُسجَّل أو تبقى `[MEASURED — TO BE FILLED]`. ✅
- البناء على مضيف Windows يبقى أخضر؛ لا Linux work. ✅

---

## 6. المرحلة 3 — عناصر تحكم بأسلوب iOS (Phase 3 — widget controls, additive only)

**DRI:** Widget/UI owner (يُسمَّى من قِبل المالك). **قيد حاسم:** هذه المرحلة **إضافة فقط (additive only)**؛ لا تعديل أي سطر من الدوال القائمة.

**الهدف:** توفير مجموعة composables جديدة بأسلوب iOS (شريط تحكم مدمج، تسمية زمن، مؤقّت/تبديل، تخطيط آمن للشاشات المحمولة) دون المساس بالسلوك الحالي.

### 6.1 ما يُستخدم كما هو (لا يُلمَس) `[SOURCE-VERIFIED]`

| الأصل | المرجع | الاستخدام الجديد |
|---|---|---|
| `VideoPlayerHandle` مع `seek_fraction`, `set_speed`, `set_volume`, `set_muted`, `decoded_size` | `src/widget.rs:27-30`, `69-75`, `105-111`, `147-148` | تُقرأ فقط من التجميع الجديد. |
| `VideoProgressSliderWith` (سحب أثناء التشغيل لا يزيح المؤشر) | `src/widget.rs:301-356`, خصوصاً `:315`, `:325-331`, `:344-352` | يُعاد استخدامه؛ لا إعادة تنفيذ منطق السحب. |
| `progress_fraction_for` (مدخلات بلا مدة ⇒ `0.0`) | `src/widget.rs:358-370` | يُستخدم للتسمية النسبية. |
| `LaunchedEffect` + `with_frame_nanos` لتحديث الحالة | `src/widget.rs:192-203`, `284-292`, `313-332` | نفس النمط للساعة الجديدة. |
| `control(...)` و`status(...)` و`clock(...)` | `src/widget.rs:476-492`, `494-512`, `514-519` | لا تُعدَّل؛ التجميع الجديد يبني أزراره الخاصة بأسماء جديدة. |

### 6.2 الإضافات المقترحة (كلها بأسماء جديدة)

| # | العنصر الجديد (الاسم المقترح) | السلوك | الوسم |
|---|---|---|---|
| 3.1 | `VideoControlsCompact(handle, modifier)` + `VideoWithCompactControls(...)` | شريط تحكم رفيع: toggle، -5s/+5s، مؤقّت زمني. يعيد استخدام `SEEK_STEP=5s` (`src/widget.rs:14-15`) و`VideoProgressSlider`. | `[SOURCE-VERIFIED]` للأصول؛ التصميم `[HYPOTHESIS]` حتى مراجعة UI |
| 3.2 | `VideoTimeLabel(handle, modifier, style)` | يعرض `clock(position) / clock(duration)` مع كلمة «live» عند غياب المدة (`src/widget.rs:495-498`)؛ لا تغيير في `clock` نفسه. | `[SOURCE-VERIFIED]` |
| 3.3 | `VideoControlsStyle` (بنية جديدة بقيم افتراضية) | ألوان/أحجام الشريط والزر؛ القيم الافتراضية مطابقة للموجود حالياً (`PROGRESS_TRACK_HEIGHT=4.0`, `PROGRESS_THUMB=12.0` في `src/widget.rs:17-21`) حتى لا يتغير مظهر القديم. | `[SOURCE-VERIFIED]` |
| 3.4 | تبديل «المتبقّي / المنقضي» (remaining toggle) | حساب `duration - position` من `handle` فقط؛ لا ساعة جديدة. | `[HYPOTHESIS]` لتجربة المستخدم؛ `[SOURCE-VERIFIED]` للمصدر |
| 3.5 | تمرير سحب المؤشر أثناء التشغيل | موجود أصلاً في `VideoProgressSliderWith`؛ يُعاد استخدامه لا إعادة كتابته. | `[SOURCE-VERIFIED] src/widget.rs:315,325-331` |
| 3.6 | إمكانية الوصول: أسماء/تسميات نصية للأزرار الجديدة | تُضاف للعناصر الجديدة فقط؛ النمط الحالي يستخدم `Text` داخل `Button` (`src/widget.rs:476-492`). | `[SOURCE-VERIFIED]` للنمط؛ `[HYPOTHESIS]` للتقييم على جهاز |

> **تنبيه اعتمادي [HYPOTHESIS]:** توفر أي gesture إضافي (نقر على الفيديو للإظهار/الإخفاء، pinch) يعتمد على primitives غير مؤكدة في `cranpose-ui` المُثبَّت (`Cargo.toml:46-47`). لا يُفترض وجودها؛ يُكتفى بـ `Button`/`Slider` المعروفين حتى يُتحقَّق من المصدر. لا تُخترع واجهة.

**بوابة الخروج (Exit criteria):**
- `git diff` (أو لقطة الملفات) يُظهر **صفر تعديل** داخل `src/widget.rs`؛ كل الإضافات في عناصر جديدة/وحدة جديدة. ✅
- اختبارات composable جديدة تتحقق من: سحب لا يزيح المؤشر، مدة غائبة ⇒ تسمية «live»، القيم الافتراضية للـ style مطابقة للقديم. ✅
- لا تغيير في قيم `SEEK_STEP/PROGRESS_TRACK_HEIGHT/PROGRESS_THUMB`. ✅
- لا انحدار في بوابة المرحلة 0. ✅

---

## 7. المرحلة 4 — منظومة الاختبار (Phase 4 — test harness)

**DRI:** Test/QA owner (يُسمَّى من قِبل المالك). المرجع الحالي: `tests/audio_route_switch.rs` (4 سيناريوهات موثّقة في `docs/audio-route-switch-diagnosis-round2-2026-11-20.md:43-47`).

**الهدف:** تحويل كل إصلاح/إضافة في المراحل 1–3 إلى حارس آلي device-free، وتجهيز قنوات القياس على الأجهزة دون اختراع أرقام.

| # | الحارس المطلوب | المكان المرجعي `[SOURCE-VERIFIED]` | الوسم |
|---|---|---|---|
| 4.1 | حارس backoff بخطوة واحدة (موجود بالفعل، يُبقى ويُوسَّع). | `src/session.rs:539-565` | `[SOURCE-VERIFIED]` |
| 4.2 | حارس فشل الـ prepare مرة واحدة لكل مصدر + إعادة التسليح. | `src/audio.rs:944-1007` | `[SOURCE-VERIFIED]` |
| 4.3 | حارس `needs_attach` بعد فشل rebind. | `src/audio.rs:929-942`, `:509-516` | `[SOURCE-VERIFIED]` — ملاحظة AAudio `[HYPOTHESIS]` في `:936-939` |
| 4.4 | حارس ربط المسار يتطلب تطابق PCM تام. | `src/audio.rs:862-871`, `:96-105` | `[SOURCE-VERIFIED]` |
| 4.5 | حارس الاستعادة: فشل ⇒ Recovering ⇒ نجاح ⇒ Playing مع إطار منشور. | `src/player/threaded.rs:1058-1125` | `[SOURCE-VERIFIED]` |
| 4.6 | حارس بث حيّ فارغ بعد الاستعادة ⇒ إعادة تسليح لا إنهاء. | `src/player/threaded.rs:1156-1208` | `[SOURCE-VERIFIED]` |
| 4.7 | حارس Panic boundary للـ decoder. | `src/player/threaded.rs:157-183` | `[SOURCE-VERIFIED]` |
| 4.8 | سيناريوهات التكامل الأربعة الحالية (`tests/audio_route_switch.rs`) تُبقى وتُشغَّل مع كل تغيير. | `docs/audio-route-switch-diagnosis-round2-2026-11-20.md:43-47` | `[MEASURED (historical)]` |
| 4.9 | حراس جديدة لمرحلة Android (طائرات/ألوان/`Ok(None)`/خروج مبكر) ببيانات مُصنَّعة. | المراحل 1.1–1.6 | `[MEASURED — TO BE FILLED]` |
| 4.10 | حراس جديدة لمرحلة iOS (حزم/لون/نوع البكسل) ببيانات مُصنَّعة. | المراحل 2.1–2.6 | `[MEASURED — TO BE FILLED]` |
| 4.11 | حراس composable للمرحلة 3 (بدون جهاز). | المرحلة 3 | `[MEASURED — TO BE FILLED]` |
| 4.12 | قناة قياس جهازية: أمر + بيئة + مخرجات + طراز جهاز + إصدار نظام؛ تُسجَّل في المرحلة 5 فقط. | `docs/android-test-checklist.md:76-81` | `[MEASURED — TO BE FILLED]` |

**بوابة الخروج (Exit criteria):**
- كل حارس في 4.1–4.8 يعمل في التجميع الحالي بعد تغييرات المراحل 1–3. ✅
- كل خانة 4.9–4.12 إما مملوءة بقياس مُسجَّل أو فارغة موسومة `[MEASURED — TO BE FILLED]`. ✅
- لا رقم `[MEASURED]` بلا سجل/أمر مُرفق. ✅

---

## 8. المرحلة 5 — الوثائق وتوقيع الجاهزية (Phase 5 — docs & signoff)

**DRI:** Docs/Release owner + Owner (التوقيع النهائي). **لا commit إلا بإذن صريح من المالك.**

| # | الإجراء | المرجع | الوسم |
|---|---|---|---|
| 5.1 | تحديث `docs/final-report-ar.md` و`docs/audio-route-switch-diagnosis-*.md` بنتائج المراحل 1–4 مع نفس منظومة الوسوم. | `docs/final-report-ar.md:7`, `:177-186` | `[SOURCE-VERIFIED]` |
| 5.2 | تحديث `docs/mobile-platform-audit.md`: تحويل IOS/AND من «ملاحظة شيفرة» إلى «مُختبَر/غير مُختبَر على جهاز» صراحةً. | `docs/mobile-platform-audit.md:4-8`, `:133-139`, `:214-219` | `[SOURCE-VERIFIED]` |
| 5.3 | تحديث `docs/android-test-checklist.md` من `PENDING DEVICE RUN` إلى حالة حقيقية، أو إبقائها معلّقة. | `docs/android-test-checklist.md:3-8` | `[MEASURED — TO BE FILLED]` |
| 5.4 | إنتاج ملخّص جاهزية (go / go-with-conditions / no-go) مع قائمة `[HYPOTHESIS]` المتبقية: صوت مسموع على الأجهزة، HLS على الشبكة الحية، GPU/compositor، الطاقة/الحرارة. | `docs/final-report-ar.md:92-102`, `:186` | `[HYPOTHESIS]` |
| 5.5 | تدقيق العقد C1–C8 مرة أخيرة وتوثيق أن composables الجديدة أُضيفت دون تعديل القديم. | القسم 2 + بوابة المرحلة 3 | `[SOURCE-VERIFIED]` |
| 5.6 | تسجيل قرار المالك بالتوقيع/الإرجاع؛ لا يُنشأ commit ولا push ولا PR دون إذن. | قيد المهمة؛ `docs/final-report-ar.md:5` | `[SOURCE-VERIFIED]` |

**بوابة الخروج النهائية (Production-readiness exit criteria):**
- كل بوابة من المراحل 0–4 موقَّعة. ✅
- صفر أرقام `[MEASURED]` مختلقة؛ كل خانة غير مقاسة موسومة. ✅
- صفر كسر للعقد C1–C8؛ صفر تعديل على `src/widget.rs` القديم. ✅
- صفر عمل Linux. ✅
- التغييرات ما زالت غير مُودَعة حتى يأمر المالك بغير ذلك. ✅

---

## 9. مصفوفة المخاطر المتبقية (كلها `[HYPOTHESIS]` حتى القياس)

| الخطر | المرجع | من يغلقه |
|---|---|---|
| ترتيب كروما/لون Android على أجهزة حقيقية | `docs/mobile-platform-audit.md:67,72`, `src/platform/android.rs:791-845`, `:944-953` | DRI المرحلة 1 |
| clean aperture/HDR على iOS | `docs/mobile-platform-audit.md:64-65,86-87`, `src/platform/apple.rs:302-368` | DRI المرحلة 2 |
| زمن تبديل الصوت أقل من 500 ms على جهاز المستخدم | `docs/audio-route-switch-diagnosis-round2-2026-11-20.md:54-56` | DRI المرحلة 4/5 |
| استعادة مسموعة بعد انقطاع AAudio/focus loss | `src/audio.rs:936-939`; `docs/final-report-ar.md:196` | DRI المرحلة 1/4 |
| HLS rollout على شبكة حية، GPU، الطاقة | `docs/final-report-ar.md:70-74,96-102,186` | DRI المرحلة 5 |

---

## 10. ترتيب التنفيذ وتبعياته

```
Phase 0 (gates, no src edits)
   │
   ├── Phase 1 (Android) ─┐
   ├── Phase 2 (iOS) ─────┼──► Phase 4 (test harness) ──► Phase 5 (docs & signoff)
   └── Phase 3 (widget) ──┘
```

- المراحل 1 و2 و3 **مستقلة تقنياً** ويمكن توازيها، لكنها تتشارك بوابة المرحلة 4 ويجب أن تُمرَّر كل منها بوابة الخروج الخاصة بها أولاً.
- أي اكتشاف في المرحلة 1/2 يمسّ `VideoDecoder` يجب أن يمرّ على العقد C4 قبل الدمج في الفرع المنطقي للمرحلة 4.
- لا تُفتح المرحلة 5 إلا بعد توقيع بوابات 0–4 جميعها.

---

**نهاية الخطة.** هذه الوثيقة هي الملف الوحيد المكتوب في هذه المهمة؛ المسار:
`D:\counter\cranpose-video\docs\PRODUCTION-MASTER-PLAN.md`
