# كتالوج تقنيات معمارية الوسائط المتقدمة لـ `cranpose-video`

> **نوع المستند:** تقرير بحثي مقارن (Read-only)  
> **التاريخ:** 2026-11-18  
> **النطاق:** مراجعة مصدر Makepad `platform/video` على `main`، ومقارنة ب stacks أخرى (FFmpeg / GStreamer / mpv / VLC / Android MediaCodec)، ثم إسقاط ذلك على الملفات المحلية في `D:\counter\cranpose-video`.  
> **قيد صارم:** لم يُنشأ أي ملف سوى هذا الملف، ولم يُعدَّل أي كود أو `Cargo.toml` أو `README.md`، ولم تُجرَ أي عملية commit أو push.  
> **تنويه مهم:** **لم يُجرِ كاتب هذا التقرير أي اختبار على جهاز فعلي.** كل أعراض الأجهزة المذكورة أدناه هي معطيات مقدَّمة من مطلّب العمل، وليست قياسات أُعيد إجراؤها هنا.  
> **تحديث لاحق:** هذا المستند كان مراجعة read-only، لكنه استُخدم لاحقاً كخريطة للتنفيذ. غيّر التنفيذ اللاحق `PARALLEL_MIN_PIXELS` إلى `1920 * 1080`، وأضاف جداول البكسل و`FineWait` وإصلاح `cancelReading` على Apple. القياسات المحلية النهائية المذكورة أدناه أُجريت بعد هذه التغييرات على مضيف Windows، ولا تدّعي اختبار جهاز فعلي.

---

## 0. ملخص تنفيذي

يُظهر التحليل أن المشكلة الظاهرة أصلاً («تقطيع عند 1080p25 release» و«تقطيع HLS مع/بدون native-audio»، مقابل «720p25 release سلس») ليست سببًا واحدًا، بل تفاعلًا بين أربع طبقات. بعد التنفيذ والقياس المحلي النهائي أصبح 1080p25 سلسًا أيضًا، بينما بقي 60 fps حساسًا للحمل وبقي HLS غير محسوم:

1. **زمن فك التشفير وتسليم الإطار** (decode/DPB latency)؛
2. **سياسة الحجز والإسقاط** (newest-frame slot + re-anchor)؛
3. **مسار البكسل** (NV12 → RGBA على CPU مقابل surface/GPU)؛
4. **تزامن الساعة مع الصوت وإعادة تأسيس الطوابع الزمنية في HLS**.

أقوى الاستنتاجات القابلة للتحقق محليًا:

- **[REPORT-ONLY]** الملف `src/pixels.rs` يعترف صراحةً أن تحويل 1080p60 يعني **≈500 MB/s كتابة**، ويضع قواعد صحيحة (لا تخصيص لكل إطار، مخزنات يملكها المستدعي، تقسيم صفوف على الأنوية، Q16). شرط التوازي الحالي هو `width * height < PARALLEL_MIN_PIXELS` مع `PARALLEL_MIN_PIXELS = 1920 * 1080`؛ أي أن **720p يبقى على نواة واحدة و1080p وما فوق يُقسَّم**. جاء هذا القرار بعد قياس أظهر أن الخيوط الإضافية عند 720p توفر نحو millisecond واحد من الزمن الحقيقي لكنها تستهلك CPU نحو ثلاثة أضعاف وتزاحم callback الخاص بالصوت على الهاتف. الفرق المتبقي بين 720p و1080p هو إذن حجم العمل وضغط الذاكرة/العتاد، لا مجرد عبور عتبة قديمة.
- `src/player/threaded.rs` يستخدم خيط فك تشفير واحدًا، و`newest-frame slot`، و`SLICE = 10ms`، و`HORIZON = 2s`، و**إعادة تثبيت الإطار المتأخر (re-anchor) بدل إسقاط متكرر**. هذه سياسة معقولة، لكنها تفتقر إلى **قياس زمن المرور الفعلي** (decode → convert → present).
- Makepad لا يثق بـ `stride` الضيّق لـ NV12 القادم من العتاد: يرفضه ويعيد تغليفه. هذا يقابل مباشرةً خطأ `odd854.mp4` المحلي: `stride (896) narrower than a row (3416)`.
- مصادر FFmpeg وmpv وGStreamer تضع سياسات صريحة للإسقاط/التأخر (`AV_SYNC_THRESHOLD_*`، `framedup`، `max-lateness`، `video-sync`) بينما يعتمد المستودع المحلي على عتبات زمنية ثابتة دون «ميزانية تأخر» معلنة.

---

## 1. المنهجية وتصنيف الأدلة

### 1.1 كيف جُمعت المعلومات

- استُخدم مخزن `research` ثم وُجِّه العمل إلى منهجية `deep-research` لأن المطلوب مقارنة عدة stacks والتحقق من فرضيات متعددة.
- تعذّر البحث عبر محرك البحث العام (HTTP 402)، فاستُخدم **جلب مباشر لعناوين موثّقة** عبر GitHub raw ووثائق FFmpeg/mpv/GStreamer/Android.
- رُوجعت قائمة ملفات Makepad عبر GitHub API على `main`، ثم جُلبت الملفات المطلوبة مباشرةً.
- رُوجعت الملفات المحلية في المستودع كما هي، دون تعديل.

