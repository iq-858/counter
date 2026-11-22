# تشخيص خروج تطبيق Android فور الإقلاع — 2026-11-22

> تحليل من الـ APK المبني + المصادر الرسمية + كود المشروع. الوسوم:
> `[SOURCE-VERIFIED]` مقروء مباشرة، `[HYPOTHESIS]` يحتاج جهازاً/logcat.

## 1. العَرَض
المستخدم: «عند الدخول إلى التطبيق يخرج مباشرة، والواجهة لا تُرسَم إطلاقاً».

## 2. الاستبعاد بالفحص المباشر للـ APK
فكّفتُ `app-release.apk` وفحصتُ كل طبقة الإقلاع:

| الطبقة | الفحص | النتيجة |
|---|---|---|
| **AndroidManifest المدمج** | استخرجتُ الـ AXML وقرأتُ الـ strings | ✅ `dev.cranpose.android.CranposeActivity` + `MAIN` + `LAUNCHER` + `exported=true` + `android.app.lib_name=counter` + `INTERNET`/`ACCESS_NETWORK_STATE` — سليم |
| **المكتبة الأصلية** | `lib/arm64-v8a/libcounter.so`, `lib/x86_64/libcounter.so` | ✅ موجودة |
| **الرموز** | فحص جدول الرموز | ✅ `android_main`، `ANativeActivity_onCreate`، `Java_dev_cranpose_android_*` كلها موجودة |
| **`android_main!`** | `cranpose-0.24.0/src/android_entry.rs:21-57` | ✅ يولّد `pub fn android_main(app: AndroidApp)` صحيحاً |
| **`loadNativeLibrary`** | `CranposeActivity.java:2601-2626` | ✅ يقرأ `lib_name` من الـ meta-data ويستدعي `System.loadLibrary` |
| **features/خدمات** | سجل البناء: `counter for arm64-v8a with the release profile and android` | ✅ سليمة |

## 3. السبب الجذري `[SOURCE-VERIFIED]`
الـ APK كان يحوي **ABIs اثنين فقط**: `arm64-v8a` و `x86_64`.

- الجهاز ذو الـ **32-بت (armeabi-v7a)** لا يجد `libcounter.so` الخاص به.
- `System.loadLibrary("counter")` يفشل بـ `UnsatisfiedLinkError` (`CranposeActivity.java:2622`).
- `loadNativeLibrary()` تُعيد `false` → الـ `NativeActivity` لا يجد رمز `android_main` →
  **لا يبدأ أي رسم، ويُغلق النشاط فوراً**.
- هذا يفسّر لماذا **لا تظهر واجهة ولا خطأ**: الفشل يحدث في طبقة Java قبل أن
  يبدأ أي كود Rust، فلا يوجد كود تطبيق ليعرض رسالة.

> ملاحظة: الأصول المفقودة (MP4) كانت مشكلة حقيقية لكنها **ليست** سبب الخروج
> الفوري — لأن `ActiveReel` (`reels.rs:140-148`) يتعامل مع فشل فتح الفيديو بأمان
> ويسجّله كخطأ، ولا يُخرج التطبيق.

## 4. الإصلاح المطبّق
- `android/app/build.gradle.kts`: `releaseAbis = [arm64-v8a, armeabi-v7a, x86, x86_64]`.
- `.github/workflows/android-build.yml`: أضفت targets الـ 32-بت
  (`armv7-linux-androideabi`, `i686-linux-android`) ووحّدت `ANDROID_NDK_HOME`
  و `ANDROID_NDK_ROOT` على إصدار واحد.
- `src/app_model.rs`: ضمّنت المقاطع بـ `include_bytes!` وكتبتُها إلى مسار قابل
  للكتابة (الأصل داخل APK لا يُقرأ بـ `std::fs`).

## 5. التحذير المتبقي `[HYPOTHESIS]`
بعض التبعيات (مثل `aws-lc-sys` من rustls/reqwest) قد لا تبني على 32-بت.
إن فشل بناء ABIs الجديدة، الحل هو `releaseAbiFeatures` لاستبعاد ميزة الشبكة
عن 32-بت، أو الإبقاء على arm64 فقط مع توثيق الحد الأدنى للجهاز. **لم يُبنَ APK
بعد الإصلاح** (حسب طلب المستخدم)، فالنتيجة النهائية غير مؤكدة.

## 6. المصادر
- `developer.android.com/reference/android/content/res/AssetManager` — أصول APK
  تُقرأ عبر AssetManager لا `std::fs`.
- `developer.android.com/guide/topics/manifest/activity-element` — `exported`.
- كود Cranpose 0.24.0: `android_entry.rs`, `CranposeActivity.java`,
  `CranposeProcess.java`, `android/cranpose-gradle-plugin/**/*.kt`.
- الـ APK: `dist/github-android/counter-signed-apk/app-release.apk`.
