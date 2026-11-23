# حلول التثبيت المتقدمة — ما هو ممكن فعلاً على iOS

> 5 مصادر رسمية: MDN `Trigger install prompt`, MDN `Making PWAs installable`,
> MDN `Navigator.share`, web.dev `Installation`, web.dev `Installation prompt`.
> كلها `[SOURCE-VERIFIED]` (أُعيدت قراءتها 2026-11-23).

## الخلاصة الحاسمة
> MDN: «The technique ... depends on `beforeinstallprompt`, which is **non-standard and currently only implemented in Chromium-based browsers**.»
> web.dev: «**A browser prompt to install your PWA doesn't exist on iOS/iPadOS** ... added manually using a browser share menu.»

**لا يوجد API ويب يفتح «إضافة إلى الشاشة الرئيسية» مباشرة على iOS.** التثبيت بصفر نقرة مستحيل. الإجابة «الأفضل» تعني تقليل الاحتكاك بأقصى حد يسمح به النظام.

## الحل 1 (جذري، زر حقيقي على iOS): Smart App Banner
إذا كان لديك تطبيق في App Store، أضف في `<head>`:
```html
<meta name="apple-itunes-app" content="app-id=YOUR_APP_ID">
```
MDN/web.dev `[SOURCE-VERIFIED]`: Safari يعرض **شريط تثبيت أصلي** أعلى الصفحة بزر «Get/Open» — ضغطة واحدة. هذا أقرب شيء لزر «إضافة مباشرة».

## الحل 2 (جذري): غلّف الـ PWA كتطبيق iOS أصلي
- **PWABuilder** (Microsoft، مجاني) يولّد مشروع Xcode بـ WKWebView + App-Bound Domains.
- **Capacitor / Cordova** بديل.
- ثم انشر في App Store → المستخدم يضغط «Get» → تثبيت بضغطة واحدة كأي تطبيق.
- التكلفة: حساب Apple Developer ($99/سنة) + مراجعة.

## الحل 3 (Android، مجاني): WebAPK عبر TWA
- **Bubblewrap CLI** (Google) يغلّف الـ PWA في TWA → WebAPK يُثبّت صامتاً بعد موافقة المتصفح.
- web.dev `[SOURCE-VERIFIED]`: «the browser installs that app silently on the user's device... no need for sideloading.»

## الحل 4 (ويب فقط، الأفضل المتاح): in-app install sheet
بدل نص عادي، اعرض **شريط تثبيت بنمط متجر التطبيقات** + زر يفتح لوحة المشاركة (Web Share API) + حركة توضيحية تلفت الانتباه إلى خيار «إضافة إلى الشاشة الرئيسية». هذا ما طبّقناه في الموقع الحي (`cranpose-pwa/demo/`).

## مقارنة القرار
| هدفك | الحل |
|---|---|
| زر تثبيت واحد على iPhone **دون متجر** | ❌ مستحيل — قاعدة Apple |
| زر تثبيت واحد على iPhone **عبر متجر** | Smart App Banner أو تطبيق أصلي (الحل 1/2) |
| تثبيت صامت على Android | WebAPK/TWA (الحل 3) |
| تقليل الخطوات بموقع ويب فقط | install sheet + Web Share (الحل 4) |

## ما أضفناه للمكتبة
- `share_for_install()` — Web Share API (أقرب حل ويب رسمي)
- `beforeinstallprompt` + `appinstalled` (Chromium)
- `INSTALL_UI_CSS` — إخفاء واجهة التثبيت في standalone
- `persist` — منع التكرار
- `assets::IOS_META` — apple-touch-icon + apple-mobile-web-app-capable

## الخاتمة الصادقة
إن أردتَ «زر إضافة إلى الشاشة الرئيسية» بضغطة واحدة على iPhone، **يجب** أن تمر عبر App Store (الحل 1/2). إن بقيتَ ويباً خالصاً، فالحل 4 هو الأقصى التقني، وبعده يبقى الاختيار الأخير بيد المستخدم في لوحة المشاركة.