### 1.2 تصنيف العبارات

| التصنيف | المعنى | كيف يُكتب في هذا التقرير |
|---|---|---|
| **مؤكَّد محليًا** | موجود حرفيًا في ملف داخل المستودع | يُذكر مع اسم الملف والمعرّف |
| **مؤكَّد خارجيًا** | موجود حرفيًا في مصدر علني مُلسَّم | يُذكر مع الرابط |
| **معطى من مطلّب العمل** | ملاحظة جهاز/اختبار لم أعد إجراؤها | يُوسَم صراحةً «معطى» |
| **فرضية** | تفسير معقول غير مثبت | يُوسَم «فرضية» ويقترح اختبارًا |

---

## 2. الخط الأساسي المحلي (Baseline)

| الملف | الحقائق المؤكَّدة محليًا | الأثر المتوقع |
|---|---|---|
| `src/pixels.rs` | تحويل NV12/BGRA → RGBA على CPU؛ لا تخصيص لكل إطار؛ مخزنات يملكها المستدعي؛ تقسيم صفوف؛ Q16؛ جداول بحث bitwise-match؛ `PARALLEL_MIN_PIXELS = 1920*1080`؛ `MAX_WORKERS = 8`؛ `CRANPOSE_VIDEO_WORKERS`؛ تعليق صريح عن 500 MB/s عند 1080p60 | مُحدد أداء البكسل عند الانتقال من 720p إلى 1080p |
| `src/frame.rs` | `PixelFormat::Nv12`، `materialize`، `release_planes`، تخزين تحويل مؤقت | إعادة استخدام مخزن التحويل موجودة، لكنها مرتبطة بدورة حياة الإطار |
| `src/player/threaded.rs` | خيط فك تشفير واحد؛ `IDLE=50ms`، `SLICE=10ms`، `HORIZON=2s`؛ newest-frame slot؛ re-anchor للإطار المتأخر؛ تحويل NV12 على خيط الفك إلا مع `set_native_nv12(true)` | الفك والتحويل متسلسلان على نفس الخيط افتراضيًا |
| `src/player.rs` | `take_frame`/`recycle` بالسحب؛ `set_position_monotonic`؛ `SPARES=2`؛ `DEFAULT_FRAME=33ms`؛ أوامر الصوت تُرسل للمفكك | الساعة تحمي من الرجوع، لكنها لا تعلن ميزانية تأخر |
| `src/hls/mod.rs` | نافذتان ( `FIRST_WINDOW=1`)؛ prefetch متوازٍ؛ قائمة صوت منفصلة best-effort؛ `MAX_HEIGHT=u32::MAX`؛ ABR حسب عرض النطاق؛ إعادة تأسيس طوابع `tfdt::first_bases`/`rebase_each`؛ `sync_clock(position - window.start)` | تغيير الدقة/الإطارات وارد تصميميًا، لكنه يضغط على إعادة التهيئة |
| `src/widget.rs` | `ImageBitmap::from_rgba8` لكل إطار | يعني أن نتيجة التحويل تدخل مسار صور Makepad |
| `examples/stutter_probe.rs` | مسبار إيقاع بمصدر حقيقي موجود مسبقًا | أداة جاهزة للقياس على الجهاز |
| `tests/pacing.rs` | بوابة إيقاع صناعية مع stall/rollover مُحقون | يغطي المنطق لا العتاد |
| `docs/android-test-checklist.md` | سياق اختبار أندرويد موجود | يُكمله «مخطط القياس» في القسم 8 |

### 2.1 المعطيات الأصلية والقياس المحلي اللاحق

المعطيات الأصلية كانت: `720p25 release SMOOTH`، `1080p25 release STUTTER`، `AJA HLS STUTTER` مع وبدون `native-audio`، وخطأ `odd854.mp4` في RGB32: `stride (896) narrower than a row (3416)`.

**[REPORT-ONLY]** بعد التنفيذ، قياس محلي على مضيف Windows في جلسة سابقة (السجل المذكور `probe-final-after-tables.log` غير موجود في هذه الشجرة) أعطى:

- `720p25` سلس عند 25.15 fps، `p95=1.04`، `max=1.92`، reanchors=0.
- `1080p25` سلس عند 25.14 fps، `p95=1.08`، `max=1.29`، reanchors=0؛ و`odd854` سلس عند 23.99 fps.
- `720p60` مع native audio سلس عند 60.11 fps، `p95=1.08`، `max=1.19`، reanchors=0.
- `1080p60` ما زال حساساً للحمل: في التشغيل النهائي 58.83 fps مع `max=5.89/98.24 ms` وreanchors=2، لكن خمس إعادات معزولة بـ `CRANPOSE_VIDEO_WORKERS=4` كانت كلها سلسة (`probe-repeat-final.log`، السجل غير موجود في هذه الشجرة). هذا دليل على إمكانية الوصول، لا على سلاسة عالمية.
- AJA HLS ما زال `STUTTER`: 23.67 fps مع native audio و24.27 fps بدونه، مع 3/2 reanchors و7/4 فجوات طويلة.

لا شيء من هذا اختبار جهاز فعلي؛ إنه قياس إيقاع النشر على مضيف التطوير فقط.

