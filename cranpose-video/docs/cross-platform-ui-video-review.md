# مراجعة أدوات الواجهة ومصفوفة تشغيل الفيديو عبر المنصات — `cranpose-video`

> **الحالة:** مراجعة ثابتة (Static / قراءة فقط). لم أُجرِ أي تشغيل على جهاز حقيقي، ولم أُعيد أي اختبار أداء أو بناء في هذه الجولة، ولا يُدّعى هذا المستند ذلك.  
> **نطاق التعديل:** هذا الملف فقط. لم تُعدَّل أي شيفرة Rust، ولا `Cargo.toml`، ولا `README.md`، ولا أي ملف آخر، ولم يُنشأ commit أو push.  
> **التاريخ:** 2026-11-18.  
> **العلاقة بالتدقيق الموجود:** هذا المستند **لا يكرّر** `docs/mobile-platform-audit.md`. ذاك التدقيق يغطي مسارات الجوال/الوسائط الداخلية؛ أمّا هنا فالتركيز على: (1) مكتبات/أدوات الواجهة، (2) مصفوفة التشغيل الفعلي لكل نظام، (3) مسار العرض الحالي وطريقة دمجه دون كسر الواجهة العامة.  
> **رموز الحالة:** `[مؤكّد]` = مقروء من الشيفرة أو من مصدر رسمي تطابق؛ `[فرضية]` = استنتاج تصميمي يحتاج قياساً أو تجربة؛ `[غير مُختبَر]` = لم يُشغّل على جهاز/متصفح في هذه المراجعة.

---

## 1. سجل Skills المستخدمة

تم تحميل المهارات التالية قبل بدء العمل، والجدول يبيّن موضع استخدام كل واحدة في هذا التقرير:

| # | Skill | موضع الاستخدام في التقرير |
|---|---|---|
| 1 | `makepad-2.0-design-judgment` | ترتيب خيار Makepad أولاً وتجنّب استبدال نواة Rust القائمة. |
| 2 | `makepad-2.0-app-structure` | التمييز بين غلاف التطبيق ووحدة تكامل إضافية لا تلمس `main.rs`. |
| 3 | `makepad-2.0-widgets` | تحليل `Video` / `VideoPlayerRef` وواجهة `VideoFrameSession`. |
| 4 | `makepad-2.0-performance` | تقييم مسارات النسيج/النسخة الصفرية ومقارنتها بـ `ImageBitmap`. |
| 5 | `makepad-2.0-layout` | تقييم سلوك ملء المساحة والتحجيم في جدول العرض. |
| 6 | `makepad-2.0-events` | ربط أحداث المشغّل (play/pause/seek) بأحداث الأداة. |
| 7 | `makepad-2.0-theme` | أثر الثيم على عناصر التحكم المقترحة دون تغيير توقيعها. |
| 8 | `makepad-2.0-shaders` | اعتبار أخذ عيّنتَي NV12 كنسيجين بدل تحويل RGBA. |
| 9 | `rust-uniffi-building` | مقترح روابط Swift/Kotlin فوق نواة Rust القائمة. |
| 10 | `flutter-expert` | صف Flutter في مصفوفة الأدوات. |
| 11 | `react-native-expert` | صف React Native وطبيعة الوحدة الأصلية. |
| 12 | `swift-expert` | صف SwiftUI/UIKit وتوصية iOS. |
| 13 | `kotlin-specialist` | صف Jetpack Compose وMedia3. |
| 14 | `interface-design` | معايير تقييم الواجهة (اتساق، حالات، تكلفة تكامل). |
| 15 | `apple-hig-expert` | اعتبارات واجهة Apple وطبقة العرض على iOS. |
| 16 | `react-expert` | صف React على الويب. |
| 17 | `vue-expert` | صف Vue على الويب. |
| 18 | `nextjs-developer` | اعتبارات تغليف صفحة الويب/البناء في مسار الويب. |
| 19 | `a11y-audit` | ملاحظة الوصولية لعناصر التحكم الحالية/المقترحة. |
| 20 | `ux-researcher-designer` | معايير المقارنة تجريبياً وتجنّب ادّعاء نتائج لم تُقَس. |

