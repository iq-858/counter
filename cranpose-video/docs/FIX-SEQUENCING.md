# FIX-SEQUENCING — ترتيب تنفيذ الإصلاحات الواجبة (must-fix)

> **النطاق**: هذا المستند فقط. لم أعدّل أي ملف مصدر، ولم أنفّذ commit/stage/push، ولم أستخدم Linux أو أي أمر جهاز. كل ما كُتب هنا مقروء من الشجرة الحالية في `D:\counter\cranpose-video`.
>
> **الوسوم**: `[VERIFIED]` = مقروء مباشرة من المصدر برقم سطر في هذه الشجرة (لا يعني إثباتاً على جهاز). `[HYPOTHESIS]` = استنتاج منطقي يحتاج تحققاً على جهاز Apple/Android أو على شبكة حيّة. **لا أرقام زمنية ولا نسب مئوية مُختلَقة**: أي رقم مذكور هو رقم سطر أو نص حالة، لا قياس أداء.
>
> **القيد الحاكم**: إصلاح Windows غير المُودَع (`src/platform/windows.rs` + `src/platform/windows/mf_audio.rs`, الموصوف في `docs/audio-route-switch-diagnosis-2026-11-20.md` و`...round2...`) يجب أن يبقى سليماً. لذلك: **لا تلمس** هذين الملفين، ولا تغيّر `Shared::fail` / `resume_after_recovery` / `note_frame` / `maybe_recover` إلا بإضافة لا تعديل، ولا تغيّر دلالة `PlaybackSession::failed()` ولا الـ backoff.

---

## 0. لماذا هذا الترتيب (قاعدة غير كاسرة)

1. **الخطوة 1** تثبت سلامة عقد الاستعادة (`recover()` → قراءة جديدة) الذي يعتمد عليه كل ما يلي — بما فيه إصلاح Windows.
2. **الخطوتان 2–3** تلمسان `player.rs`/`threaded.rs` لكن بإضافة فقط؛ تُنجَز قبل أي تعديل خاص بالمنصات حتى يبقى خطر الانحدار على Windows محصوراً في ملفين منفصلين.
3. **الخطوات 4–6** خاصة بـ Apple/Android؛ لا تُترجَم على مضيف Windows، لذا تُتحقَّق بالمصدر + `cargo check` للهدف، ويبقى الإثبات الجهازي `[HYPOTHESIS]`. تُؤجَّل لأنها لا تلمس مسار Windows إطلاقاً.
4. **الخطوتان 7–8** سطح عام/additive؛ تُنجَز أخيراً لأنها لا تعتمد عليها أي خطوة أخرى، وأي تراجع فيها لا يؤثر على النواة.

**بوابة قبل كل خطوة** (أوامر تحقق محلية، `[VERIFIED]` أنها تُستخدم في هذه الشجرة سابقاً حسب `docs/hardening-audit-plan-2026-11-19.md`):

```
cargo check --release --no-default-features --features native-audio
cargo test  --release --no-default-features --features native-audio --lib
```

وبعد كل خطوة: أعد تشغيل الأمرين + اختبار التكامل الخاص بالخطوة، ثم قارن النتيجة بالسطر/السلوك المتوقع قبل الانتقال. لا تنتقل باختبار أحمر.

---

## Step 1 — `recover()` يُجدول قراءة جديدة ولا يُعلّق حالة `Failed`

**الحالة**: `[VERIFIED]` — الإصلاح موجود أصلاً في الشجرة؛ المطلوب تثبيته كـ invariant لا إعادة كتابته.

**المواقع**:
- `src/player/threaded.rs:273-296` — `maybe_recover()`؛ السطر **288** (`self.refresh = true`) هو المجدول، والسطر **287** (`let _ = source_failure;`) يُسجّل أن البوابة القديمة كانت تُبقي فشل `Decode` على iOS عالقاً في `Failed` مع عامل نائم في `recv()`.
- `src/platform/apple.rs:420-433` — `recover()` يُعيد بناء القارئ عبر `open_at()` (يُصفّر `finished` عند `apple.rs:184`) لكنه **لا** يُصدر قراءة بنفسه.
- `src/player.rs:305-314` (`resume_after_recovery`) و`543-566` (`note_frame`) — مسح `Failed` لا يحدث إلا بعد إطار حقيقي.
- الاختبار الحالي: `src/player/threaded.rs:1060-1125`.