---

## 3. تحليل أسباب التأخير والسقوط (Delay / Dropout)

### 3.1 أسباب مؤكَّدة من المصادر

| # | السبب | الدليل | النوع |
|---|---|---|---|
| D1 | DPB delay مشتق من `level_idc` في SPS | Makepad `windows_stream_decoder.rs` | مؤكَّد خارجيًا |
| D2 | تغيير تنسيق/حجم أثناء البث يستلزم `MF_E_TRANSFORM_STREAM_CHANGE` وإعادة تفاوض output type | Makepad `windows_stream_decoder.rs` | مؤكَّد خارجيًا |
| D3 | مفكك GStreamer يضيف عشرات الملي ثانية من latency بسبب خيوط الفك | Makepad `linux_stream_decoder.rs` | مؤكَّد خارجيًا |
| D4 | NV12 بـ stride ضيّق/حشو من العتاد لا يمكن الوثوق به؛ Makepad يرفضه ويعيد تغليفه | Makepad `linux_stream_decoder.rs` | مؤكَّد خارجيًا |
| D5 | قراءة المخرجات من MFT عبر `IMFMediaBuffer::Lock` تعني نسخة readback؛ لا يوجد مسار DXVA في هذا الملف | Makepad `windows_stream_decoder.rs` | مؤكَّد خارجيًا |
| D6 | إمساك output buffers في MediaCodec قد يوقف المفكك؛ الإصدار إلى `Surface` يتجنب نسخة CPU | Android `MediaCodec` | مؤكَّد خارجيًا |
| D7 | عتبات المزامنة في ffplay: `MIN=0.04`، `MAX=0.1`، `FRAMEDUP=0.1`، `NOSYNC=10.0`، وتصحيح العينة ≤10% | FFmpeg `ffplay.c` | مؤكَّد خارجيًا |
| D8 | تغيير `serial` في الطابور يستوجب `avcodec_flush_buffers` | FFmpeg `ffplay.c` | مؤكَّد خارجيًا |
| D9 | `max-lateness` وQoS في sinks تحدد متى يُعتبر الإطار متأخرًا ويُسقط | GStreamer `GstBaseSink` | مؤكَّد خارجيًا |
| D10 | mpv يوفّر `framedrop` و`display-vdrop/adrop/resample/tempo` و`max-audio-change=0.125` | mpv manual | مؤكَّد خارجيًا |
| D11 | تحويل 1080p60 إلى RGBA ≈500 MB/s كتابة | `src/pixels.rs` | مؤكَّد محليًا |
| D12 | خيط فك واحد + تحويل افتراضي على نفس الخيط | `src/player/threaded.rs` | مؤكَّد محليًا |
| D13 | re-anchor بدل إسقاط متكرر عند الإطار المتأخر | `src/player/threaded.rs` | مؤكَّد محليًا |
| D14 | نافذة HLS من مقطعين + إعادة تأسيس طوابع + native-audio window control | `src/hls/mod.rs` | مؤكَّد محليًا |
| D15 | `ImageBitmap::from_rgba8` لكل إطار | `src/widget.rs` | مؤكَّد محليًا |

### 3.2 فرضيات تفسيرية (غير مؤكَّدة)

| ID | الفرضية | لماذا تبدو معقولة | مطلوب لإثباتها |
|---|---|---|---|
| H1 | اختلاف 720p/1080p ناتج عن حجم عمل التحويل وضغط الذاكرة/العتاد، لا عن عبور عتبة `PARALLEL_MIN_PIXELS` | كان 720p = العتبة القديمة؛ العتبة الحالية `1920*1080` تجعل 720p أحادي النواة و1080p متعدد النوى | **مقاس محلياً:** مسح `workers=1..2` أعطى 1080p 11.63→6.86 ms و720p ≈5.0 ms دون مكسب يُذكر؛ هذا يبرر العتبة الجديدة |
| H2 | التقطيع في 1080p ناتج عن كون التحويل على خيط الفك نفسه | مؤكَّد محليًا أن الافتراضي كذلك، لكن الأثر غير مقاس | مقارنة `set_native_nv12(true)` مقابل الافتراضي باستخدام `stutter_probe` |
| H3 | تقطيع HLA/AJA ناتج عن إعادة تأسيس الطوابع عند rollover/تبديل مقطع لا عن الصوت | `native-audio` لا يغيّر النتيجة حسب المعطى | مقارنة طوابع `tfdt` مع زمن العرض قبل/بعد `rebase_each` |
| H4 | خطأ `odd854.mp4` هو من نوع «stride ضيّق/محاذاة صورة» الذي ترفضه Makepad أيضًا | تشابه مباشر مع D4 | تسجيل `stride`, `width`, `crop`, `offsets` للإطار قبل التحويل |
| H5 | حرارة/تقييد طاقة على الجوال يقلص فعالية التقسيم متعدد الخيوط | معطى محليًا أن `MAX_WORKERS=8`، لكن السلوك الحراري غير مقاس | قياس حراري/تردد مع عدد خيوط ثابت ومتغير |

---

## 4. مسار البكسل: NV12 مقابل GPU-assisted مقابل CPU