---

## 2. لقطة مؤكّدة من المشروع (ما يهمّ العرض فقط)

| # | الحقيقة | المرجع |
|---|---|---|
| C1 | الواجهة الحالية تمرّر كل إطار عبر `ImageBitmap::from_rgba8(width, height, frame.into_rgba())` ثم `Image` داخل composable. | `src/widget.rs:161,171-176,188-196` `[مؤكّد]` |
| C2 | مفكّكات المنصات تُنتج NV12 أصلاً مع بديل BGRA/RGBA عند الحاجة؛ أي أن التحويل إلى RGBA ليس فرضاً تقنياً بل قرار عرض. | `README.md:126-150`؛ `src/platform/apple.rs:154-171,337-369` `[مؤكّد]` |
| C3 | أداة القياس `stutter_probe` تقيس نشر الإطارات فقط ولا تختبر مسار الـ bitmap/الرفع إلى GPU في `src/widget.rs`. | `README.md:571-573` `[مؤكّد]` |
| C4 | على `wasm32-unknown-unknown` لا توجد خيوط؛ المتصفح يحتفظ بالساعة عبر عنصر `<video>` منفصل، ثم رسم على canvas وقراءة `getImageData` RGBA. | `src/platform/web.rs:1-9,41-93,307-352` `[مؤكّد]` |
| C5 | التوجيه المنصّي يعرف `Ios` صراحةً، ويرسل HLS الأصلي عبر `crate::hls::open` لكل من Windows/Android/macOS/iOS/Linux، ويترك متصفح الويب يتولّى HLS بنفسه. | `src/platform/mod.rs:3-23,43-63,114-132` `[مؤكّد]` |
| C6 | الواجهة العامة `$Video` / `VideoWithControls` / `VideoPlayerHandle` قائمة ومستقرة؛ أي مقترح أدناه يضيف مساراً جديداً ولا يغيّر توقيعاً قائماً. | `src/widget.rs:20-155`؛ `README.md:9-12,32` `[مؤكّد]` |
| C7 | الميزات `widget` و`player` اختيارية؛ بناء يكتفي بالفكّ يستطيع تعطيل واجهة Cranpose الحالية. | `Cargo.toml:15-40` `[مؤكّد]` |
| C8 | لم يُجرَ تشغيل على جهاز حقيقي لنظام Android أو iOS، ولا فحص HLS وقت التشغيل على Linux، وبناء هدف Android ممنوع محلياً لغياب `ANDROID_NDK_HOME`. | `README.md:588` `[مؤكّد من التوثيق]` |

**أكبر فجوة عرض (Display gap):** المسار المؤكّد في `src/widget.rs` يحوّل كل إطار إلى RGBA كامل ثم ينشئ bitmap جديداً كل إطار، بينما المفكّكات توفّر طائرات NV12. على الويب تتضاعف النسخة (فيديو → canvas → RGBA → bitmap → GPU). `[مؤكّد كمسار]` أمّا المقدار الفعلي للتكلفة على كل جهاز فهو `[فرضية]` لأن أدوات القياس الحالية لا تغطي هذا المسار (C3).

---

## 3. مصفوفة أدوات/مكتبات الواجهة

> ملاحظة: «التكامل» يعني إعادة استخدام نواة `cranpose-video` نفسها؛ حيث لا يكون ذلك ممكناً بشكل طبيعي، يصبح الحل «غلافاً» يستخدم مشغّل المنصة مباشرةً ويُسجّل كخيار أقل تفضيلاً. كل الصفوف تفترض **إضافة** وحدة/حزمة جديدة، لا تعديل الواجهة الحالية.