**شكل التصحيح الأدنى (لا كود كامل)**: لا تغيير في المصدر. أضف تعليق حارس قصير عند `threaded.rs:288` يربط `refresh` بـ "أي نجاح لبناء القارئ، بغض النظر عن نوع الفشل السابق"، وأبقِ `source_failure` مُستهلكاً (`let _ = ...`).

**الاختبار المضاف**: حالة اختبار جديدة في `threaded.rs` بنفس نمط `a_transient_decode_failure_returns_to_playing_after_recovery`: مُفكّك يُرجع `Err(VideoError::Decode(...))` (ليس `Source`) ثم `recover() == true` ثم إطار — يُثبت `Failed → Recovering → إطار → Playing` و`worker.refresh == true` بعد `maybe_recover()`. هذا يقيّد تحديداً حالة iOS التي ذكرها تعليق السطر 284.

**الخطر**: إعادة `refresh = true` في مكان آخر (مثلاً داخل `apple.rs::recover()`) ستُجدول قراءة على خيط خاطئ أو مزدوجة؛ لا تفعل ذلك.

**التراجع**: حذف التعليق/الاختبار الجديد فقط؛ لا يوجد تعديل مصدر لتراجعه.

---

## Step 2 — HLS: إعادة نشر `live`/`seekable` بعد التحميل (snapshot قبل التحميل كان متصلاً)

**الحالة**: `[VERIFIED]` أن الخلل موجود؛ `[HYPOTHESIS]` أن أثره على واجهة التطبيق (live/seek UI) سيختفي بعد الإصلاح — لا توجد شبكة حيّة هنا.

**المواقع**:
- `src/player.rs:865-882` — `from_decoder()` يأخذ `live`/`seekable` من `decoder.is_live()`/`is_seekable()` **قبل** بدء خيط فك الترميز ويخزّنهما في `Shared` (السطر 882).
- `src/hls/mod.rs:345-405` — `Stream::new()` يضع `live: false` (السطر 366) و`seekable: true` (السطر 369) كقيم متفائلة قبل التحميل.
- `src/hls/mod.rs:413-437` — `ensure_loaded()` يقلبهما عند السطرين **434-435** (`self.live = live; self.seekable = !live;`) على خيط فك الترميز، لكن `Shared` لا يُحدَّث.
- `src/player.rs:141-169` — الحقلان داخل `Mutex<Inner>`، قابلان للتحديث بأمان.
- `src/player/threaded.rs:438-443` — نقطة `adopt_info` بعد أول إطار، مكان مناسب للنشر.
- النتيجة الخاطئة اليوم: `player.is_live() == false` و`player.is_seekable() == true` لبثّ حيّ؛ `player.rs:996-1006` يسمح بإعادة تشغيل live بعد `Ended`، و`Command::Stop` يُرسل على live.

**شكل التصحيح الأدنى**:
- أضف `pub(crate) fn adopt_capabilities(&self, live: bool, seekable: bool)` إلى `Shared` بجوار `adopt_info` (`player.rs:521`)؛ يكتب الحقلين تحت القفل الحالي، بدون `emit` وبدون لمس `state`/`recovering`.
- في `Worker::step()` عند `threaded.rs:438-443` (بعد `adopt_info`) استدعِ الطريقة بقراءة `self.decoder.is_live()` / `is_seekable()`؛ اجعلها idempotent (استدعاء متكرر بلا أثر) لتفادي مشكلة تعدد المكالمات.
- **لا** تغيّر `from_decoder()` ولا قيم `Stream::new()`: اللقطة قبل التحميل متعمّدة لمنع الشبكة على خيط التركيب (`hls/mod.rs:340-344`)، والصحيح هو تحديثها بعد التحميل لا إلغاؤها.

