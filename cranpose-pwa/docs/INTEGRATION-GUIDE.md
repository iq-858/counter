# دمج cranpose-pwa في مشاريع الويب — شرح تفصيلي صادق

> المصادر: كود المكتبة + MDN `Making PWAs installable` (2026-10-06) +
> مستودع Cranpose الرسمي. الوسوم `[SOURCE-VERIFIED]`.

## 1) ماذا تحقق المكتبة فعلاً؟

| الوحدة | ما تفعله | متى |
|---|---|---|
| `device` | كشف iPhone/iPad/Android/Desktop + WebGPU/WebGL2/SW/standalone | وقت التشغيل (WASM) |
| `backend` | حقن `backend=gl` تلقائياً على iOS WebKit | وقت التشغيل، قبل قراءة الرابط |
| `install` | التقاط `beforeinstallprompt`، تتبع قابلية التثبيت، `prompt()`، تعليمات iOS | وقت التشغيل |
| `service_worker` | تسجيل `sw.js` | وقت التشغيل |
| `cache` | 3 استراتيجيات caching + توليد محتوى `sw.js` | وقت البناء (build.rs) |
| `manifest` | بناء `manifest.webmanifest` صالح | وقت البناء |
| `preload` | وسوم `<link rel=preload/modulepreload>` | وقت البناء |
| `build::emit` | **يكتب الملفات الثلاثة تلقائياً من build.rs** | وقت البناء |

## 2) كيف تكشف عن الملفات عند البناء؟ (أسماء ثابتة أم لا؟)

**الأسماء قابلة للتخصيص بالكامل** عبر `PwaBuildConfig`:

```rust
// build.rs في مشروعك (سطران فقط)
fn main() {
    cranpose_pwa::emit(&cranpose_pwa::PwaBuildConfig {
        name: "My App".into(),
        short_name: "App".into(),
        out_dir: "wwwroot".into(),          // ← أنت تختار المجلد
        wasm_url: "/pkg/my_app_bg.wasm".into(),  // ← اسم WASM الفعلي لديك
        js_url: "/pkg/my_app.js".into(),
        ..Default::default()
    }).unwrap();
}
```

هذا يولّد في `wwwroot/`:
- `manifest.webmanifest` (باسمك وأيقونات 192/512)
- `sw.js` (يخبّئ WASM/JS بالاسم الذي أعطيته + 3 استراتيجيات)
- `head-snippet.html` (وسوم preload + iOS meta)

**إذن: أنت لا تكتب ملفات JS أو manifest يدوياً** — المولّد يقرأ أسماء ملفاتك من `PwaBuildConfig` ويولّدها. ما عليك سوى:
1. سطر `build.rs` أعلاه.
2. نسخ محتوى `head-snippet.html` داخل `<head>` في صفحتك (أو حقنه عبر قالبك).
3. التأكد أن الأيقونات `icon-192.png`/`icon-512.png` موجودة (عدّل أسماءها في `manifest.rs` إن اختلفت).

## 3) هل المكتبة «مستقلة بالكامل» بحيث يكفي تفعيلها من `.rs` بلا أي ملفات أخرى؟

**الحقيقة الصادقة: لا، وليس بسبب المكتبة — بل بسبب قواعد المتصفح `[SOURCE-VERIFIED MDN]`:**

> «The manifest is included using a `<link rel="manifest" href="manifest.json" />` element in the app's HTML. If the PWA has more than one page, every page must reference the manifest.»
> «For a PWA to be installable it must be served using https or localhost.»

أي أن المتصفح **يجب** أن يجد:
1. ملف manifest **حقيقي** على HTTP(S) قبل أن يقرر أن الموقع قابل للتثبيت.
2. ملف `sw.js` **حقيقي** مسجّل من الصفحة.
3. `<link rel="manifest">` في HTML.

**Rust/WASM لا يستطيع حقن هذه الملفات بعد تحميل الصفحة** — المتصفح اتخذ قرار_installability قبل أن يبدأ WASM. لهذا لا يمكن أن تكون «`.rs` فقط صفر ملفات».

ما تستطيع المكتبة فعله (وفعلته): **توليد كل هذه الملفات تلقائياً من `build.rs`**، فيبقى على المطور سطران + سطر `<link>`، لا كتابة يدوية لأي JS/HTML/JSON.

## 4) هل يمكن التثبيت تلقائياً بالكامل بلا زر موافقة من المستخدم؟

**لا — وهذا قرار أمني للمتصفح، ليس قرار مكتبتنا `[SOURCE-VERIFIED MDN]`:**

> «The user will be offered the chance to install the app... if they accept, the PWA is installed.»
> `beforeinstallprompt.prompt()` يجب أن يُستدعى من **user gesture** (نقرة).
> **«This is not supported on iOS»** — أي أن prompt المخصص لا يعمل على iOS إطلاقاً؛ التثبيت يتم يدوياً من Share menu.

| المنصة | هل يمكن تثبيت دون موافقة المستخدم؟ |
|---|---|
| Android (Chrome) | ❌ لا — `prompt()` يحتاج نقرة؛ المتصفح يعرض تأكيده |
| iOS (Safari/Chrome) | ❌ لا — Share menu يدوي فقط |
| Desktop Chrome | ❌ لا — أيقونة URL bar + تأكيد |

أي مكتبة تدّعي «تثبيت صامت بدون موافقة» إما تكذب أو تستخدم متصفحاً مملوكاً (kiosk). **المكتبة تفتح أقرب نقطة ممكنة**: تلتقط الـ prompt، تخفي الـ mini-infobar، وتعرض زر «تثبيت» في اللحظة المناسبة — لكن النقرة الأخيرة للمستخدم.

## 5) الخلاصة العملية — أقل 작업 على المطور

```rust
// 1) build.rs — سطران، يولّد manifest + sw + head snippet
fn main() { cranpose_pwa::emit(&cranpose_pwa::PwaBuildConfig::default()).unwrap(); }
```
```html
<!-- 2) في <head> — انسخ محتوى head-snippet.html المولّد (سطر واحد) -->
<link rel="manifest" href="/manifest.webmanifest">
```
```rust
// 3) في main.rs — تفعيل المكتبة
let mut pwa = Pwa::detect();       // كشف + backend=gl تلقائي على iOS
pwa.listen_for_install();
pwa.capture_prompt();
let _ = pwa.register_service_worker();
match pwa.advice() { ShowPrompt => ..., ShowManualInstructions => ..., Hide => ... }
```

**النتيجة**: صفر كتابة يدوية لـ JS/JSON/HTML (الملفات مولّدة)، أقصى أتمتة يسمح بها المتصفح، وموافقة التثبيت تظل بيد المستخدم (قاعدة أمنية لا يمكن تجاوزها).