| الأداة | المنصات (حسب مصادرها) | مسار تشغيل الفيديو الطبيعي | طريقة دمج `cranpose-video` | أثر الواجهة العامة | التقييم |
|---|---|---|---|---|---|
| **Makepad** `[مؤكّد من مصدر المشروع]` | سطح المكتب والجوال والويب (README الرسمي) | ميزة `media` مدمجة؛ `VideoPlayerRef::open` يقبل مساراً/ملفاً/محتوى/http ويعالج `.m3u8`/`.mpd` كبث؛ `Video` يقبل `set_source_session` / `set_boxed_source_session` ووضع `YuvPlanes`؛ مسارات نسيج أصلية: DMA-Buf NV12 على Linux، D3D11VA NV12 على Windows، OES/SurfaceTexture على Android، Metal على Apple | الأقوى: تنفيذ `VideoFrameSession` جديد يقرأ من `VideoPlayer::take_frame()` ويمرّر luma/chroma كما هما، مع حفظ تواقيع `Video`/`VideoPlayerHandle` كما هي | إضافة فقط: حزمة تكامل + feature اختياري | **الأفضل**؛ يضرب فجوة C1 مباشرة `[مؤكّد من واجهة Makepad، والأثر النهائي فرضية حتى يُقاس]` |
| **Flutter** `[مؤكّد من docs.flutter.dev]` | iOS/Android/Windows/macOS/Linux/Web | `video_player` يعتمد على مشغّل المنصة ( cookbook الرسمي ) | غلاف مع MethodChannel/FFI إلى نواة Rust؛ أو ترك `video_player` يشغّل وتُستخدم النواة للحالات التي تحتاج فكّاً مخصصاً | إضافة فقط، لكنها تكرّر مكدّس الوسائط إذا شغّلت الإضافة | جيد للواجهة، متوسط للدمج؛ مكدّس مزدوج `[فرضية تكلفة]` |
| **React Native** `[مؤكّد من reactnative.dev]` | iOS/Android رسمياً؛ Windows/macOS عبر جسور منفصلة `[يحتاج تحققاً]` | لا مشغّل مدمج في النواة؛ وحدة أصلية/مكتبة مجتمعية تستخدم AVPlayer/ExoPlayer | TurboModule/NativeModule يغلّف `VideoPlayerHandle` أو يستدعي cdylib | إضافة فقط؛ جهد JNI/ObjC مرتفع | خيار ثانوي؛ أعلى جهد ربط `[فرضية]` |
| **SwiftUI / UIKit** `[مؤكّد من Apple archive + swift-expert]` | Apple فقط | `AVPlayer` + `AVPlayerItem` + `AVPlayerLayer`، وHLS عبر URL | UniFFI أو C ABI يكشف واجهة مشغّل مبسّطة، ثم عرض عبر Metal/AVPlayerLayer حسب الحاجة | إضافة طبقة روابط فقط | الأفضل لـ iOS إن أردنا غلافاً أصلياً، لكنه لا يغطي بقية المنصات |
| **Jetpack Compose** `[مؤكّد من developer.android.com]` | Android | Media3/ExoPlayer هو المسار الرسمي | JNI/UniFFI إلى النواة مع `@Composable` يعرض الـ surface | إضافة فقط | جيد لـ Android وحده؛ لا يحل مشكلة iOS/الويب |
| **Qt / Qt Multimedia** `[مؤكّد من doc.qt.io]` | واسع (سطح مكتب/جوال/مدمج) | `QMediaPlayer` / QtMultimedia | ربط C++/QML بـ Rust عبر جسر (cxx/UniFFI/cdylib) | إضافة فقط، لكن جسر C++ كبير | تكلفة مرتفعة؛ ترخيص Qt يحتاج مراجعة منفصلة `[تنويه، لا حكم قانوني]` |
| **Avalonia / .NET MAUI** `[مؤكّد من learn.microsoft.com و docs.avaloniaui.net]` | Windows/macOS/iOS/Android/Web حسب كل إطار | عناصر/إضافات وسائط تعتمد على مشغّل المنصة `[يحتاج تحققاً من إصدار MAUI]` | P/Invoke إلى cdylib من نواة Rust | إضافة فقط؛ juggling بين مسارات .NET | تكلفة ربط عالية؛ لا مكاسب عرض مقابل Makepad |
| **Electron / Tauri v2** `[مؤكّد من electronjs.org و v2.tauri.app]` | سطح المكتب + حزم الويب | HTML5 `<video>` + فكّ WebView/النظام | Tauri: يستدعي أوامر Rust مباشرةً في العملية الرئيسية؛ Electron: يحتاج إضافة أصلية أصلي لاستدعاء النواة | إضافة غلاف فقط | خيار قوي لسطح المكتب/الويب، خصوصاً Tauri؛ يعتمد على تراتب codecs في WebView `[فرضية]` |
| **Vue / React على الويب** `[مؤكّد من vuejs.org و react.dev/learn]` | المتصفح | `<video>` + `<canvas>` + WebCodecs حيث تتوفر | يستهلك بناء wasm الحالي مباشرةً (`src/platform/web.rs`) مع `hls.js` عند الحاجة | إضافة صفحة/حزمة فقط | أقل تكلفة وأضيق تغطية (ويب فقط)؛ يبقى مسار النسخة RGBA قائماً `[مؤكّد]` |