| المسار | الوصف | المزايا | التكلفة/الخطر | موقف المستودع | مصادر |
|---|---|---|---|---|---|
| **NV12 → RGBA على CPU** | تحويلQ16 مع تقسيم صفوف ومخزنات مملوكة | بسيط، قابل للتنبؤ، لا اعتماد على عتاد | ≈500 MB/s عند 1080p60؛ يستهلك خيط الفك افتراضيًا | **مطبَّق حاليًا** في `src/pixels.rs` + `src/player/threaded.rs` | محلي |
| **NV12 مضغوط (tight) من العتاد** | الوثوق بـ `stride = width` | أقل نسخة | فاسح عمليًا؛ الحشو/المحاذاة/Crop تجعله غير موثوق | يواجهه المستودع في خطأ `odd854.mp4` | Makepad `linux_stream_decoder.rs`؛ GStreamer `GstVideoMeta` |
| **إعادة تغليف NV12 بـ stride صحيح** | نسخ الصفوف إلى مخزن محاذٍ قبل أي تحويل | يتوافق مع `pixels.rs` ويمنع القراءة خارج الصف | نسخة إضافية | **أقرب تحسين منخفض المخاطر** | Makepad `linux_stream_decoder.rs` |
| **GPU-assisted NV12→RGBA / shader** | أخذ العينات من Y/UV ورسمها إلى RGBA على GPU | يلغي حركة الـ 500 MB/s عبر CPU | تعقيد المزامنة؛ يتطلب فences وتفاوتات ألوان | غير مطبَّق؛ يقابل `GpuCompletion` في Makepad | Makepad `surface.rs`, `color.rs`, `hal_stream_decoder.rs` |
| **Zero-copy إلى Surface** | `releaseOutputBuffer(..., true)` في أندرويد / BGRA texture intake في Apple | لا readback | يعتمد على المنصة؛ يصعب إعادة التحويل/الـ crop | غير مطبَّق؛ يتطلب مسارًا موازيًا | Android `MediaCodec`؛ Makepad `apple_texture_intake.rs` |
| **MFT Lock readback** | `IMFMediaBuffer::Lock` ثم تحويل | يعمل دون DXVA | نسخة إلزامية + زمن أعلى | يوصف في Makepad كواقع حالي لا كخيار مثالي | Makepad `windows_stream_decoder.rs` |

**الحكم:** أقرب تحسين هو **رفض/تطبيع NV12 غير الموثوق قبل دخوله `src/pixels.rs`**، ثم إضافة **توقيت GPU/fence** قبل التفكير في استبدال التحويل كليًا.

---

## 5. اقتران خيط الصوت (Audio-thread Coupling)

الحقائق:

- `src/player.rs` يرسل أوامر الصوت إلى المفكك، ويستخدم `set_position_monotonic` و`DEFAULT_FRAME=33ms`.
- `src/hls/mod.rs` يتحكم بنافذة التشغيل عبر native-audio replay.
- معطى المستخدم: تقطيع AJA HLS **يتكرر مع وبدون native-audio**.
- FFmpeg يفصل عتبات المزامنة ويحدّ من تصحيح العينة إلى 10%.
- mpv يوفّر أوضاع `video-sync` صريحة و`max-audio-change=0.125`.
- Makepad `stream_split_bench.rs` يستخدم **خيوط فك لكل tile تتشارك ساعة واحدة**، ويعرض median/p90.

الاستنتاج:

- **مؤكَّد:** الصوت لا يُعامل كخيط مستقل تمامًا؛ هناك اقتران عبر الأوامر والساعة.
- **فرضية H3:** بقاء التقطيع بدون native-audio يرجّح أن الخلل في **زمن توفر الإطار/إعادة التأسيس** لا في تأخر الصوت نفسه.
- **التوصية غير التعديلية:** أضف قياسًا (داخل أدوات القياس فقط) لـ `audio_cmd → decode_wake → frame_ready`، وقارن مع عتبات ffplay كمرجع تفكيري لا كقيم مُلزمة.

---

## 6. قيود Reels على الجوال: الخيوط، البطارية، الحرارة

| البند | الحالة المحلية | ما تقوله المصادر | التوصية |
|---|---|---|---|
| عدد الخيوط | `MAX_WORKERS=8` + `CRANPOSE_VIDEO_WORKERS` + خيط فك واحد | `stream_split_bench.rs` يوزّع حسب tiles ويقيس median/p90 | لا تزيد الخيوط عمياءً؛ اجعلها دالةً لـ `width×height` ودرجة الحرارة |
| البطارية | لا توجد سياسة طاقة محليًا | MediaCodec يوفّر إصدارًا إلى Surface لتجنب readback | فضّل zero-copy على الجوال قبل رفع التوازي |
| الحرارة | لا قياس محلي | Makepad يقرّ بزمن فك GStreamer؛ mpv يوفّر framedrop | أضف «ميزانية إسقاط» بدل محاولة عرض كل إطار عند السخونة |
| الإسقاط | newest-frame slot + re-anchor | ffplay/mpv/GStreamer كلها تضع سياسات صريحة | اجعل السياسة قابلة للقياس: كم إطار سقط؟ كم تأخر؟ متى أُعيد التثبيت؟ |

