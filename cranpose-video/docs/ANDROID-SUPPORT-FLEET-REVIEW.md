# مراجعة دعم أندرويد — أسطول الوكلاء (قراءة فقط + مصادر رسمية)

> التاريخ: 2026-11-21. 7 وكلاء بمهام متمايزة، كلهم مأمورون بمراجعة مصادر Android الرسمية وتصنيف `VERIFIED`/`HYPOTHESIS`.
> لا تعديلات كود من الوكلاء؛ التعديلات الوحيدة هي مني بعد التقرير.

## 1. ما أكده وكيل التشغيل الخلفي [SOURCE-VERIFIED من developer.android.com]

| الحقيقة | المصدر |
|---|---|
| أندرويد 8+ يوقف خدمات الخلفية بعد دقائق؛ تشغيل صوت يجب أن يكون foreground service | developer.android.com/about/versions/oreo/background |
| أندرويد 14+ يتطلب نوع FGS `mediaPlayback` وصلاحية `FOREGROUND_SERVICE_MEDIA_PLAYBACK` | developer.android.com/develop/background-work/services/fgs/service-types |
| تشغيل صوت خلفي يحتاج `MediaSessionService` + إشعار، ويُوقف بعد 10 دقائق توقف | developer.android.com/media/media3/session/background-playback |
| Doze يُعلّق الشبكة ويتجاهل wake locks؛ الصوت يدير wake lock تلقائياً | developer.android.com/training/monitoring-device-state/doze-standby + /training/scheduling/wakelock |
| تركيز الصوت: `AUDIOFOCUS_GAIN`، تحقق من المنح قبل التشغيل، إيقاف عند فقدان دائم؛ API 35+ يفشل الطلب إلا لتطبيق أعلى/ FGS | developer.android.com/media/optimize/audio-focus + /reference/android/media/AudioFocusRequest |
| PiP هو البديل الموصى به لفيديو خلفي | fgs/service-types |

**الخلاصة `[VERIFIED]`:** هذه مسؤوليات **التطبيق** (manifest، FGS، إشعار، PiP). المكتبة لا تستطيع إعلانها. لكن المكتبة تستطيع: إدارة التركيز، الإيقاف/الاستئناف عند دورة الحياة، وعدم محاربة Doze.

## 2. الفجوة الحقيقية في المكتبة (من وكلاء سابقين + هذا الأسطول)

- `InterruptionReason::AudioFocusLoss` مُعرَّف لكن **بلا مستدعٍ إنتاجي** على أندرويد.
- لا يوجد `AudioManager.requestAudioFocus` داخل المكتبة — التركيز يُدار (إن كان) في `cranpose-media`/`cranpose-services`.
- `PlaybackSession::route_changed()` موجود لكن لا مستدعٍ على أندرويد (عكس iOS حيث أضفنا مراقب AVAudioSession).
- التحكيم بالمجموعات (`arbitration.rs`) يعمل الآن على مسار `SoftwareMediaPlayer` العام عبر `Audio::arbitrate()`، وهذا هو مسار أندرويد (لأن `Reader.carries_audio()=false` فيُزاوج بصوت عام).

## 3. ما لا تستطيع المكتبة إصلاحه `[SOURCE-VERIFIED]`
- إعلان `FOREGROUND_SERVICE`/`FOREGROUND_SERVICE_MEDIA_PLAYBACK` — manifest التطبيق.
- تشغيل FGS/إشعار الوسائط — التطبيق.
- تجاوز تركيز النظام/Doze — سياسة النظام.
- PiP — التطبيق.

## 4. ما تفعله المكتبة بالفعل (بعد إصلاحاتي)
- إيقاف/استئناف عند دورة الحياة عبر `play`/`pause` (Command).
- تحكيم صوت بمجموعات: group 0 = الأحدث يفوز تلقائياً (Reels)، مجموعات مختلفة = مقطعان معاً.
- كتم عبر volume=0 لا `stop()` (يحفظ إمكانية الاستئناف/الربط).

## 5. بانتظار بقية الوكلاء
- JNI lifecycle (15+ finding) — ses_e02e397e6f
- Audio focus/AAudio مع مصادر رسمية — ses_e02e2bb2c
- توافق الصيغ/NV21 — ses_e02e27e3
- مراجعة عدائية للتحكيم على أندرويد — ses_e02e208a
- اختبارات arbitration — ses_e02e1cfe

سأدمج نتائجهم وأطبّق ما يثبت ثم أدفع إلى GitHub بدون بناء.