**الاختبار المضاف**: في `src/hls/mod.rs` اختبار يبني `Stream` بـ playlist محلي مزيف (`ended = false`)، يتحقق أن `is_live()/is_seekable()` قبل `ensure_loaded()` هما `false/true`، وبعده `true/false`؛ ثم اختبار في `player.rs` أو `tests/` يستخدم مُفكّكاً يُغيّر `is_live()` بعد أول `read()` ويُثبت أن `player.is_live()` و`is_seekable()` يتقاربان من القيم الجديدة بعد الإطار، وأن `stop()` على live يُرجع `UnsupportedOperation` (`player.rs:1041-1047`).

**الخطر**: نشر القيم قبل أول إطار قد يجعل `play()` يرفض seek على ملف عادي؛ لذلك الربط بأول إطار (نقطة `adopt_info`) إلزامي. خطر آخر: أي تعديل لـ `Shared::fail` أثناء إضافة الطريقة قد يكسر إصلاح Windows — لا تلمسه.

**التراجع**: احذف `adopt_capabilities` والاستدعاء عند `threaded.rs:443`؛ يعود السلوك إلى اللقطة المتفائلة السابقة.

---

## Step 3 — `is_closed` يصبح `true` بعد panic في خيط فك الترميز

**الحالة**: `[VERIFIED]` من المصدر: التعليق يقول "أو خيط فك الترميز ذهب" لكن التنفيذ يقرأ علماً واحداً فقط.

**المواقع**:
- `src/player.rs:976-979` — `is_closed()` يقرأ `self.closed` فقط.
- `src/player.rs:782-787` — حقل `closed: AtomicBool`؛ يُضبط في `close()` عند `1306-1309` فقط.
- `src/player.rs:1341-1347` — `check()` يقرأ نفس العلم.
- `src/player/threaded.rs:167-183` — `Worker::run()` يمسك الـ panic عبر `catch_unwind`، يُبلّغ `shared.fail(...)`, يغلق المُفكّك، ثم ينتهي الخيط **دون** إعلام `VideoPlayer`.
- `src/player/threaded.rs:111-122` — `Engine::shutdown()` يُسقط `JoinHandle` (detach)، فلا يمكن استعلام `is_finished()`.
- `tests/panic_boundary.rs:94-147` — اختبارات الـ panic الحالية تتحقق من `Failed` والحدث، لا من `is_closed`.

**شكل التصحيح الأدنى (additive)**:
- أضف `done: Arc<AtomicBool>` إلى `Engine` (`threaded.rs:60-63`) و`Worker` (`threaded.rs:125-155`)، تُنشأ في `Engine::start()` (`threaded.rs:66-102`) وتُمرَّر بالاستنساخ.
- في نهاية `Worker::run()` (بعد `catch_unwind` الثاني عند `threaded.rs:176-182`) اضبط `done = true`.
- في `VideoPlayer` (`player.rs:782-787`) احتفظ بالعلم نفسه، واجعل `is_closed()` (`977-979`) يُرجع `closed || done`، و`check()` (`1341-1347`) يستخدم نفس التعبير حتى تُرفض الأوامر بعد موت الخيط بـ `VideoError::Closed`.
- **لا** تغيّر `Engine::shutdown()` ولا منطق الإغلاق؛ detach مقصود لمنع انتظار read عالق.

**الاختبار المضاف**: في `tests/panic_boundary.rs` أضف اختباراً يستخدم `PanickingDecoder::new(PanicIn::Read)`، ينتظر حتى يظهر `Failed`، ثم ينتظر `player.is_closed() == true`، ثم يتحقق أن `player.play()` يُرجع `VideoError::Closed`. لا يُضف أي توقيت مُخترَع؛ استخدم نفس `wait_until` الموجود (السطر 76-85).

