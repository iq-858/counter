# إصلاحات الهاتف — 2026-11-22 (دفع بدون بناء)

## 1) iOS: توقف الفيديو والصوت عند فصل سماعة الرأس

**السبب الجذري `[SOURCE-VERIFIED]`:** `AVAudioSessionRouteChangeNotification`
يستدعي `route_changed_all()`، الذي كان يُدخل كل الجلسات في `Recovering`
(`session.rs:279`). هذا يُجبر الـ worker على إيقاف نشر الإطارات والانتظار،
فتتوقف الصورة والصوت معاً، ولا يعودان إلا بإعادة فتح المقطع.

**الإصلاح:**
- `session.rs`: أضيفت `ROUTE_CHANGE_GENERATION` (عدّاد ذري). `route_changed_all()`
  تزيده وتسجّل `SessionEvent::RouteChanged` فقط — **بدون** `Recovering`.
- `audio.rs`: أضيف حقل `route_generation` وعدّل `observe_output_route()` على iOS
  لإعادة ربط الصوت العام فقط عند تغيير العدّاد. AVPlayer يتجاهل ذلك لأن
  AVAudioSession يعيد توجيهه تلقائياً.
- النتيجة: تغيير المسار لا يوقف تدفق الإطارات أبداً؛ الصوت العام يُعاد ربطه
  تلقائياً؛ AVPlayer يواصل بصوت عبر المسار الجديد.

## 2) Android: خروج التطبيق فور الإقلاع

**التحليل `[SOURCE-VERIFIED]` من الـ APK المبني:**
- الـ manifest مدمج وسليم (Activity + MAIN/LAUNCHER + exported + lib_name=counter).
- `libcounter.so` موجود للـ ABIs ورمز `android_main` موجود.
- الخدمة `network` والأذونات سليمة.
- **الأصول (ملفات MP4) غير مضمّنة في APK**، و`resolved_source` على Android
  كان يستخدم `current_exe().parent()/assets/...` — مسار لا يمكنه قراءة أصول APK
  (`std::fs` لا يقرأ `assets/`؛ يلزم AssetManager).

**الإصلاح:**
- `app_model.rs`: أُضيفت `android_asset_path()` على Android تستخدم
  `include_bytes!` لتضمين المقاطع في الـ binary وكتابتها إلى مجلد قابل للكتابة
  (`temp_dir`) عند الطلب، مع اسم مشتق من تجزئة ثابتة لإعادة الاستخدام.

**تحذير صادق:** الخروج الفوري قد يكون له سبب وقت تشغيل إضافي لا يظهر بالفحص
الساكن (مثل panic في AVAudioSession/JNI). **يلزم logcat من جهاز حقيقي** لتأكيد
زوال الخروج:
```
adb logcat | grep -i "AndroidRuntime\|cranpose\|FATAL\|panic"
```
إن بقي الخروج بعد هذا الإصلاح، أرسل السطر الذي يظهر قبل الخروج لأحدده بدقة.

## التحقق
- `cargo check` (Windows): ناجح بلا أخطاء.
- اختبارات audio_route_switch: 5/5 ناجحة.
- لم يُبنَ أي APK أو IPA (حسب طلب المستخدم).
