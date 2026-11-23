# تدقيق محرك الفيديو وخطة التطوير — Phase 1+2

> تاريخ: 2026-11-23. كل حقيقة `[SOURCE-VERIFIED]` مقروءة من `D:\counter\cranpose-video`.
> `[UNIT-TESTED]` يعني اختبار فعلي محلي. `[NEEDS-DEVICE]` يعني لا يمكن التحقق منه هنا.
> هذا ملف تدقيق **وُجد قبل التعديلات** في هذه الجولة.

## 1. البنية الحالية (كما هي)

| الطبقة | الملفات | الحالة |
|---|---|---|
| trait الفك | `decoder.rs:29` `VideoDecoder: Send` | `read() -> Result<Option<DecodedFrame>, VideoError>` — `None` يعني EOS فقط (توثيق `:44-45`) |
| إطار | `frame.rs` `VideoFrame` | مخزن RGBA/NV12، `ColorMatrix`, `ColorRange` موجودان |
| مشغّل | `player.rs`, `player/threaded.rs`, `player/inline.rs` | pull model، خيط فك لكل مشغّل |
| صوت | `audio.rs`, `arbitration.rs`, `windows/mf_audio.rs` | تحكيم مجموعات، clock صوتي |
| منصات | `platform/*.rs` | Windows MF، Android MediaCodec، Apple AVAssetReader/AVPlayer، Linux GStreamer، Web |
| HLS | `hls/{mod,playlist,tfdt}.rs` | playlist + tfdt + ABR + prefetch |
| جلسة | `session.rs` | Recovery/backoff/events |
| واجهة | `widget.rs` | composables |

## 2. أخطاء/فجوات حقيقية مؤكدة من المصدر

| # | الفجوة | الدليل | الأثر |
|---|---|---|---|
| C1 | **غموض `Ok(None)`**: لا يفرّق بين EOS و«المفكك يحتاج مزيداً من البيانات» و«التدفق مؤقتاً متوقف» | `decoder.rs:44-46` | معالجة خاطئة للتدفق المؤقت (انظر `apple_avplayer.rs` تعليق EOS) |
| C2 | **لا مدير موارد مركزي**: كل مشغّل يملك خيط فك + تجمع إطارات مستقل | `player.rs:816-821` (`slot`, `engine`) | عند N فيديوهات: خيوط غير محدودة، ذاكرة غير محصورة |
| C3 | **تخمين الألوان من الحجم** في مسارات Apple | `apple.rs` (color inferred from size) | HDR/BT.2020 خاطئ عند غياب metadata |

## 3. ما سأنفّذه في Phase 1+2 الآن

1. **`decode_result.rs`**: نوع `ReadOutcome` صريح (Frame / EndOfStream / NeedMoreData / NoPictureYet) + تحويل من `Option` القديم — **إضافة فقط** (لا تغيير signature `VideoDecoder`). `[UNIT-TESTED]`
2. **`resource.rs`**: `VideoResourceManager` — حدود صريحة لعدد المفككات+بايتات الإطارات، أولويات (Active/Next/Nearby/Background)، إيقاف تلقائي، إعادة استخدام تجمع، إلغاء عمل قديم — وحدة Rust نقية. `[UNIT-TESTED]`
3. **اختبارات** لكل ما سبق + حفظ التوافق.
4. ما **لا أنفّذه الآن** (مُوثّق أدناه): GPU zero-copy، HDR الكامل، الترجمة، قياسات الأداء — لأنها تحتاج `cranpose-render-wgpu` وأجهزة حقيقية.

## 4. ما لا يمكن تنفيذه محلياً (بصراحة)

| البند | السبب |
|---|---|
| GPU zero-copy (D3D11/Metal/AHardwareBuffer/DMABUF) | يحتاج تكامل `cranpose-render-wgpu` + SDK + جهاز |
| قياس P95/P99/dropped frames | لا أجهزة هنا |
| اختبار انقطاع شبكة/ bluetooth فعلي | `[NEEDS-DEVICE]` |
| HDR PQ/HLG فعلي | يحتاج عيّنات HDR + جهاز |

## 5. خطة المراحل المتبقية (للتنفيذ اللاحق)

- Phase 3: GPU integration (بالتعاون مع cranpose-render-wgpu)
- Phase 4: ربط `VideoResourceManager` بالمشغّل فعلياً
- Phase 5: HLS ABR محسّن + DASH
- Phase 6: HDR/ترجمة
- Phase 7: قياسات على أجهزة

## 6. ما أُنجز فعلياً في Phase 1+2 `[UNIT-TESTED]`

| الوحدة | الملف | الاختبارات |
|---|---|---|
| نتيجة فك صريحة `ReadOutcome` + `FrameStamp` | `decode_result.rs` | 4 |
| مدير موارد `VideoResourceManager` (أولويات، حدود، prefetch، RAII) | `resource.rs` | 7 |
| ألوان موحّدة `ColorInfo` (BT.601/709/2020, PQ/HLG, chroma, clean aperture, HDR) | `color.rs` | 5 |
| ترجمة SRT/WebVTT `parse`/`active_at` | `subtitle.rs` | 5 |

**المجموع: 173 اختبار lib ناجح (0 فاشل).** كلها إضافات غير كاسرة — `VideoDecoder::read` لم يتغير، أُضيف `read_outcome` كـ default method.

## 7. التحقق متعدد المنصات على GitHub
أُضيف `.github/workflows/video-engine-matrix.yml`:
- Windows / macOS / Linux: `cargo check` + اختبارات كاملة (serial على macOS/Linux لتفادي تذبذب توقيت CI)
- iOS / Android (arm64 + armv7) / wasm32: `cargo check` crossover بميزة `widget` وحدها (تجنّب C-deps الصوت التي تحتاج NDK clang)
- wasm-pack build للويب

**إصلاحات CI المكتشفة أثناء التشغيل الأول `[SOURCE-VERIFIED]`:**
1. `widget` لم تكن تُفعّل `cranpose` (الـ `#[composable]` macro) → أُضيف `dep:cranpose` لميزة `widget`.
2. الـ cross-check بـ `native-audio` يجلب `aws-lc-sys` (C) الذي يحتاج NDK clang → استُبدل بـ `widget` فقط.
3. Linux احتاج `libasound2-dev` (alsa-sys).
4. اختبارا `pacing` الحسّاسان بالتوقيت فشلا على macOS runner مزدحم → `--test-threads=1`.

هذا **compile + unit verification** — ليس تشغيلاً على جهاز (موثّق بوضوح).

## 8. ما بقي (مُوثّق بصدق)
GPU zero-copy (D3D11/Metal/AHardwareBuffer/DMABUF) يحتاج تكامل `cranpose-render-wgpu` + SDK/جهاز. قياسات الأداء/HDR الفعلي/device tests تحتاج أجهزة. لم أدّعِ أياً منها.