**الخطر**: ضبط `done` عند الخروج الطبيعي بعد `close()` لا يغيّر شيئاً لأن `closed` مضبوط أصلاً؛ الخطر الحقيقي هو قراءة `done` قبل نشر `Failed` — ترتيب الضبط بعد `shared.fail` في `run()` يمنعه.

**التراجع**: استبدل تعبير `closed || done` بـ `closed` وأزل الحقل والاستدعاء؛ لا تلمس `Worker::run()`.

---

## Step 4 — Apple: فشل `attach` يجب أن يُبقي التشغيل (لا `Ok(None)` صامت)

**الحالة**: `[VERIFIED]` من المصدر؛ الزمن/السلوك على جهاز `[HYPOTHESIS]`.

**المواقع**:
- `src/platform/apple.rs:174-199` — `open_at()`.
- السطران **179-182**: `self.playback.take()` ثم `cancelReading()` **قبل** محاولة البناء الجديد.
- السطران **190-195**: `attach(Nv12)` ثم `attach(Rgba8)?`؛ إذا فشلا معاً يخرج `?` و`self.playback` يبقى `None`.
- السطران **459-461**: `pull()` يُرجع `Ok(None)` عندما `playback == None` — أي نهاية/توقف صامت بدل خطأ قابل لإعادة المحاولة.

**شكل التصحيح الأدنى**:
- انقل `take()`/`cancelReading()` إلى **بعد** نجاح البناء: ابنِ `Playback` الجديد في متغير محلي أولاً، ثم استبدل القديم وألغِ قراءته.
- قبل المحاولة، احفظ لقطة `start` و`finished` (السطران 183-184) وأعدها عند الفشل، حتى لا يتقدّم موضع البدء بعد محاولة فاشلة.
- عند فشل كلا التنسيقين: اترك `self.playback` القديم قائماً وأعد `Err`; لا تُسقطه. إن لم يكن هناك قديم، اترك `None` لكن اجعل `pull()` يُبلّغ فشلاً `(VideoError::Decode)` بدل `Ok(None)` في هذه الحالة (ميّز "لم يُفتح بعد" عن "فشل الفتح").
- لا تغيّر `recover()` (`apple.rs:420-433`) — Step 1 ضمن أن جدولة القراءة تتم في العامل.

**الاختبار المضاف**: اختبار وحدة في `apple.rs` (غير قابل للتشغيل على مضيف Windows — `[HYPOTHESIS]` حتى يُشغَّل على macOS) بمُفكّك/قارئ وهمي يُجعل `attach` يفشل للمرة الأولى ثم ينجح؛ يتحقق أن `open_at()` أعاد `Err` مع بقاء `playback` القديم، وأن `pull()` بعد `recover()` يُنتج إطاراً. يمكن تغطية جزء "لا `Ok(None)` صامت" باختبار عام في `threaded.rs` بنمط `a_live_recovery_without_a_picture_rearms_instead_of_ending` (`threaded.rs:1158`).

**الخطر**: إبقاء قارئ قديم فاشل قد يُعيد إطارات قديمة بعد seek؛ لذلك يجب إلغاء القديم فور نجاح الجديد، ولا يُعاد إلا عند فشل كل المحاولات. خطر ثانٍ: تغيير `pull()` قد يحوّل نهاية ملف حقيقية إلى خطأ — ميّز حالة `finished == true` (السطر 443) عن "الفشل قبل الفتح".

**التراجع**: أعد ترتيب السطور 179-198 إلى الحالة الأصلية (take/cancel أولاً) وأزل لقطة `start`/`finished`؛ أعد `pull()` إلى `Ok(None)` عند `playback == None`.

---

## Step 5 — Apple/Android: الفتح الحاجز خارج خيط التركيب (composition thread)

**الحالة**: Apple `[VERIFIED]` أنه حاجز؛ Android `[VERIFIED]` أنه lazy أصلاً (لا إجراء مطلوب سوى الحماية).

