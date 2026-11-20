# مذكّرة قرار تنفيذية — `cranpose-video`
**إلى المالك (Owner) · 2026-11-20 · صفحة واحدة · عربي بمصطلحات إنجليزية**

> **النطاق:** كُتب هذا الملف فقط؛ لا تعديل شيفرة، لا commit/push/PR، لا أي عمل على Linux (`docs/PRODUCTION-MASTER-PLAN.md:5`؛ `docs/final-report-ar.md:5`).
> **الوسوم:** `[VERIFIED]` = مقروء من مصدر/وثيقة مع `file:line`؛ `[HYPOTHESIS]` = نتيجة جهاز/شبكة/زمن تشغيل غير مقاسة؛ `[REPORT-ONLY]` = رقم تاريخي من سجلّات مفقودة في هذه الشجرة (لا يُقدَّم كدليل حالي)؛ `[MEASURED — TO BE FILLED]` = خانة قياس فارغة لا تُختلَق (`docs/final-report-ar.md:7`؛ `docs/PRODUCTION-MASTER-PLAN.md:10-20`).

---

## 1. إجماع 30+ agent حول تكافؤ Android/iOS
`[VERIFIED]` عبر بيانات المهارات/المراجعين في `docs/phone-integration-review.md:37-62`، `docs/cross-platform-ui-video-review.md:11-35`، `docs/rust-code-architecture-review.md:27-29`، `docs/desktop-integration-review.md:31-33`.

| البند | الحكم |
|---|---|
| وجود الـ decoder | `[VERIFIED]` كلتاهما مدعومتان في الكرايت: iOS عبر AVFoundation (`Cargo.toml:97-134`؛ `docs/cross-platform-ui-video-review.md:85,93-95`)، Android عبر MediaCodec/JNI (`Cargo.toml:144-146`). لا «غياب خلفية» على أيٍّ منهما. |
| تكافؤ وقت التشغيل | **[HYPOTHESIS — غير مُثبت]** ولا يُدّعى: لم يُجرَ تشغيل على أي جهاز؛ بناء Android محجوب لغياب NDK، ولا جهاز iOS (`docs/android-test-checklist.md:3-8`؛ `docs/final-report-ar.md:98,186`؛ `docs/mobile-platform-audit.md:133-138`). |
| تكافؤ العرض | `[VERIFIED]` المسار نفسه على الاثنين: NV12 → RGBA على الـ CPU ثم `ImageBitmap::from_rgba8` لكل إطار؛ لا سطح/نسيج أصلي (`src/widget.rs:161,171-176`؛ `docs/phone-integration-review.md:85-90`؛ Android `null Surface` في `android.rs:241-253`). فجوة العرض مشتركة، لا ميزة لأحدهما. |
| ترتيب الخطر | `[VERIFIED]` Android أعلى خطراً (حرجة: ترتيب الكروما/اللون)، ثم iOS (عالية: لون ثابت + aperture)، ثم Linux/HLS (`docs/mobile-platform-audit.md:216-219`). |

**الخلاصة:** تكافؤ *معماري/مصدر* نعم `[VERIFIED]`؛ تكافؤ *ميداني* غير مُثبت `[HYPOTHESIS]` حتى القياس على جهاز.

## 2. عوائق P0 — ثلاثة لكل منصة (SOURCE-VERIFIED، الأثر الجهازي HYPOTHESIS)

