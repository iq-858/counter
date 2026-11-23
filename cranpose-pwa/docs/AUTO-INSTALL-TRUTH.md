# التثبيت التلقائي لـ PWA — الحقيقة من 3 مصادر رسمية

> مصادر: MDN `Making PWAs installable` (2026-10-06) + web.dev `Installation` و
> `Installation prompt` + Apple/WebKit (web.dev iOS section). كلها `[SOURCE-VERIFIED]`.

## السؤال: هل يمكن تثبيت PWA تلقائياً بموافقة المستخدم ثم يكتمل دون تدخله؟

**الإجابة القصيرة: لا — لا يوجد «تثبيت صامت» في أي متصفح mainstream. موافقة المستخدم إلزامية، وعلى iOS الخطوات يدوية بالكامل.**

## ما تقوله المصادر حرفياً

### 1) MDN
> «The user will be offered the chance to install the app. This means you can distribute your PWA as a website... When the user selects the icon, the browser displays a prompt asking if they want to install the PWA, and **if they accept**, the PWA is installed.»

> «This technique relies on the `beforeinstallprompt` event... in the event handler for its own 'Install' button, call `prompt()`. **This is not supported on iOS.**»

### 2) web.dev — Installation
> «**A browser prompt to install your PWA doesn't exist On iOS and iPadOS.** These apps have to be added manually to the home screen using a browser share menu.»

خطوات iOS الرسمية (لا يمكن تجاوزها):
1. افتح Share menu
2. Add to Home Screen
3. Confirm الاسم
4. Add

### 3) web.dev — Installation prompt
> «You can only call `prompt()` on the deferred event **once**. If the user dismisses it, you need to wait until `beforeinstallprompt` is fired again.»

> «**The event will not fire if: the user has already installed the current PWA** (desktop/WebAPK Android).»

> «Chrome and Edge on iOS do not support PWA installation... the only option is to open the PWA using Safari.»

## ماذا يعني هذا عملياً؟

| المنصة | أقصى أتمتة ممكنة | موافقة المستخدم |
|---|---|---|
| Android (Chrome/Edge, WebAPK) | نلتقط `beforeinstallprompt` ونعرض زرنا في اللحظة المناسبة، ثم `prompt()` | ✅ نقرة واحدة على زرنا + تأكيد المتصفح |
| iOS (Safari/Chrome) | لا يوجد prompt أصلاً — نعرض تعليمات Share فقط | ✅ 4 خطوات يدوية في Share menu |
| Desktop Chromium | زرنا + `prompt()` | ✅ نقرة + تأكيد |

**لا توجد واجهة برمجية (API) تتخطى موافقة المستخدم** — هذا قرار أمني متعمّد لمنع المواقع من تثبيت نفسها دون علم المستخدم. أي مكتبة تدّعي `silent_install()` إما تكذب أو تتطلب kiosk browser.

## لماذا ظهرت رسالة التثبيت بعد أن ثبّتّها على iOS؟

من web.dev `[SOURCE-VERIFIED]`:
> «On Apple devices, you can install the same PWA multiple times. **Each installation has its own isolated storage**... browser and Web.app run in different processes and may have **isolated storage**.»

أي أن:
- `localStorage` في Safari ≠ `localStorage` في التطبيق المثبّت.
- حفظ «لا تظهر مجدداً» في Safari **لا ينتقل** للتطبيق المثبّت.

### الحل القاطع (طبّقناه في الموقع)
web.dev نفسه يعطي الحل:
> «You should only render these instructions in browser mode; other display options such as `standalone` mean the user has already installed. To render the element only in browser mode, use the `display-mode` media query.»

```css
@media (display-mode: standalone), (display-mode: fullscreen) {
  [data-pwa-install-ui] { display: none !important; }
}
```
هذا **CSS خالص** — يعمل حتى لو كان localStorage معزولاً أو JS لم يحمّل بعد. أضفناه في `assets::INSTALL_UI_CSS` وفي موقع الاختبار. النتيجة: بمجرد فتح التطبيق المثبّت، تختفي رسالة التثبيت نهائياً بغض النظر عن أي حالة JS/localStorage.

## ما أضفناه للمكتبة لهذا السبب
1. `persist.rs` — حفظ الرفض/الطلب في localStorage (يمنع التكرار داخل نفس السياق).
2. `device.rs` — كشف `standalone` عبر `display-mode` + `navigator.standalone`.
3. `assets::INSTALL_UI_CSS` — الحل القاطع المعزول عن JS.
4. `appinstalled` event (Chromium) — يمكن إضافته تنبيهاً للـ observability لا للتثبيت.

## أقرب حل رسمي لتقليل خطوات iOS: Web Share API

MDN `[SOURCE-VERIFIED]`: `navigator.share()` «invokes the native sharing mechanism of the device»، ويُستدعى من user gesture (نقرة زر). لوحة المشاركة على iOS تحتوي «Add to Home Screen».

أضفنا `share_for_install()` في `install.rs` — زر واحد يفتح لوحة المشاركة مباشرة، فيختار المستخدم «إضافة إلى الشاشة الرئيسية». هذا **يقلل الخطوات** (لا يحتاج المستخدم البحث عن زر المشاركة) لكنه **لا يُثبّت صامتاً** — قاعدة Apple.

`getInstalledRelatedApps()` (MDN) يكشف التثبيت لكنه **تجريبي** وغير مدعوم في Safari — لا يساعد على iOS.

**لماذا «نعم — مثبّت ✅» لا تظهر عندك؟** لأنها تظهر فقط عند فتح التطبيق المثبّت **من الشاشة الرئيسية** (وضع standalone). فتح نفس الرابط في Safari يبقى `display-mode: browser` → «لا — متصفح عادي». تأكد من فتح أيقونة التطبيق، لا الرابط.

## الخلاصة
- **التثبيت التلقائي 100% مستحيل** بقرار المتصفحات (3 مصادر تؤكد).
- أقصى ما يمكن: تقليل الاحتكاك إلى نقرة واحدة على Android/Desktop، وتعليمات واضحة على iOS.
- منع تكرار الرسالة بعد التثبيت: **CSS `@media (display-mode: standalone)`** هو الموثوق الوحيد على iOS بسبب الـ isolated storage.