**خلاصة المصفوفة:** Makepad هو الوحيد الذي يوفّر واجهة استقبال إطار مدعومة رسمياً (`VideoFrameSession` + `YuvPlanes`) باللغة نفسها (Rust)، لذلك هو المرشح الوحيد الذي يعالج C1 دون إدخال مكدّس وسائط ثانٍ.

---

## 4. مصفوفة التشغيل الفعلي حسب النظام

> «التشغيل» هنا يعني ما تقوله الشيفرة/المصادر، لا ما تم تنفيذه في هذه الجولة.

| النظام | الخلفية المؤكّدة في المستودع | تنسيق الإطار اليوم | HLS اليوم | الصوت | الحالة التوثيقية |
|---|---|---|---|---|---|
| **Windows** | Media Foundation source reader (`src/platform/windows.rs`) | NV12 أصلي، مع RGB32 عند دوران يتطلّبه المعالج | يُضمّ through `crate::hls` ثم يُفتح كملف | صوت مدمج عبر نفس source reader + `cpal` | `[مؤكّد من README]` قياسات على مضيف تطوير؛ لا يُعدّ اختبار جهاز مستقل في هذه المراجعة |
| **macOS** | AVFoundation `AVAssetReader` (`src/platform/apple.rs`) | NV12 (`420v`) مع BGRA بديل | يُضمّ عبر `crate::hls` | صوت مقترن أفضل جهد | `[مؤكّد من الشيفرة] [غير مُختبَر على جهاز]` |
| **iOS** | نفس ملف Apple تحت `cfg(target_os = "ios")`؛ `AVURLAsset` + `AVAssetReaderTrackOutput` | NV12 مع BGRA بديل؛ `naturalSize`/`preferredTransform` للبعد والدوران | `src/platform/mod.rs` يُدخل `ios` صراحةً في مسار `crate::hls::open` | صوت مقترن أفضل جهد | **[مؤكّد: مدعوم ومغطى بالبناء، لكنه غير مُعاد التحقق على جهاز]** وفقاً لـ `README.md:116,122` |
| **Android** | `MediaExtractor` + `MediaCodec` عبر JNI | نسخ طائرات YUV إلى NV12 | يُضمّ عبر `crate::hls` | صوت مقترن أفضل جهد | `[مؤكّد من الشيفرة]` توجد قائمة فحص جهاز لكنها لم تُنفَّذ؛ بناء الهدف محجوب بغياب NDK |
| **Linux** | GStreamer خلف `linux-gstreamer`؛ `uridecodebin → videoconvert → appsink` بـ NV12 | طائرات NV12 | يُضمّ عبر `crate::hls` عند تفعيل الميزة | صوت مقترن أفضل جهد | `[مؤكّد]` GStreamer غير مثبّت هنا؛ فحوص HLS وقت التشغيل معلّقة |
| **Web / wasm32** | `<video>` منفصل + canvas 2D + `getImageData` | RGBA دائماً في المسار الحالي | Safari/iOS يقرأ HLS ذاتياً؛ غيره يستخدم `hls.js` | المتصفح يدير الصوت، والبدء مكتوم حتى تفاعل المستخدم | `[مؤكّد من الشيفرة]` فحص البناء نظيف حسب README؛ لم أُجرِ اختبار متصفح في هذه الجولة |
| **watchOS** | لا مفكّك؛ `AVAssetReader` غائب | — | — | — | `[مؤكّد]` يُعيد `VideoError::Unsupported`؛ خارج نطاق التوصية |

