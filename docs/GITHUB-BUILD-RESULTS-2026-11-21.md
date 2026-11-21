# نتائج البناء على GitHub — 2026-11-21

> كل البناء جرى على **GitHub Actions فقط** — لا بناء محلي (حسب طلب المستخدم).
> المستودع: `https://github.com/iq-858/counter`، الفرع `master`.

## 1. iOS (IPA) — ✅ نجح

- الالتزام المبني: `3f844ee` (إصلاح أخطاء compile في `apple_avplayer.rs`).
- الـ run: https://github.com/iq-858/counter/actions/runs/44897122847 (`cranpose-build/ios-1795258906861`)
- الأرشيف: `cranpose-ios-debug`
- الملف المُنزَّل: `dist/github-ios/cranpose-ios-debug/application.ipa`
- الحجم: 32,739,601 بايت
- SHA256: `E93DF9EA64ED1274C01305276FE527EDCD3FAA6344C4DCCACE7511076D2C90E0` (مطابق لـ `artifact.json`)
- المعرّف: `com.example.counter`

**مهم:** `artifact.json` يقول `"needs_device_signing": true` — هذا IPA من نوع debug
ويحتاج توقيع جهاز (provisioning profile + certificate) لتثبيته على iPhone. البناء
ناجح، لكن التثبيت النهائي يتطلب حساب Apple Developer. هذا ليس خللاً في الكود.

### محاولة فاشلة سابقة (سجلّ)
الـ run `44896224894` فشل بثلاثة أخطاء Rust أدخلتها تعديلات سابقة في
`apple_avplayer.rs`:
- `set_audible` كان داخل `impl VideoDecoder` بدل `impl AvPlayerDecoder`.
- `Instant` غير مستورد.
- `set_audible` غير موجود (نتيجة للأول).

أُصلحت في `3f844ee`. **الدرس:** `cargo check` على Windows لا يترجم مسار iOS،
فلم تكشفها الفحوص المحلية — بناء GitHub هو التحقق الحقيقي.

## 2. Android (APK) — قيد التنفيذ

أُضيف مشروع Gradle رسمي (مأخوذ من قالب `apps/android-demo/android` في مستودع
Cranpose) و workflow مخصص `.github/workflows/android-build.yml` لأن أداة
`cranpose-build --on github` تدعم Apple فقط.

التوقيع: الـ workflow يولّد keystore (RSA 2048، PKCS12، صلاحية 10 سنوات) عبر
`keytool` على الـ runner، يوقّع به `:app:assembleRelease`، يتحقق بـ `apksigner`,
ثم يرفع **APK الموقّع** و**keystore** كأرشيفين منفصلين.

محاولات سابقة (كلها أُصلحت):
1. `44897497636` — `android-actions/setup-android@v3` فشل على حزمة `tools` القديمة.
   → اعتماد SDK المثبّت مسبقاً واستخدام `sdkmanager` مباشرة.
2. `44897747904` — `./gradlew: Permission denied`.
   → `git update-index --chmod=+x android/gradlew` + استدعاء `bash ./gradlew`.
3. `44898100217` — `AppLauncher::try_run` غير موجود على Android.
   → استبعاد دالة `run()` المشتركة على Android (`#[cfg(not(target_os = "android"))]`)
   لأن `android_main!` هو نقطة الدخول هناك.

آخر run: `44898804928` (قيد التنفيذ لحظة كتابة هذا الملف).

## 3. إعادة مراجعة الادعاءات
انظر `cranpose-video/docs/REVIEW-RECHECK-2026-11-21.md` — تحقق مباشر من المصدر،
بدون وكلاء على نسخ خاطئة. الخلاصة: الادعاء 1 صحيح، 2 دقيق جزئياً (PiP مبالغ فيه
والـ handle يفتقر APIs)، 3 صحيح، 4 (مشكلة الوكلاء) مؤكدة.
