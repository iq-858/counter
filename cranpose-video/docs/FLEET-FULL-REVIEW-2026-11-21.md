# المراجعة الشاملة للمكتبة — أسطول 9 وكلاء (2026-11-21)

> الوسوم: `[SOURCE-VERIFIED]` مقروء من المصدر/وثيقة، `[HYPOTHESIS]` يحتاج جهازاً، `[REPORT-ONLY]`.
> التعديلات مدفوعة إلى GitHub بدون بناء IPA.

## 1. سؤال: كم فيديو تستطيع المكتبة تشغيله في نفس اللحظة/النافذة؟

**الإجابة `[SOURCE-VERIFIED]` من وكيل ملاءمة المشاريع:** **متعدد** — كل `VideoPlayer` يملك `slot` و`engine` مستقلين (`player.rs:816-821`)، ولا يوجد مُفكِّك singleton. لاعبان يُفتحان ويُشغَّلان معاً في `tests/audio_group.rs:125-128`.

- قيد «مُفكِّك واحد لكل تطبيق» هو **اتفاقية تركيب Reels** (`docs/hardening-audit-plan:55,158`)، **ليس قيد مكتبة**.
- التحكيم بالمجموعات: group 0 (افتراضي) يُكتم فيه الأقدم، ومجموعات مختلفة تعمل معاً (`arbitration.rs:3-5,21`).
- المطالبة عند أول `play()` فقط، فالتحميل المسبق لا يُكتم الحالي (`audio.rs:432-434`).

## 2. جدول ملاءمة المشاريع `[SOURCE-VERIFIED]`

| نوع المشروع | الملاءمة | الدليل |
|---|---|---|
| Reels بمُشغّل نشط واحد | جيد | group 0 افتراضي |
| شبكة/معرض عدة لاعبين | جيد | `player.rs:816-821` + `tests/audio_group.rs` |
| تحميل المقطع التالي مسبقاً | جيد | claim عند play لا عند open |
| HLS مباشر | جيد/جزئي | لا `seek_to_live` عام |
| مُفكِّك مخصص | جيد | `VideoDecoder` trait + `with_decoder` |
| السرعة/التكرار | جيد | `set_speed`/`set_looping` |
| PiP/خلفية | جزئي | الجلسة محلية، لينكس خارج |
| محرر دقيق الإطارات | جزئي | `seek`/`take_frame` بلا frame-step |
| صوت فقط | ضعيف | لا نوع `VideoSource` صوتي |
| مصادر متسابقة (first-frame-wins) | ضعيف | لا primitive لذلك |

## 3. ما تفتقره المكتبة (يُستكمل من الوكلاء)
- قيد إمكانية `VideoPlayerHandle`: لا يعرض `set_audio_group`/`take_frame`/`close`/`with_decoder` (`widget.rs:48-149`) — يُصلح بإضافات.
- لا `seek_to_live` عام، لا frame-step، لا audio-only، لا source racing.
- الجلسة محلية (ليست OS media session)، لينكس خارجها.

## 4. نتائج الأسطول الكامل (اكتمل — 9 + دفعة إضافية)

| الوكيل | الحكم المختصر |
|---|---|
| مهندس معماري | **SOUND** لكن ليس جاهزاً للإنتاج؛ أصلحتُ تسريب توكن المجموعة عند إعادة التعيين |
| أداء | بطء عند `open` المتزامن على خيط التركيب، و`take_frame` لا يُعيد الـ recycle |
| شبكة | تصنيف الخطأ يُمحى بين الطبقات؛ أصلحتُ اكتشاف كل 4xx دائمة |
| iOS | فجوات audio session/PiP/interruption/HDR (معظمها مسؤولية التطبيق) |
| أندرويد | BT2020/full-range/cropRect/audio focus (بعضه `(HYPOTHESIS)`) |
| مراجع عدائي | بنية سليمة + حواجز: بلا إثبات جهاز، parity، تسريب توكن (أُصلح)، Windows decode لا يُبلّغ الجلسة (أُصلح) |
| QA | 212 اختباراً، تغطية GOOD للخيوط/الصوت، NONE لـ iOS AVPlayer وJNI |
| بحث مقارن | GStreamer/ExoPlayer/AVPlayer توفّر buffering ناضج؛ تبنٍّ عبر `VideoDecoder` |
| أمن | CDN script بلا SRI (web)، تسريب توكن في `VideoError` المُعاد، SSRF عبر playlist، ملفات مؤقتة بصلاحيات عالمية |

## 5. ما أصلحته فعلياً هذه الجولة `[VERIFIED]`
- `arbitration.rs`: أعدت التصميم بمكدس توكنات → **ترقية** الباقي عند إغلاق المالك + حذف المفتاح فارغاً (يمنع البقاء مكتوماً/Nمو غير محدود).
- `audio.rs`: شرط `arbitrate` كان معكوساً (لا كتم إطلاقاً) → أصلحته؛ `audible_volume` في كل مسارات الصوت؛ `release` عند `detach`/`set_group`؛ المطالبة عند `play` لا `sync_clock`.
- `apple_avplayer.rs`: المطالبة عند `play` لا عند الفتح؛ `release` عند `close`؛ إعادة تعيين المجموعة تحرّر القديم.
- `windows.rs`: `session.failed("mf.decode")` عند فشل الفك (كان بلا استعادة)؛ إصلاح `report_decode_failure` واستثناء JNI؛ `getSampleTime` يمسح الاستثناء.
- `hls/mod.rs`: كل أخطاء 4xx (عدا 408/425/429) تُصنَّف دائمة → لا حلقة إعادة محاولة إلى الأبد.
- وثائق: `HLS-IOS-REVIEW.md`وسمته تاريخياً `[REPORT-ONLY]`؛ `README.md` صحّحت `PARALLEL_MIN_PIXELS=3840*2160` (التقسيم من 4K لا 1080p).

## 6. اختبارات
152 lib + 6 resilience + 4 audio_group + 5 audio_route_switch، كلها خضراء. وثائق جديدة: `PRODUCTION-COOKBOOK.md`، `CROSS-PLATFORM-PARITY.md`.

## 7. حكم نهائي
البنية **سليمة** لكن **ليست جاهزة للإنتاج** بدون إثبات جهاز حقيقي (iOS/Android/Windows/Linux) وسد فجوات parity (audio focus أندرويد، AVAudioSession iOS). لا حكم نهائي بالجاهزية قبل اختبارات الجهاز.