> **فرضية H5:** رفع عدد الخيوط قد يحسّن الحالة الباردة ويسوّئها بعد دقائق بسبب الخنق الحراري. لا يمكن تأكيدها دون قياس جهازي.

> **[MEASURED — TO BE FILLED AFTER LINUX/REELS RUN]:** خانة محجوزة لأرقام الخيوط/الذاكرة/الحرارة الفعلية لأنماط Reels (§6). لا تُكتب قيم مقاسة هنا قبل توفر تشغيل فعلي على Linux/جهاز مع command وبيئة ومخرجات مسجَّلة؛ تُملأ من ذلك السجل فقط، ولا تُختلَق الأرقام.

---

## 7. عدد الإطارات المتغير والدقة المتغيرة

الحقائق المؤكَّدة:

- HLS المحلي يسمح بـ `MAX_HEIGHT = u32::MAX` ويختار ABR حسب عرض النطاق، ويعيد تأسيس `tfdt` عبر `first_bases`/`rebase_each`.
- Makepad Windows يتعامل مع `MF_E_TRANSFORM_STREAM_CHANGE` و DPB من `level_idc`.
- Makepad GStreamer يرفض tight NV12 ويعيد التفاوض.
- FFmpeg `AVFrame` يحمل `color_range`, `color_primaries`, `color_trc`, `colorspace`, `chroma_location` لكل إطار.
- Makepad `color.rs` يقرأ بيانات H.273 لكل إطار ويحافظ على توافق fixed-point بين CPU وGPU.
- Android `MediaFormat` يعرّف `KEY_COLOR_RANGE`, `KEY_COLOR_TRANSFER`, `KEY_COLOR_STANDARD`, `KEY_ROTATION`, `KEY_FRAME_RATE`.

النتيجة: «الدقة/عدد الإطارات المتغير» ليس حالة استثنائية؛ هو مسار تصميمي في HLS. نقطة الضعف المحلية هي أن **إعادة التهيئة وبيانات اللون/الـ stride تُعامل كحالة إطار عادية** بدل أن تكون **حدثًا مُقاسًا ومُسجَّلًا**.

---

## 8. خطة قياس على جهاز الإصدار (نُفِّذ جزئياً محلياً)

> نُفِّذت الحالات A وB وD-partial وF/G على مضيف Windows، لكن **لم يُجرَّ أي اختبار على جهاز فعلي**. البنود المتبقية مقترحات قياس فقط.

### 8.1 الأدوات الموجودة مسبقًا

- `examples/stutter_probe.rs`: مسبار بمصدر حقيقي.
- `tests/pacing.rs`: بوابة صناعية مع stall/rollover.
- `docs/android-test-checklist.md`: قائمة أندرويد.

### 8.2 مصفوفة القياس

| الحالة | المصدر | الإعداد | المخرج المطلوب |
|---|---|---|---|
| A | `720p25 release` | الافتراضي | **[REPORT-ONLY]** نُفِّذ محلياً في جلسة سابقة: SMOOTH، 25.15 fps، p95=1.04 |
| B | `1080p25 release` | الافتراضي | **[REPORT-ONLY]** نُفِّذ محلياً في جلسة سابقة: SMOOTH، 25.14 fps، p95=1.08 (العطسة القديمة لم تعد موجودة) |
| C | `1080p25 release` | `set_native_nv12(true)` | ما زال مطلوباً على جهاز فعلي؛ لا يوجد قياس محلي نهائي لهذا الإعداد |
| D | `1080p60 release` | `CRANPOSE_VIDEO_WORKERS=1..8` | **[REPORT-ONLY]** نُفِّذ جزئياً محلياً في جلسة سابقة: 1080p 11.63→6.86 ms عند 1→2 worker؛ 720p ≈5.0 ms دون مكسب يُذكر |
| E | `odd854.mp4` | تسجيل stride/crop/offsets | تأكيد فرضية H4 |
| F | AJA HLS | مع native-audio | طوابع قبل/بعد `rebase_each` |
| G | AJA HLS | بدون native-audio | مقارنة F |
| H | HLS rollover | حقن stall/rollover | زمن إعادة التثبيت |

### 8.3 المقاييس الإلزامية

1. `decode_submit → decode_done` (median / p90).
2. `decode_done → convert_done` (إن وُجد تحويل CPU).
3. `convert_done → present` مع عدد الإطارات المتأخرة والمسقوطة.
4. عدد مرات `rebase_each` و`stream change`.
5. عدد الخيوط الفعّالة، استهلاك CPU، حرارة الجهاز/ترددها.
6. نسخة البناء، الجهاز، نظام التشغيل، مسار NV12 الخام.

### 8.4 معايير نجاح مقترحة (تُثبَّت قبل التنفيذ)

- ألا يتجاوز p90 لـ `decode→present` ميزانية الإطار + عتبة معلنة (تُقارن بمنطق `AV_SYNC_THRESHOLD_MAX` لا بقيمة مطلقة).
- أن يبقى عدد الإطارات المسقوطة ضمن سقف معلن لكل 10 ثوانٍ.
- ألا يتغير سلوك F مقابل G بشكل جوهري إذا كانت H3 صحيحة.

---

## 9. كتالوج التقنيات