| # | Android — الحاجز والمرجع | # | iOS — الحاجز والمرجع |
|---|---|---|---|
| A1 | افتراض NV12 (Cb→Cr) أثناء نسخ الكروما مع سقوط `COLOR_STANDARD≠1` إلى Bt601 وفقدان BT2020/PQ/HLG — `docs/mobile-platform-audit.md:96` (AND-1)؛ `src/platform/android.rs:681-733,830-834,881-890`؛ خطة المرحلة 1.1–1.2 `docs/PRODUCTION-MASTER-PLAN.md:87-88`. | I1 | تثبيت تنسيق الـ output ونطاق الفيديو مع تجاهل بيانات اللون/HDR الفعلية — `docs/mobile-platform-audit.md:86` (IOS-1)؛ `src/platform/apple.rs:392-394,549-565`؛ المرحلة 2.1 `docs/PRODUCTION-MASTER-PLAN.md:112`. |
| A2 | نسخ لا يحترم `rowStride/pixelStride/cropRect` → تشويه/انقلاب قنوات — `:97` (AND-2)؛ `android.rs:681-733`؛ المرحلة 1.3 `:89`. | I2 | نسخ من أبعاد السطح الخام بلا `clean aperture`/crop — `:87` (IOS-2)؛ `apple.rs:302-368,65-79`؛ المرحلة 2.2 `:113`. |
| A3 | `Ok(None)` مؤقت يُقرأ كـ EOF فيُوقف التشغيل خطأً — `:98` (AND-3) + THR-2 `:125`؛ `android.rs:589-591,625-627`؛ `src/player/threaded.rs:224-233`؛ المرحلة 1.4 `:90`. | I3 | `cancelReading()` في `open_at` **أُصلح في المصدر لكنه غير مُختبَر على جهاز** — `:88` (IOS-3)؛ `apple.rs:174-199,396-402`؛ المرحلة 2.4 `:115`. |

> `[VERIFIED]` بوابة مشتركة إضافية: لا إثبات جهازي لأيٍّ من البنود أعلاه (`docs/android-test-checklist.md:3-8`)؛ `[MEASURED — TO BE FILLED]` في `docs/PRODUCTION-MASTER-PLAN.md:94,122`.

## 3. هل ينتقل نمط إصلاح صوت Windows؟
**الجواب: انتقال تصميمي جزئي فقط — لا انتقال تنفيذي كامل `[VERIFIED]`، والاستعادة المسموعة `[HYPOTHESIS]`.**

- نمط Windows (`[VERIFIED]`): تصنيف عدم تطابق PCM كـ `Unavailable` قابل لإعادة المحاولة، فحص مسار استباقي كل 500 ms، تعداد أجهزة إخراج حقيقية بدل المقبض الافتراضي المتجمّد، ومصدر فشل واحد لتفادي مضاعفة الـ backoff — `docs/audio-route-switch-diagnosis-2026-11-20.md:20-49`؛ `docs/audio-route-switch-diagnosis-round2-2026-11-20.md:19-32`؛ `src/platform/windows/mf_audio.rs:119-137,1140-1156`؛ `src/platform/windows.rs:707-760`؛ `src/session.rs:539-565`.
- ما ينتقل فعلاً `[VERIFIED]`: منطق «الفحص الدوري للمسار من خيط الفك» موجود أصلاً في المسار العام cpal (`src/audio.rs:49-97,242-301,355-405,727-747`؛ `docs/final-report-ar.md:184`) ويخدم macOS/Linux.
- ما لا ينتقل `[VERIFIED]`: تعداد أجهزة WASAPI خاص بـ cpal على Windows؛ استبُعد iOS من تبعية cpal لأن `AVAudioEngine` يدير المسار (`Cargo.toml:86-93`)؛ فقدان تركيز صوت Android `InterruptionReason::AudioFocusLoss` **مُعرَّف لكن غير موصول بأي مُستدعٍ** (`src/session.rs:65-70`؛ `docs/final-report-ar.md:196`؛ `docs/PRODUCTION-MASTER-PLAN.md:33,93`).
- `[HYPOTHESIS]` زمن التبديل «<500 ms» تقدير منطقي وليس رقماً مقاساً (`docs/audio-route-switch-diagnosis-round2-2026-11-20.md:54-56`). يجب توصيل الـ hook على Android وقياسه على جهاز قبل الادّعاء.