### 4.1 حالة iOS صريحةً

- **موجود فعلاً:** `Platform::Ios`، وقاعدة `#[cfg(any(target_os = "macos", target_os = "ios"))] mod apple;`، واعتمادات `objc2-av-foundation` مُوجّهة لـ iOS في `Cargo.toml:86-129`. `[مؤكّد]`
- **ما تفعله الخلفية:** تفتح `AVURLAsset`، وتقرأ `naturalSize` و`preferredTransform` و`nominalFrameRate`، ثم تنشئ `AVAssetReader` مع `AVAssetReaderTrackOutput`؛ وتحاول NV12 ثم تتراجع إلى BGRA، وتحتفظ بـ PTS من `CMSampleBuffer`. `[مؤكّد: src/platform/apple.rs:45-93,154-171,223-334,337-369]`
- **HLS على iOS:** الموجّه يُدخل `ios` في شرط `crate::hls::open`، أي أن البث يُضمّ إلى نافذة ملفات ثم يُفتح بـ AVFoundation، وليس عبر `AVPlayer` الأصلي للبث المباشر. `[مؤكّد: src/platform/mod.rs:121-132]`
- **مصادر Apple:** وثيقة AVFoundation المؤرشفة تؤكد `AVPlayer`/`AVPlayerItem`/`AVPlayerLayer`، وإنشاء عنصر من URL لـ HLS، ومراقبة الحالة/الجاهزية عبر KVO، وتحديث الواجهة على الخيط الرئيسي. صفحات Apple الحديثة المقابلة عادت فارغة في هذه الجولة، لذا اعتُمد الأرشيف. `[مؤكّد من المصدر المؤرشف]`
- **ما لم يحدث:** لا تشغيل على جهاز iOS، ولا تحقق من سلوك الخلفية/انقطاع الجلسة الصوتية/الذاكرة، ولا قياس لزمن العرض. `[غير مُختبَر]`
- **الفجوة الخاصة بـ iOS:** المكوّن الحالي يعرض RGBA عبر `ImageBitmap`، بينما يوفّر AVFoundation طائرات NV12 ويمكن للمشغّل الأصلي عرضها عبر طبقة/نسيج؛ أي أن iOS لا يعاني غياب خلفية فكّ، بل غياب «سطح عرض» يستقبل الطائرات. `[مؤكّد كوصف][فرضية كأثر أداء]`

---

## 5. مقترحات الدمج (كلها غير كاسرة للواجهة)

### 5.1 الخيار 1 — Makepad `media` + `VideoFrameSession` (الأول)

**الفكرة:** حزمة جديدة — مثلاً `cranpose-video-makepad` خلف feature اختياري — تُنشئ جلسة مصدر تُطعم `Video` ببيانات المشغّل الحالي.

```text
cranpose_video::VideoPlayer  →  take_frame() → luma()/chroma()  →  VideoFrameSession(YuvPlanes)  →  Makepad Video
```

**لماذا هو غير كاسر:**
- تُضاف دالة/نوع جديد مثل `video_from_player(handle) -> VideoRef` ولا يُغيّر `Video(source, modifier)` القائم.
- `Video::set_source_session` و`set_boxed_source_session` موجودان في واجهة Makepad، و`VideoSourceMode::YuvPlanes` يسمح بتمرير الطائرات بدل RGBA. `[مؤكّد]`
- تظل `VideoPlayerHandle` و`VideoWithControls` كما هما؛ عناصر التحكم الجديدة تُبنى فوقها.