**المواقع**:
- `src/player.rs:799-806` — `VideoPlayer::open()` يستدعي `crate::platform::open(&source)` على خيط الاستدعاء (خيط التركيب).
- `src/platform/mod.rs:121-133` — الموجّه يمرّر إلى `open_native`.
- `src/platform/apple.rs:54-105` — `open()` ينفّذ `TempFile::write` (السطر 56)، `AVURLAsset::initWithURL_options` (64)، `tracksWithMediaType` (70)، `naturalSize`/`preferredTransform`/`nominalFrameRate`/`duration` (74-80)، ثم `open_at(ZERO)` (103) الذي يبدأ الـ `AVAssetReader` على نفس الخيط.
- `src/platform/apple.rs:372-390` — `info()`/`output_format()` رخيصان نسبياً لكنهما يعتمدان على قيم محسوبة في `open()`.
- `src/player/threaded.rs:438-443` — `adopt_info` بعد أول إطار يسمح بنشر المقاس الحقيقي لاحقاً.
- `src/platform/android.rs:89-95` — `open()` يُرجع `Reader::new(source.clone())` فقط؛ التعليق صراحةً يقول إن JVM و MediaExtractor/MediaCodec يحدثان لاحقاً على خيط فك الترميز. **لا تغيير هنا** — أضف تعليق حارس فقط.

**شكل التصحيح الأدنى (Apple)**:
- احذف الاستدعاء `reader.open_at(Duration::ZERO)?` من نهاية `open()` (السطر 103)؛ اترك `playback: None`.
- في `pull()` (`apple.rs:438-461`) استبدل العائد الصامت عند `playback == None` بـ `self.open_at(self.start)?` مرة واحدة ثم تابع الحلقة — أي أن أول قراءة تبني القارئ على خيط فك الترميز. (هذا يكمل Step 4: نفس نقطة التفرّع "لم يُفتح بعد" مقابل "فشل الفتح").
- لـ `VideoSource::Bytes` انقل `TempFile::write` إلى نفس اللحظة؛ لغيره احتفظ بـ `NSURL` خفيف. اجعل `info()` يُرجع `VideoInfo` بـ width/height = 0 قبل الفتح (`player.rs:193` يتعامل معه كـ `Loading`) واترك `adopt_info` ينشر المقاس بعد الإطار.
- لا تغيّر التوقيعات العامة ولا `VideoPlayer::open()`؛ المُفكّك وحده يصبح كسولاً (lazy).

**الاختبار المضاف**: اختبار في `apple.rs` يُثبت أن `open()` لا يستدعي `startReading` (عدّاد وهمي) وأن أول `pull()` يفعل ذلك؛ واختبار عام في `tests/first_play.rs` يؤكد أن `VideoState::Loading` ثم `Playing` يبقى سليماً بعد تأخير الفتح. على Android: لا اختبار جديد، فقط تأكيد بالمصدر أن `open()` لا يستدعي `vm()`.

**الخطر**: `output_format()` قبل الفتح يُرجع `PixelFormat::Nv12` الافتراضي بينما قد يختار `open_at` لاحقاً `Rgba8`؛ العامل يقرأ `output_format()` قبل كل `read()` (`threaded.rs:340`) ولديه مسار `native_refused` (`threaded.rs:403-413`) — تحقق من هذا الانتقال في الاختبار. خطر ثانٍ: تأخير `Loading` أطول على live؛ لا يوجد قياس هنا، `[HYPOTHESIS]`.

**التراجع**: أعد `reader.open_at(Duration::ZERO)?` إلى `open()` وأعد `pull()` إلى `Ok(None)` عند غياب `playback`.

---

## Step 6 — Android: إزالة التبليغ المزدوج للفشل (double-report)

**الحالة**: `[VERIFIED]` — مصدران يُبلّغان عن نفس `VideoError::Source` في نفس generation.