> عمود **الانطباق** يعني: هل يمكن تطبيقه داخل `cranpose-video` دون تغيير API عام؟ كل التقنيات المذكورة هنا متوافقة مع هذا القيد إن نُفِّذت كتفاصيل داخلية/أدوات قياس.

| ID | التقنية | المصدر | الفائدة | التكلفة | المخاطرة | الانطباق على المستودع |
|---|---|---|---|---|---|---|
| M1 | `GpuCompletion` مع `sync_file` fence على Linux | Makepad `surface.rs`, `linux_stream_decoder.rs` | معرفة زمن اكتمال GPU بدل التخمين | متوسطة | متوسطة (توقيت/منصات) | عالي: يحل مكان «زمن غير مقاس» |
| M2 | بيانات ألوان H.273 لكل إطار + توافق Q16 CPU/GPU | Makepad `color.rs`؛ FFmpeg `frame.h`؛ Android `MediaFormat` | منع انزياح اللون عند تغيير المحتوى | منخفضة-متوسطة | منخفضة | عالي جدًا: يكمل `src/pixels.rs` |
| M3 | رفض NV12 tight وإعادة تغليفه | Makepad `linux_stream_decoder.rs`؛ GStreamer `GstVideoMeta` | يمنع قراءة خارج الصف؛ يفسّر خطأ odd854 | منخفضة | منخفضة | عالي جدًا؛ قبل `pixels.rs` مباشرة |
| M4 | الاعتراف بـ decode latency عشرات الملي ثانية | Makepad `linux_stream_decoder.rs` | يمنع توقعات خاطئة للميزانية | منخفضة | منخفضة | عالي: يغذّي ميزانية الإسقاط |
| M5 | التفاوض عند `MF_E_TRANSFORM_STREAM_CHANGE` + DPB من `level_idc` | Makepad `windows_stream_decoder.rs` | تعامل صحيح مع تغير الدقة/عدد الإطارات | متوسطة | متوسطة | عالي: يوازي `hls/mod.rs` |
| M6 | كشف غياب DXVA وقياس تكلفة `IMFMediaBuffer::Lock` | Makepad `windows_stream_decoder.rs` | تفسير زمن Windows | منخفضة | منخفضة | متوسط:قياس فقط |
| M7 | GPU picture tickets مع إصدار عند الإسقاط | Makepad `android_decoder.rs` | يمنع تجميد المفكك بسبب تراكم buffers | متوسطة | متوسطة | عالي على أندرويد |
| M8 | BGRA→NV12 intake على VideoToolbox | Makepad `apple_texture_intake.rs` | تقليل readback على Apple | عالية | عالية | مشروط بمنصة Apple |
| M9 | bench بمخزن `PICTURES` و`--frames N` | Makepad `examples/stream_bench.rs` | قياس throughput_frames/زمن بلا تخمين | منخفضة | منخفضة | عالي جدًا: يكمل `stutter_probe` |
| M10 | خيوط فك لكل tile تتشارك ساعة + median/p90 | Makepad `examples/stream_split_bench.rs` | يفصل زمن الفك عن زمن العرض | متوسطة | متوسطة | عالي: يغذّي D/C في خطة القياس |
| M11 | اختبارات state/session تحت الانقسام | Makepad `examples/stream_sessions.rs`, `stream_decode_check.rs` | يكشف أخطاء إعادة التهيئة | منخفضة | منخفضة | عالي: يكمل `tests/pacing.rs` |
| M12 | دورة حياة surface/texture صريحة | Makepad `surface.rs` | تقليل الأخطاء عند recycle | متوسطة | متوسطة | متوسط |
| M13 | تجريد HAL stream decoder | Makepad `hal_stream_decoder.rs` | توحيد منطق المنصات | عالية | عالية | متوسط: قد يكسر البساطة الحالية |
| F1 | عتبات مزامنة ffplay + framedup + flush عند serial change | FFmpeg `fftools/ffplay.c` | سياسة إسقاط قابلة للتفسير | منخفضة | متوسطة | عالي كمرجع سياسة |
| F2 | بيانات لون لكل إطار | FFmpeg `libavutil/frame.h` | سلامة لون عبر ABR | منخفضة | منخفضة | عالي (مكرر مع M2) |
| F3 | خيارات HWAccel ونسخ الـ copy-back | FFmpeg `HWAccelIntro`, `Chapter_hardware` | فهم تكلفة readback | منخفضة | منخفضة | عالي كمرجع معماري |
| F4 | واجهة ffmpeg لخيارات المزامنة/الفك | `ffmpeg.org/ffmpeg.html` | مرجع أسماء الخيارات والسلوك | منخفضة | منخفضة | متوسط |
| G1 | تجميع GstVideoDecoder وإ_SUPPORT subframe | GStreamer `GstVideoDecoder` | فهم زمن التجميع/التجزئة | منخفضة | متوسطة | متوسط |
| G2 | `GstVideoMeta` للـ stride/offset/crop/color | GStreamer `GstVideoMeta` | المعالجة الصحيحة للإطارات غير المضغوطة | منخفضة | منخفضة | عالي جدًا (مكرر مع M3/M2) |
| G3 | `max-lateness` وQoS في Sink | GStreamer `GstBaseSink` | تعريف «متأخر» رسميًا | منخفضة | متوسطة | عالي |
| G4 | إعادة تفاوض Pipeline | GStreamer `GstPipeline` | نموذج لتغير الدقة أثناء البث | متوسطة | متوسطة | متوسط |
| P1 | `framedrop` + `hwdec=*-copy` + `video-sync` + `max-audio-change` | mpv manual | سياسات عرض/صوت ناضجة | منخفضة | متوسطة | عالي كمرجع |
| V1 | وحدة avcodec / فك العتاد في VLC | VLC wiki `avcodec` | مقارنة سلوك المفكك/النسخة | منخفضة | منخفضة | متوسط |
| A1 | `releaseOutputBuffer` إلى Surface + تحذير إمساك buffers | Android `MediaCodec` | تقليل نسخة CPU ومنع توقف المفكك | متوسطة | متوسطة | عالي على أندرويد |
| A2 | مفاتيح `MediaFormat` للون/الدوران/معدل الإطار | Android `MediaFormat` | نقل بيانات الوصف مع الإطار | منخفضة | منخفضة | عالي |
| L1 | decode-ahead + newest-frame slot | `src/player/threaded.rs` | موجود بالفعل | — | — | مطبَّق |
| L2 | `SLICE/HORIZON/IDLE` pacing | `src/player/threaded.rs` | موجود بالفعل | — | — | مطبَّق |
| L3 | re-anchor عند تأخر الإطار | `src/player/threaded.rs` | موجود بالفعل؛ يحتاج قياسًا | — | — | مطبَّق |
| L4 | تحويل Q16 بمخزنات المستدعي وتقسيم الصفوف | `src/pixels.rs` | موجود بالفعل | — | — | مطبَّق |
| L5 | ABR/prefetch/rebase في HLS | `src/hls/mod.rs` | موجود بالفعل | — | — | مطبَّق |
| L6 | مسبار التقطيع + بوابة pacing | `examples/stutter_probe.rs`, `tests/pacing.rs` | موجود بالفعل | — | — | مطبَّق |