**ما يُكتسب:** إلغاء إنشاء `ImageBitmap` لكل إطار في المسار الجديد، والاستفادة من مسور النسيج الأصلية الموثّقة في مصدر Makepad (DMA-Buf/D3D11VA/OES/Metal). `[مؤكّد من المصدر]` تحقّق النتيجة النهائية على الأجهزة `[فرضية]`.

**الحدود:** يضيف تبعية Makepad اختيارية؛ لا يفيد browsers إلا إذا استُخدم مسار الويب الخاص بـ Makepad، ويبقى مسار `src/platform/web.rs` قائماً للويب العام.

### 5.2 الخيار 2 — روابط UniFFI حول نواة Rust (الثاني)

**الفكرة:** حزمة روابط جديدة تعرض كائناً شفافاً (`VideoPlayer`) بعمليات `open/play/pause/seek/take_frame` للغة Swift وKotlin (وغيرها حسب UniFFI). تُستخدم من SwiftUI/UIKit أو Compose عند الحاجة إلى غلاف أصلي.

**غير كاسر:** حزمة منفصلة إضافة بالكامل؛ لا تغيير في `Cargo.toml` الحالي إلا كعضو جديد إن رُغب، ولا تغيير في أي توقيع.

**الإيجاب:** يعيد استخدام مفكّكات Apple/Android الحالية بدل تكرارها داخل `video_player` أو Media3؛ يمنح iOS مساراً أصلياً بدون استبدال `AVAssetReader`.

**الحدود:** نقل الإطارات عبر FFI يبقى تكلفة محتملة `[فرضية]`؛ يجب قياسها قبل استبدال مسار العرض.

### 5.3 الخيار 3 — Tauri/WebView غلاف لسطح المكتب والويب (الثالث)

**الفكرة:** غلاف Tauri v2 يستدعي أوامر Rust من الواجهة، ويستخدم عنصر `<video>` الموجود أصلاً في `src/platform/web.rs` مع `hls.js` عند غياب HLS أصلي.

**غير كاسر:** يستهلك البناء الحالي كما هو؛ لا تعديل على `Video`/`VideoPlayerHandle`.

**الإيجاب:** أقل جهد لسطح المكتب + الويب معاً، ويستفيد من نواة Rust مباشرةً في عملية Tauri.

**الحدود:** يعتمد على codecs المتاحة في WebView2/WKWebView/WebKitGTK `[فرضية]`؛ لا يعالج فجوة RGBA على الويب.

### 5.4 خيارات ثانوية مذكورة للاكتمال

- **Flutter `video_player`**: ممتاز للواجهة لكنه يكرّر مكدّس الوسائط؛ يُفضّل كـ غلاف فقط إذا كانت تجربة Flutter مطلباً مستقلاً.
- **React Native module**: ممكن لكنه الأعلى جهداً للربط والأقل عائداً على فجوة العرض.
- **Qt / Avalonia / MAUI / Electron**: قابلة تقنياً عبر cdylib، لكنها تُدخل جسراً إضافياً دون مكاسب عرض مقابل الخيارات 1–3.
- **Vue/Reactمجرد ويب**: الأرخص لكنه يغطّي الويب فقط ويُبقي النسخة RGBA.

### 5.5 قاعدة غير كاسرة إلزامية لكل المقترحات

1. لا تغيير لأي توقيع في `src/widget.rs` أو `src/lib.rs`.
2. لا متغيرات جديدة في `VideoState`، ولا حقول جديدة في structs عامة.
3. أي خطاف جديد يأخذ implementation افتراضية أو يعيش في حزمة منفصلة.
4. تبقى `Video`/`VideoWithControls` هي الواجهة الافتراضية، والمسار الجديد اختياري.
5. لا يُسجّل أي مقترح كـ «تم اختباره» حتى يُقاس على جهاز/متصفح فعلي.

---

## 6. الترتيب حسب `MINPAT chịu تكلفة`