**المواقع**:
- `src/platform/android.rs:312-324` — `report_setup_failure()` يستدعي `session.failed("android.recover", reason)` عند السطر **322**.
- `src/player.rs:485-506` — `Shared::fail()` يستدعي `self.playback.failed("decoder.read", reason)` عند السطر **504** عندما يكون الخطأ `transient`.
- `src/error.rs:50-52` — `is_transient() == matches!(Self::Source(_))`؛ و`android.rs:1659-1661` (`source_error`) يُنتج `Source` عند فشل إعداد JNI.
- `src/session.rs:269-292` — كل `failed()` يزيد `phase.failures` ويضاعف الـ backoff ويضيف حدث `Failed`.
- النمط المكافئ المُصلَح على Windows موثّق في `docs/audio-route-switch-diagnosis-round2-2026-11-20.md:32` (مصدر واحد فقط يُبلّغ الجلسة).

**شكل التصحيح الأدنى (في `android.rs` وحده — حمايةً لـ Windows)**:
- في `report_setup_failure()` (`android.rs:312-324`) احتفظ بـ `crate::diag` والتحقق من generation، ثم **تخطَّ** `session.failed(...)` عندما يكون `error.is_transient() == true`؛ في هذه الحالة `Shared::fail` هو المبلّغ الوحيد (`decoder.read`). اترك `failure_reported_for` كما هو للفشل غير الـ transient (`Decode`) الذي لا يُبلَّغ من `Shared::fail`.
- بديل مكافئ إن فُضِّل عنصر تحكم صريح: أضف وسيط `report: bool` إلى `ensure_ready` (`android.rs:256`) ومرّره `false` من مسار `read` و`true` من مسار `recover` — لكن هذا يلمس أكثر من سطر، والأول أدنى خطراً.

**الاختبار المضاف**: اختبار وحدة في `android.rs` (تحت `cfg(target_os = "android")` — غير قابل للتشغيل هنا، `[HYPOTHESIS]`) يحسب عدد أحداث `SessionEvent::Failed` بعد فشل `Source` واحد ويُثبت أنه **واحد**، وأن `PlaybackSession::state() == Recovering`. يمكن محاكاة المنطق على المضيف عبر اختبار في `src/session.rs` يُثبت أن استدعاء `failed()` مرتين في نفس generation يضاعف العدّ (يبرر الحاجة) — دون تغيير `failed()` نفسه.

**الخطر**: إذا لم يصل الخطأ إلى `step()` (مثلاً أُبلِغ داخل `close`) فلن يُبلَّغ أحد؛ لكن `report_setup_failure` محمي أصلاً بـ `!gate.stopped()` (`android.rs:301`)، فتلك الحالات مستبعدة. لا تلمس `src/session.rs` ولا `src/player.rs` في هذه الخطوة.

**التراجع**: أعد سطر `session.failed("android.recover", reason)` وإزالة شرط `is_transient()`؛ يعود التبليغ المزدوج.

---

## Step 7 — كشف المسار على الموبايل: خطاف عام additive + مراقب جهة التطبيق

**الحالة**: `[VERIFIED]` أن `PlaybackSession::route_changed()` و`AudioFocusLoss` معرَّفان بلا مُستدعٍ في هذه الشجرة؛ `[HYPOTHESIS]` أن مراقب التطبيق سيتصل بالخطاف (لا يمكن تعديل التطبيق من هنا).

**المواقع**:
- `src/session.rs:247-261` — `route_changed()` عامة وتُدرج `SessionEvent::RouteChanged` وتُسلّح محاولة تحقق واحدة.
- `src/session.rs:65-75` — `InterruptionReason::{AudioFocusLoss, RouteChanged, DeviceLost, ...}`.
- `src/session.rs:226-238` — `interrupted(reason)` تُسلّح retry أيضاً.
- `src/player.rs:1186-1207` — الأسطح العامة الحالية: `playback_state()` و`recovery_events()`؛ لا يوجد مُدخل عام للإبلاغ.
- `src/player.rs:276` — `playback_session()` `pub(crate)` فقط؛ `lib.rs:49-53` يصدّر `PlaybackSession`/`InterruptionReason` لكن التطبيق لا يملك مقبض الجلسة.
- `src/audio.rs:355-405` — `observe_output_route`/`current_output_route` موجودان للغلاف العام فقط، وليس للمسار الأصلي الموبايل.