## 4. الحكم الصادق على استهلاك الموارد (بلا أرقام مختلقة)
- `[VERIFIED]` حقائق بنيوية فقط: `MAX_WORKERS=8` و`PARALLEL_MIN_PIXELS=1920*1080` (`src/pixels.rs:9-13,129-154`)؛ `SPARES=2` (`src/player.rs:44-48`)؛ خيط فك واحد/مشغّل (`src/player/threaded.rs:66-68`)؛ خيط prefetch واحد/بث + حتى 4 عمال جلب (`src/hls/mod.rs:631-632,1343-1383`)؛ bitmap لكل إطار (`src/widget.rs:161,171-176`).
- `[REPORT-ONLY]` جدول CPU/ذاكرة مضيف Windows القديم (مثلاً 5094 ms CPU/113 MB ذروة لـ `pixel-bench`) منقول من `docs/final-report-ar.md:88`، وسجلّاته غير موجودة في هذه الشجرة (`:7`) — **لا يُقدَّم كرقم حالي ولا كدليل على الجوال**.
- `[HYPOTHESIS]` تقديرات من الثوابت لا من قياس: NV12 1080p ≈3.1 MB، RGBA ≈8.3 MB، «عشرات الميجابايتات لكل مشغّل»، وارتفاع حاد في الذروة مع عدة مشغّلات + 8 عمال (`docs/mobile-platform-audit.md:164-171`؛ صيغة الخيوط `:155-162`). لا مقياس طاقة/حرارة على المضيف (`docs/final-report-ar.md:98,100,186`).
- **الحكم:** لا يوجد رقم موثوق لاستهلاك الموارد على Android/iOS اليوم. أي ادعاء بـ«طاقة أقل» هو `[HYPOTHESIS]` مشتق من تقليل عمل الـ CPU، وليس قياساً. تبقى الخانات `[MEASURED — TO BE FILLED]` (`docs/mobile-platform-audit.md:173`) حتى تشغيل بأمر+بيئة+مخرجات مسجّلة.

## 5. توصية الويدجت — أدوات iOS-style إضافية داخل تبعية `cranpose-ui` الحالية
- `[VERIFIED]` القدرات المطلوبة موجودة أصلاً في الطبقة الأساسية (`mute/volume/loop/speed/seek/stop`) لكن الشريط الحالي يعرض play/pause و±5s وrestart والسرعة فقط — `docs/WIDGET-CONTROLS-DESIGN.md:26-39`؛ `src/widget.rs:39-150,191-272`؛ `src/player.rs:64-81,1036-1053`. لا حاجة إلى مفكّك أو تبعية UI جديدة.
- **التوصية (تصميم `[HYPOTHESIS]`):** إضافة فقط بأسماء جديدة داخل `src/widget.rs` + طريقة `stop()` واحدة على `VideoPlayerHandle` + سطر re-export في `src/lib.rs:57-61`: `ProControl`، `GlassStyle`، `VideoControlsOptions`، `VideoControlsPro`، وأزرار `VideoMuteButton/VideoVolumeSlider/VideoLoopButton/VideoStopButton/VideoSpeedButton`، مع إعادة استخدام `VideoProgressSlider` و`SEEK_STEP=5s` — `docs/WIDGET-CONTROLS-DESIGN.md:143-265,276-283`؛ مرحلة API `docs/PRODUCTION-MASTER-PLAN.md:127-160`.
- `[VERIFIED]` قيد غير قابل للكسر: صفر تعديل على التواقيع والثوابت القائمة (`SEEK_STEP=5s`, `PROGRESS_TRACK_HEIGHT=4.0`, `PROGRESS_THUMB=12.0` في `src/widget.rs:14-21`) ولا تبعية خارج `cranpose-ui` (`Cargo.toml:19,47`)؛ `docs/PRODUCTION-MASTER-PLAN.md:43-52,157-160`.
- `[HYPOTHESIS]` Blur/material حقيقي أو إيماءات (tap/pinch) تعتمد على primitives غير مؤكدة في `cranpose-ui` المُثبَت — لا تُخترع واجهة؛ يُكتفى بـ `Button`/`Slider` (`docs/WIDGET-CONTROLS-DESIGN.md:45,154`).