---

## 10. نقاط الضعف المرتّبة في المستودع

| الرتبة | نقطة الضعف | الدليل | نوع الحكم | التقنية المقابلة |
|---|---|---|---|---|
| 1 | لا يوجد قياس زمني من الفك إلى العرض | `stutter_probe` موجود لكن لا توجد أرقام p50/p90 في المستندات | فجوة قياس | M9/M10 |
| 2 | تحويل NV12 على خيط الفك افتراضيًا | `threaded.rs` | مؤكَّد محليًا | M1/M7 |
| 3 | الثقة بـ stride/تنسيق الإطار قبل `pixels.rs` | خطأ odd854 + M3 | فرضية H4 قوية | M3/G2 |
| 4 | بيانات اللون/الدقة المتغيرة ليست حدثًا مُقاسًا | `hls/mod.rs` + M2/A2 | مؤكَّد محليًا + خارجيًا | M2/M5/F2 |
| 5 | سياسة الإسقاط ضمنية (newest + re-anchor) بلا سقف معلن | `threaded.rs` | مؤكَّد محليًا | F1/G3/P1 |
| 6 | ميزانية الخيوط ثابتة نسبيًا مقابل الحرارة | `MAX_WORKERS=8` | فرضية H5 | M10/A1 |
| 7 | مسار Windows يتضمن readback محتملًا | M5/M6 | مؤكَّد خارجيًا | M6/F3 |
| 8 | اقتران أوامر الصوت بالمفكك دون فصل زمني معلن | `player.rs` + معطى AJA | مؤكَّد + فرضية H3 | F1/P1 |

---

## 11. الترتيب النهائي: أفضل 10 تقنيات

| الترتيب | التقنية | الفائدة | التكلفة | المخاطرة | الانطباق | لماذا في هذا الترتيب |
|---:|---|---|---|---|---|---|
| 1 | **M1 — GpuCompletion + fence** | عالية | متوسطة | متوسطة | عالي | يعطي «زمن الحقيقة» بدل التخمين؛ يفتح الباب لكل تحسين لاحق |
| 2 | **M2/F2/A2 — بيانات اللون لكل إطار** | عالية | منخفضة-متوسطة | منخفضة | عالي جدًا | يمنع انزياح الألوان عند ABR/تغير المصدر ويكمل Q16 الحالي |
| 3 | **M3/G2 — رفض/تطبيع NV12 غير الموثوق** | عالية | منخفضة | منخفضة | عالي جدًا | يضرب مباشرة في خطأ `odd854.mp4` ويحمي `pixels.rs` |
| 4 | **M5 — التفاوض عند تغيير الدقة/DPB** | عالية | متوسطة | متوسطة | عالي | يتعامل مع جوهر HLS متعدد الدقات |
| 5 | **M7/A1 — إصدار GPU/Surface عند الإسقاط** | عالية | متوسطة | متوسطة | عالي (أندرويد) | يمنع تراكم buffers ويقلل readback على الجوال |
| 6 | **M9/M10 — bench بـ median/p90 وخيوط مقسّمة** | عالية | منخفضة-متوسطة | منخفضة | عالي جدًا | أرخص طريقة لتحويل الفرضيات H1/H2/H5 إلى أرقام |
| 7 | **M8/F3 — GPU-assisted/zero-copy حسب المنصة** | عالية | عالية | عالية | مشروط | أكبر أثر نظري لكنه يتطلب فences ومسارًا موازيًا |
| 8 | **F1/P1/G3 — سياسة مزامنة/إسقاط صريحة** | متوسطة-عالية | منخفضة | متوسطة | عالي | يحوّل newest-frame slot إلى سياسة قابلة للتفسير والقياس |
| 9 | **M4/M11 — latencyBudget + اختبارات session** | متوسطة-عالية | منخفضة | منخفضة | عالي | يحمي من regressions عند rollover/تبديل مقطع |
| 10 | **M13/G4 — تجريد موحد لإعادة التهيئة** | متوسطة | عالية | عالية | متوسط | مفيد طويل الأمد لكنه لا يعالج العرض الحالي مباشرة |