**شكل التصحيح الأدنى (additive، بلا تعديل توقيعات قائمة)**:
- أضف إلى `VideoPlayer` بجوار `playback_state()` (`player.rs:1193-1196`) طريقتين عامتين مُقيَّدتين بنفس الـ cfg (`not(linux)` و`not(wasm32)`):
  - `pub fn note_route_changed(&self)` → `self.shared.playback_session().route_changed()`.
  - `pub fn note_interruption(&self, reason: InterruptionReason)` → `self.shared.playback_session().interrupted(reason)`.
- لا تغيّر `SessionEvent` ولا `PlaybackState` ولا `lib.rs` (الأنواع مصدَّرة أصلاً).
- **خارج نطاق هذه الشجرة**: مراقب التطبيق (AudioManager `AUDIO_DEVICE_CHANGE` / `AudioFocus` على Android، `AVAudioSession.routeChangeNotification` على iOS) يستدعي هاتين الطريقتين على الـ `VideoPlayer` الذي يملكه. لا تعديل تطبيق هنا؛ سجَّل هذا كـ تبعية `[HYPOTHESIS]` للمُتكامل.

**الاختبار المضاف**: اختبار في `player.rs`/`tests/` يستدعي `note_route_changed()` على مشغّل بـ `BareDecoder` ويُثبت `playback_state() == Recovering`، و`recovery_events()` يحتوي `RouteChanged`، و`next_retry()` ليس `None`؛ واختبار ثانٍ لـ `note_interruption(AudioFocusLoss)` يُثبت `Interrupted` و`interruption_reason() == Some(AudioFocusLoss)`. هذه اختبارات مضيف قابلة للتشغيل هنا `[VERIFIED]` بالمنطق، لكن وصول الإشارة الحقيقية `[HYPOTHESIS]`.

**الخطر**: استدعاء `route_changed()` بشكل متكرر من المراقب يُسلّح محاولات متعددة؛ `session.rs:256-258` تمنع تكرار `retry_at` لكنها تضيف حدثاً لكل استدعاء — لذلك على جهة التطبيق تجميع الإشارات المتقاربة (coalescing) مطلوب، ويُوثَّق ولا يُنفَّذ هنا.

**التراجع**: احذف الطريقتين؛ لا يبقى أثر لأن الجلسة نفسها لم تتغير.

---

## Step 8 — widget: عناصر `stop`/`mute`/`volume` إضافية

**الحالة**: `mute`/`volume` على المقبض `[VERIFIED]` موجودة أصلاً؛ `stop` على المقبض `[VERIFIED]` مفقودة؛ أزرار `VideoControls` `[VERIFIED]` تفتقر للثلاثة.

**المواقع**:
- `src/widget.rs:39-150` — `VideoPlayerHandle`؛ `play/pause/toggle/seek/set_speed/set_looping` موجودة، **`stop()` غير موجودة** بين `pause()` (`54-56`) و`toggle()` (`59-61`).
- `src/widget.rs:92-111` — `set_muted`/`is_muted`/`set_volume`/`volume` موجودة (مقبض mute/volume لا يحتاج إضافة).
- `src/widget.rs:191-272` — `VideoControls`; الصفّ من `216-268` يحمل `play / pause`, `-5 s`, `+5 s`, `restart`, speed فقط.
- `src/player.rs:1041-1053` — `VideoPlayer::stop()` موجود ويرجع `UnsupportedOperation` على live.
- `tests/widget.rs:24-30` — اختبار تركيب `VideoControls` الحالي.