> المعيار: أقل تعديل على الواجهة العامة، وأقل ازدواج لمكدّس الوسائط، وأقرب مسار إلى طائرات NV12 الموجودة، ثم الجهد والغطية. الدرجات تقدير مقارن وليست قياساً.

| الرتبة | الخيار | لماذا في هذا الموضع | التكلفة/الخطر |
|---|---|---|---|
| 1 | **Makepad `media` + `VideoFrameSession`** | يلمس فجوة C1 مباشرة، نفس اللغة، واجهة استقبال طائرات جاهزة، ولا يكرّر مفكّكاً | منخفضة؛ تحتاج قياس عرض لاحقاً |
| 2 | **روابط UniFFI للنواة** | يعيد استخدام كل الخلفيات بما فيها iOS، ويخدم SwiftUI/Compose دون كتابة مفكّك جديد | متوسطة؛ نقل الإطارات عبر FFI يحتاج قياساً |
| 3 | **Tauri/WebView غلاف** | أسرع تغطية سطح المكتب والويب بأقل تعديل، ويكمل مسار wasm الحالي | متوسطة؛ يعتمد على فكّ WebView ولا يحل RGBA |
| 4 | Flutter `video_player` غلاف | واجهة ممتازة لكنها تكرّر المشغّل | متوسطة-عالية |
| 5 | React Native NativeModule | جهد جسر مرتفع وعائد عرض محدود | عالية |
| 6 | Qt / Avalonia / MAUI / Electron | مرنة لكنها تُضيف طبقة ربط بلا مكسب عرض | عالية |
| 7 | Vue/React web-only | أرخص تكلفة لكن أضيق تغطية ويُبقي النسخة RGBA | منخفضة التكلفة / محدودة الأثر |

**أفضل 3 للدمج:** Makepad media، ثم UniFFI، ثم Tauri/WebView.

---

## 7. ملاحظات وصولية وتجريبية موجزة

- عناصر التحكم الحالية أزرار بنص (`Button` + `Text`) في `src/widget.rs:224-287`؛ أي غلاف جديد يجب أن يحافظ على تسميات قابلة للقراءة وترتيب منطقي ثابت، دون تغيير التوقيع. `[مؤكّد كوصف]`
- لم يُجرِ تدقيق وصولية كامل على الألوان/التباين/تنقل لوحة المفاتيح في هذه الجولة؛ أي حكم WCAG هنا `[غير مُختبَر]`.
- عند إضافة مسار Makepad/UniFFI، يُنصح بقياس: زمن الطائرة→النسيج، معدل الإطارات المسقوطة على الشاشة، واستهلاك الذاكرة عند 1080p60، مع تسجيل أن `stutter_probe` لا يغطي العرض (C3). `[فرضية منهجية]`

---

## 8. المصادر

### 8.1 مصادر رسمية نجح الوصول إليها

- Flutter platforms: https://docs.flutter.dev/reference/supported-platforms
- Flutter video cookbook: https://docs.flutter.dev/cookbook/plugins/play-video
- React Native getting started: https://reactnative.dev/docs/getting-started
- React Native next getting started: https://reactnative.dev/docs/next/getting-started
- Android Media3/ExoPlayer: https://developer.android.com/media/media3/exoplayer
- Jetpack Compose: https://developer.android.com/jetpack/compose
- Qt supported platforms: https://doc.qt.io/qt-6/supported-platforms.html
- Qt Multimedia: https://doc.qt.io/qt-6/qtmultimedia-index.html
- .NET MAUI: https://learn.microsoft.com/en-us/dotnet/maui/what-is-maui?view=net-maui-9.0
- Avalonia docs: https://docs.avaloniaui.net/docs/
- Electron docs: https://www.electronjs.org/docs/latest
- Tauri v2: https://v2.tauri.app/ و https://v2.tauri.app/start/
- Vue: https://vuejs.org/
- React Learn: https://react.dev/learn
- MDN `<video>`: https://developer.mozilla.org/en-US/docs/Web/HTML/Element/video
- MDN canvas `getContext`: https://developer.mozilla.org/en-US/docs/Web/API/HTMLCanvasElement/getContext
- MDN WebCodecs: https://developer.mozilla.org/en-US/docs/Web/API/WebCodecs_API
- Microsoft Media Foundation: https://learn.microsoft.com/en-us/windows/win32/medfound/microsoft-media-foundation-sdk
- GStreamer `playbin`: https://gstreamer.freedesktop.org/documentation/playback/playbin.html
- Apple AVFoundation (أرشيف — Intro): https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/AVFoundationPG/Articles/00_Introduction.html
- Apple AVFoundation (أرشيف — Playback): https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/AVFoundationPG/Articles/02_Playback.html
- Apple HTTP Live Streaming: https://developer.apple.com/streaming/