### أعلى 5 للتنفيذ أولًا (بترتيب البحث لا التنفيذ الفعلي)

1. M1 — GpuCompletion/fence.
2. M2/F2/A2 — per-frame color metadata.
3. M3/G2 — رفض/تطبيع NV12.
4. M5 — stream-change/DPB.
5. M7/A1 — release-on-drop / Surface.

---

## 12. فهرس المصادر

### Makepad (raw `main`)

- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/surface.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/color.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/nv12.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/linux_stream_decoder.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/windows_stream_decoder.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/windows_decoder.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/android_decoder.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/apple_decoder.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/apple_stream_decoder.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/apple_texture_intake.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/stream_decoder.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/src/hal_stream_decoder.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/examples/stream_bench.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/examples/stream_split_bench.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/examples/stream_decode_check.rs`
- `https://raw.githubusercontent.com/makepad/makepad/main/platform/video/examples/stream_sessions.rs`
- `https://api.github.com/repos/makepad/makepad/contents/platform/video/src?ref=main`
- `https://api.github.com/repos/makepad/makepad/contents/platform/video/examples?ref=main`

### FFmpeg

- `https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/fftools/ffplay.c`
- `https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavutil/frame.h`
- `https://trac.ffmpeg.org/wiki/HWAccelIntro`
- `https://trac.ffmpeg.org/wiki/Chapter_hardware`
- `https://ffmpeg.org/ffmpeg.html`

### GStreamer

- `https://gstreamer.freedesktop.org/documentation/video/gstvideodecoder.html?gi-language=c`
- `https://gstreamer.freedesktop.org/documentation/video/gstvideometa.html?gi-language=c`
- `https://gstreamer.freedesktop.org/documentation/gstreamer/gstpipeline.html?gi-language=c`
- `https://gstreamer.freedesktop.org/documentation/gstreamer/gstbasesink.html?gi-language=c`

### mpv / VLC / Android

- `https://mpv.io/manual/master/`
- `https://wiki.videolan.org/Documentation:Modules/avcodec/`
- `https://developer.android.com/reference/android/media/MediaCodec`
- `https://developer.android.com/reference/android/media/MediaFormat`

### محلية

- `D:\counter\cranpose-video\src\pixels.rs`
- `D:\counter\cranpose-video\src\frame.rs`
- `D:\counter\cranpose-video\src\player\threaded.rs`
- `D:\counter\cranpose-video\src\player.rs`
- `D:\counter\cranpose-video\src\hls\mod.rs`
- `D:\counter\cranpose-video\src\widget.rs`
- `D:\counter\cranpose-video\examples\stutter_probe.rs`
- `D:\counter\cranpose-video\tests\pacing.rs`
- `D:\counter\cranpose-video\docs\android-test-checklist.md`

---

## 13. سجل الفرضيات الختامي

| ID | الحالة | الخلاصة |
|---|---|---|
| H1 | محسوم محلياً | العتبة القديمة لم تكن السبب؛ العتبة الجديدة `1920*1080` مبنية على قياس worker costs. الفرق المتبقي حجم عمل/عتاد |
| H2 | محسوم جزئياً | 1080p NV12→RGBA مقاس محلياً 6.65–6.74 ms لكل إطار؛ لا يزال العزل النهائي عبر `set_native_nv12(true)` يحتاج جهازاً |
| H3 | **[REPORT-ONLY]** مدعوم محلياً، غير محسوم | AJA STUTTER مع وبدون native-audio (23.67/24.27 fps) يدعم أنه ليس صوتياً بالدرجة الأولى؛ لا يزال rollover مفتوحاً |
| H4 | فرضية قوية | خطأ odd854 من فئة stride/crop غير الموثوق؛ يطابق سلوك Makepad |
| H5 | فرضية | الاختناق الحراري قد يقلب جدوى الخيوط المتعددة؛ غير مقاس |

**الخلاصة النهائية:** المستودع يملك بالفعل نواة جيّدة (pacing + newest-frame + re-anchor + Q16 + HLS rebase)، وأضيف إليها قياس زمن النشر وdecode/convert وتحسينات الجداول والانتظار الدقيق. ما زال أكبر نقص هو **بيانات الوصف المرافقة للإطار وقياس العرض على GPU/الشاشة**، لا غياب الخوارزميات. الترتيب المتبقي هو fences/بيانات اللون/device proof، مع إبقاء HLS rollover كمسار منفصل.