**شكل التصحيح الأدنى (additive فقط)**:
- أضف `pub fn stop(&self) -> Result<(), VideoError>` إلى `VideoPlayerHandle` بجوار `pause()` (`widget.rs:56`) يُفوّض إلى `VideoPlayer::stop()` — لا تغيير لأي توقيع قائم.
- لا تعديل على طرق mute/volume؛ أضف تعليقاً قصيراً أنها تُدار من المقبض لا من الأزرار الجاهزة.
- للأزرار: **فضَّل** composable جديد مستقل (مثلاً `VideoAudioControls(handle, modifier)`) يحمل `mute` و`volume` و`stop` بدل تعديل `VideoControls`، حتى لا يتغير تخطيط الواجهة الجاهزة لأي تطبيق حالي. إن رُغب في إضافتها داخل `VideoControls`، فلتكن إضافات في نهاية الصف (`widget.rs:265` قبل الإغلاق) مع عدم حذف أي زر موجود.
- صدّر الاسم الجديد من `lib.rs:57-61` (إضافة فقط إلى قائمة `pub use`).

**الاختبار المضاف**: في `tests/widget.rs` أضف اختبار تركيب للـ composable الجديد بنفس نمط `ready_made_controls_compose_from_an_existing_handle`، واختباراً يتحقق أن `VideoPlayerHandle::stop()` يُفوّض (باستخدام مشغّل وهمي أو التحقق من نتيجة `Err` على `VideoState` غير القابل للفتح). لا قياسات بصرية هنا.

**الخطر**: إضافة أزرار إلى `VideoControls` تغيّر ارتفاع/ترتيب الواجهة الجاهزة؛ لذلك المسار الموصى به هو composable منفصل. خطر ثانٍ: `stop()` على live يُرجع `UnsupportedOperation` (`player.rs:1043-1047`) — يجب ألا يفترض الزر نجاحه؛ عالج `Err` كما في بقية الأزرار (`let _ = ...`).

**التراجع**: احذف composable الجديد واستيراد `lib.rs` وطريقة `stop()`؛ `VideoControls` تبقى كما هي إن لم تُلمَس.

---

## ملخص الترتيب (gate نهائي)

| # | الخطوة | الملفات | قابل للتحقق هنا |
|---|--------|---------|-----------------|
| 0 | بوابة Windows غير المُودَع | `windows.rs`, `mf_audio.rs` (قراءة فقط) | نعم — لا تعديل |
| 1 | `recover()` → قراءة جديدة | `threaded.rs`, `player.rs`, `apple.rs` | نعم |
| 2 | HLS live/seekable بعد التحميل | `hls/mod.rs`, `player.rs`, `threaded.rs` | نعم (منطق + اختبار Offline) |
| 3 | `is_closed` بعد panic | `player.rs`, `threaded.rs`, `tests/panic_boundary.rs` | نعم |
| 4 | Apple failed attach | `apple.rs` | مصدر + cross-check فقط |
| 5 | Apple/Android open خارج التركيب | `apple.rs`, `android.rs` (حارس) | مصدر + cross-check فقط |
| 6 | Android double-report | `android.rs` | مصدر + cross-check فقط |
| 7 | خطاف المسار العام | `player.rs`, `lib.rs` | نعم (المنطق)؛ المراقب `[HYPOTHESIS]` |
| 8 | widget stop/mute/volume | `widget.rs`, `lib.rs`, `tests/widget.rs` | نعم |

**التحقق الختامي المقترح** (بدون أرقام مُخترَعة — يُقرأ من المخرجات الفعلية):

```
cargo check --release --no-default-features --features native-audio
cargo test  --release --no-default-features --features native-audio --lib
cargo test  --release --test panic_boundary --test first_play --test widget
cargo check --target aarch64-apple-ios
cargo check --target aarch64-linux-android
```

ثم تأكيد أن `windows.rs` و`mf_audio.rs` لم يتغيّرا، وأن `maybe_recover` ما زال يُبقي `self.refresh = true` عند كل `recover() == true`. أي سلوك على جهاز حقيقي (تبديل سماعة، route change، زمن الانتقال) يبقى `[HYPOTHESIS]` إلى أن يقيسه المستخدم.