### 8.2 مصادر الشيفرة المباشرة

- Makepad README: https://raw.githubusercontent.com/makepad/makepad/master/README.md
- Makepad video player example manifest: https://raw.githubusercontent.com/makepad/makepad/main/examples/video_player/Cargo.toml
- Makepad video player example source: https://raw.githubusercontent.com/makepad/makepad/main/examples/video_player/src/main.rs
- Makepad widgets manifest: https://raw.githubusercontent.com/makepad/makepad/main/widgets/Cargo.toml
- Makepad media family manifest: https://raw.githubusercontent.com/makepad/makepad/main/widgets/families/media/Cargo.toml
- Makepad `video.rs`: https://raw.githubusercontent.com/makepad/makepad/main/widgets/families/media/src/video.rs
- Makepad `video_player.rs`: https://raw.githubusercontent.com/makepad/makepad/main/widgets/families/media/src/video_player.rs
- UniFFI README: https://raw.githubusercontent.com/mozilla/uniffi-rs/main/README.md

### 8.3 مصادر تعذّر أو عادت فارغة (سُجّلت للشفافية)

- `https://makepad.dev/book/` → 404؛ استُبدل بمصادر GitHub الخام أعلاه.
- `https://react.dev/` → 404؛ استُبدل بـ `react.dev/learn`.
- `https://gstreamer.freedesktop.org/documentation/playbin/index.html` → 404؛ استُبدل بمسار `playback/playbin.html`.
- `https://facebook.github.io/react-native/docs/getting-started.html` → 404؛ استُبدل بـ `reactnative.dev`.
- `https://v2.tauri.app/concept/embedded/` → 404؛ استُبدل بصفحتي Tauri الرئيسيتين.
- `https://www.electronjs.org/docs/latest/tutorial/electron-tutorial-images` → 404؛ استُبدل بجذر التوثيق.
- صفحات Apple الحديثة `avfoundation/avplayer` و`avplayerlayer` و`avassetreader` و`avfoundation` عادت بدون محتوى قابل للاستخدام؛ اعتُمد الأرشيف.
- `websearch` أعاد HTTP 402؛ كُل البحث الخارجي تم عبر `webfetch` المباشر.

---

## 9. الحكم النهائي

1. المشروع لا يعاني غياب تشغيل على iOS؛ خلفية AVFoundation موجودة ومُوجّهة لـ `target_os = "ios"`، لكنها **غير مُعاد التحقق على جهاز**. التوصية هي **تزويد iOS بسطح عرض يستقبل NV12** عبر الخيار 1 أو 2، لا استبدال المفكّك.
2. أكبر فجوة عرض هي `ImageBitmap::from_rgba8` لكل إطار في `src/widget.rs` مع نسخة RGBA إضافية على الويب؛ المسار مؤكّد، وتكلفته تحت القياس.
3. أفضل ثلاثة أدوات دمج، مرتّبة حسب `MINPAT chịu تكلفة`: **Makepad media/VideoFrameSession**، ثم **UniFFI**، ثم **Tauri/WebView**.
4. كل ما سبق مقترحات إضافة فقط؛ لم يُعدّل أي ملف سوى هذا التقرير، ولم يُدعَ أي تشغيل حقيقي.
