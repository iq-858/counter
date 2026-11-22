# نتائج البناء على GitHub — 2026-11-21/22

> كل البناء جرى على **GitHub Actions فقط** — لا بناء محلي (حسب طلب المستخدم).
> المستودع: `https://github.com/iq-858/counter`، الفرع `master`.
> أُعيد البناء بعد اكتمال كل الإصلاحات (آخر التزامات: `af8180a` إصلاح تجمد Live،
> `bd41449` أذونات Android).

## 1. iOS (IPA) — ✅ نجح (النسخة النهائية)

- الالتزام المبني: `bd41449` (يشمل إصلاح تجمد Live).
- الـ run: https://github.com/iq-858/counter/actions/runs/45008497136
- الأرشيف: `cranpose-ios-debug`
- الملف: `dist/github-ios-final/cranpose-ios-debug/application.ipa`
- المعرّف: `com.example.counter`
- **يحتوي إصلاح تجمد Live**: `open_url` لم يعد يحجب خيط التركيب، وفحص فشل الـ item انتقل إلى خيط الفك.

> ملاحظة توقيع: `artifact.json` يقول `"needs_device_signing": true` — IPA من نوع
> debug يحتاج توقيع جهاز (provisioning profile) للتثبيت على iPhone. هذا ليس خللاً
> في الكود بل متطلب Apple.

## 2. Android (APK موقّع) — ✅ نجح

- الـ run: https://github.com/iq-858/counter/actions/runs/45008467895
- APK: `dist/github-android/counter-signed-apk/app-release.apk` (65,468,913 بايت)
- SHA256: `2ECAC2BD6331381519082D1356A324E9DB561095E8DA06FDCD2DBCE6D21BD970`
- keystore: `dist/github-android/counter-release-keystore/counter-release.jks` (2,730 بايت)
- التوقيع: تم التحقق منه في الـ workflow عبر `apksigner verify --print-certs`.
- المعرّف: `com.example.counter`، إصدار `1.0` (versionCode 1).
- ABIs: `arm64-v8a` + `x86_64`.

### سجل المحاولات السابقة (كلها أُصلحت)
1. `44897497636` — `setup-android@v3` فشل على حزمة `tools` → استخدام sdkmanager المثبّت.
2. `44897747904` — `./gradlew: Permission denied` → chmod +x واستدعاء `bash`.
3. `44898100217` — `AppLauncher::try_run` غير موجود على Android → استبعاد `run()` عبر cfg.
4. `44898804928` — manifest check: أذونات `INTERNET`/`ACCESS_NETWORK_STATE` ناقصة → أُضيفت.

## 3. ملاحظات
- الـ IPA الحالي **يسبق** إصلاح تجمد Live؟ لا — أُعيد بناؤه بعد الإصلاح (`45008497136`).
- الـ APK موقّع بمفتاح مُولَّد حديثاً على الـ runner. إن أردت هوية توقيع ثابتة عبر
  البناءات، يمكن حفظ الـ keystore كـ GitHub Secret وإعادة استخدامه.
- مراجعة الادعاءات: `cranpose-video/docs/REVIEW-RECHECK-2026-11-21.md`.
