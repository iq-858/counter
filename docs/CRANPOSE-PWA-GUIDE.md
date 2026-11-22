# دعم PWA في مواقع Cranpose — التشخيص والتنفيذ

> مصادر من المستودع الرسمي `samoylenkodmitry/Cranpose` (main):
> `apps/desktop-demo/package-web.sh`, `WEB_REQUIREMENTS.md`, `index.html`.

## 1. هل يدعم Cranpose الـ PWA؟ — الإجابة: لا، ليس مدمجاً `[SOURCE-VERIFIED]`

`package-web.sh` يغلّف هذه الملفات فقط:
```
index.html
favicon.svg
apple-touch-icon.png        ← أيقونة iOS (لكنها ليست PWA)
asset-manifest.json         ← قائمة WASM/JS، ليست Web App Manifest
.nojekyll
<bundle>/desktop_app.js
<bundle>/desktop_app_bg.wasm
<bundle>/snippets/          ← اختياري
<bundle>/first_screen_glsl.txt ← اختياري
```

**لا يوجد:**
- ❌ `manifest.webmanifest` (Web App Manifest القياسي)
- ❌ `sw.js` / service worker (للـ offline والتثبيت)
- ❌ تسجيل service worker في `index.html`

`apple-touch-icon.png` يسمح بـ «Add to Home Screen» كـ **رابط مختصر ( bookmark )** فقط — لا يعمل كتطبيق PWAInstallable بكامل خصائصه (شاشة إقلاع، offline، standalone).

السبب منطقي: حزمة Cranpose تُحمّل WASM كبيراً (عدة ميغابايت) عبر `fetch` — والـ offline يحتاج استراتيجية caching دقيقة لهذا الـ WASM تحديداً.

## 2. التقنيات المطلوبة لتحويل موقع Cranpose إلى PWA

### أ) Web App Manifest (`manifest.webmanifest`) — إلزامي للتثبيت على Android
```json
{
  "name": "Cranpose Demo",
  "short_name": "Cranpose",
  "start_url": "/",
  "display": "standalone",
  "background_color": "#1c1c1e",
  "theme_color": "#5b50e8",
  "orientation": "any",
  "icons": [
    { "src": "icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any maskable" },
    { "src": "icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any maskable" }
  ]
}
```
اربطه في `<head>`:
```html
<link rel="manifest" href="/manifest.webmanifest">
<meta name="theme-color" content="#5b50e8">
```

### ب) Service Worker (`sw.js`) — إلزامي للـ offline والتثبيت الحقيقي
استراتيجية مناسبة لـ WASM كبير: **cache-first مع نسخة مُدارة**:
```js
const CACHE = 'cranpose-v1';
const CORE = [
  '/', '/index.html', '/asset-manifest.json',
  // أضف مسارات الـ bundle بعد قراءتها من asset-manifest.json
];
self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(CORE)).then(() => self.skipWaiting()));
});
self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys().then((keys) =>
    Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))
  ).then(() => self.clients.claim()));
});
self.addEventListener('fetch', (e) => {
  e.respondWith(
    caches.match(e.request).then((hit) => hit || fetch(e.request).then((res) => {
      if (res.ok && new URL(e.request).origin === location.origin) {
        const copy = res.clone();
        caches.open(CACHE).then((c) => c.put(e.request, copy));
      }
      return res;
    }))
  );
});
```
سجّله في `index.html` (فقط على https أو localhost):
```html
<script>
if ('serviceWorker' in navigator) {
  window.addEventListener('load', () =>
    navigator.serviceWorker.register('/sw.js').catch(console.error));
}
</script>
```

### ج) أيقونات بأحجام PWA
- `192x192` و`512x512` (Android) — `purpose: any maskable`.
- `apple-touch-icon.png` 180x180 موجود أصلاً (iOS).
- iOS لا يستخدم الـ manifest للتثبيت؛ يستخدم `<meta name="apple-mobile-web-app-capable" content="yes">`:
```html
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
<meta name="apple-mobile-web-app-title" content="Cranpose">
```
⚠️ **iOS PWA لا يدعم WebGPU/WebGL2 موثوقاً في الوضع standalone** — وهذا يضاعف مشكلة الألوان التي اكتشفتها. استخدم Strategy A من `CRANPOSE-WEB-AUTO-GL-PATCH.md` (كشف iOS → `backend=gl`) قبل أي شيء.

### د) HTTPS — إلزامي
service worker لا يعمل إلا على HTTPS (أو localhost). GitHub Pages يوفر HTTPS تلقائياً ✓.

## 3. الدعم حسب النظام `[SOURCE-VERIFIED من معايير PWA + تجربتك]`

| النظام | تثبيت PWA | offline | ملاحظة Cranpose |
|---|---|---|---|
| **Android (Chrome)** | ✅ عبر manifest + SW | ✅ | WebGPU سليم → يعمل مباشرة |
| **iOS (Safari)** | ✅ «Add to Home Screen» | ✅ محدود | يجب `backend=gl` + apple meta |
| **iOS (Chrome)** | ⚠️ يفتح Safari للإضافة | محدود | نفس قيود WebKit |
| **Desktop (Chrome/Edge)** | ✅ تثبيت كتطبيق | ✅ | WebGPU سليم |
| **Safari Desktop** | ⚠️ محدود | محدود | — |

## 4. خطة تنفيذ عملية على موقعك (خطوات)

1. انسخ مجلد الموقع الناتج من `package-web.sh` إلى مجلد نشر جديد.
2. أضف `manifest.webmanifest` + الأيقونات (192/512).
3. أضف `sw.js` بالكود أعلاه (حدّث قائمة CORE لتشمل أسماء ملفات الـ bundle الفعلية من `asset-manifest.json`).
4. أضف الـ `<link>` والـ `<meta>` في `index.html` + registrator الـ SW.
5. **طبّق `CRANPOSE-WEB-AUTO-GL-PATCH.md` (Strategy A) أولاً** — وإلا iOS PWA سيعرض ألواناً خاطئة.
6. انشر على GitHub Pages (HTTPS جاهز) مع `.nojekyll` (موجود أصلاً).
7. تحقق: Chrome DevTools → Application → Manifest + Service Workers؛ وعلى iOS: Add to Home Screen ثم افتح standalone.

## 5. نصيحة حاسمة
أكبر عقبة تقنية ليست الـ PWA نفسها بل **حجم الـ WASM**: تأكد أن الـ service worker يخبّئ `desktop_app_bg.wasm` صراحةً، وإلا فتح التطبيق بدون شبكة سيفشل في `fetch` الـ WASM. استخدم `asset-manifest.json` (الذي يولّده Cranpose) لمعرفة اسم الملف بالضبط — لا تفترضه.

## الخلاصة
- Cranpose **لا يدعم PWA مدمجاً** — عليك إضافتها في طبقة الـ shell (HTML/SW)، وهي إضافة **خارج نطاق مكتبتنا** `cranpose-video`.
- التقنيات: `Web App Manifest` + `Service Worker` + `Icons 192/512` + `HTTPS` + (لـ iOS) `apple-mobile-web-app-capable` + (لـ iOS Cranpose) `backend=gl` التلقائي.