## 6. ما لا يمكن إصلاحه داخل هذا الكرايت
| الصنف | الحقيقة والمرجع |
|---|---|
| **app-side route observer / GPU init** | `[VERIFIED]` إصلاح `DeviceCreate` (`adapter_limits: true`) و deadlock GLES و feature `diagnostics` تقع في تطبيق الجذر `D:\counter\src\main.rs` و`D:\counter\Cargo.toml:8`؛ جُرّبت مؤقتاً ثم **أُعيدت** لأن النطاق `cranpose-video` فقط (`docs/final-report-ar.md:106-162`، خصوصاً `:130-145,156`)؛ تدوير النافذة/dورة النشاط خارج الشجرة (`docs/phone-integration-review.md:54`) ومضخّات Reels في `D:\counter\src\app_ui\reels.rs:115-116,175` (`docs/final-report-ar.md:181`). |
| **hardware proof** | `[HYPOTHESIS]` حتى يتوفر جهاز: لا NDK، لا جهاز iOS، لا مقياس طاقة؛ إثبات تبديل الصوت، لون/ترتيب كروما على عتاد، `cancelReading` عند seek، HLS على شبكة حية، GPU/compositor، والطاقة/الحرارة كلها معلّقة (`docs/android-test-checklist.md:3-8`؛ `docs/final-report-ar.md:98,186`). |
| **pinned registry deps** | `[VERIFIED]` الإصدارات مثبّتة في `Cargo.toml:43-47` (`cranpose*` 0.24.0) و`Cargo.lock:759-760,865-866,1070-1071,1150-1153` (`cranpose-wgpu` 30.0.30)؛ تغييرها قرار تبعية/سجل خارج نطاق الإصلاح المسموح، إذ سُمح فقط بسطر تبعية `cranpose-video` في الجذر `D:\counter\Cargo.toml:13` (`docs/final-report-ar.md:145`)؛ كذلك `D:\counter\examples\probe.rs` خارج الكرايت (`:22`). |

## 7. القرار: **NO-GO للإنتاج غير المشروط · CONDITIONAL GO للإنتاج المشروط**
`[VERIFIED]` لا توجد اليوم أدلة جهازية، ولا أرقام موارد، وثلاثة عوائق P0 مفتوحة لكل منصة؛ لذلك **لا يُوقَّع إنتاج كامل الآن**. يُسمح بـ **GO WITH CONDITIONS** عند تحقيق كل ما يلي:

1. بوابات المرحلة 0 (لقطة أساس + أوامر مسجّلة) وتوقيع العقد غير الكاسر C1–C8 من المالك (`docs/PRODUCTION-MASTER-PLAN.md:37-76`).
2. إضافة تطبيع Android (A1–A3) وiOS (I1–I3) خلف قيم افتراضية + حراس device-free، بلا كسر `output_format()` أو `VideoFrame::from_nv12` (`:79-124`، C7 `:49`).
3. تشغيل قوائم الأجهزة وملء `[MEASURED — TO BE FILLED]`: تبديل صوت مسموع، لون/كروما على عتاد، seek بعد `cancelReading`، HLS حيّ، حرارة/طاقة (`docs/android-test-checklist.md:21-81`؛ `docs/PRODUCTION-MASTER-PLAN.md:214-222`).
4. الويدجت **additive فقط**: صفر تعديل على القديم، إعادة استخدام `cranpose-ui`، اختبارات composable جديدة (`docs/WIDGET-CONTROLS-DESIGN.md:289-314`).
5. إسناد «ما لا يمكن إصلاحه هنا» (القسم 6) إلى مالكي الجذر/التبعيات؛ لا يُنتظر من هذا الكرايت.
6. لا commit/push/PR دون إذن صريح؛ التغييرات تبقى غير مُودَعة (`docs/PRODUCTION-MASTER-PLAN.md:194,203`؛ `docs/final-report-ar.md:5`).

> **الخلاصة للقارئ:** المكتبة سليمة معمارياً ومُصلَحة على مستوى المصدر، لكن الحكم النهائي مشروط بإثبات جهازي. `[VERIFIED]` ما لدينا هو الشيفرة والمنطق؛ `[HYPOTHESIS]` كل ما يتعلق بالأداء والصوت المسموع والطاقة على Android/iOS.

**مسار الإخراج:** `D:\counter\cranpose-video\docs\EXECUTIVE-DECISION-MEMO.md`
