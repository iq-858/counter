# الحل التلقائي لكشف خلل WebGPU على iOS وفرض WebGL2

> الهدف: **الكود يكتشف ويحل المشكلة أوتوماتيكياً** — المستخدم لا يضيف `?backend=gl`.
> المصادر من المستودع الرسمي `samoylenkodmitry/Cranpose` (main).

## المشكلة في سطر واحد
Cranpose يختار WebGPU تلقائياً (`WebBackendPreference::Auto`) على أي متصفح يُعلن `'gpu' in navigator` — وهذا يشمل **Safari وChrome على iOS** (كلاهما WebKit). WebGPU على iOS ينتج ألواناً خاطئة. الحل: فرض `Gl` على iOS.

## استراتيجيتان (اختر واحدة)

### Strategy A — كشف WebKit/iOS من userAgent في index.html (الأبسط، موصى به)

الـ shell (صفحة index.html) تتحكم بالرابط قبل أن يقرأه WASM. أضف هذا **قبل** سطر:
```js
const backend = params.get('backend') || 'auto';
```

```js
// --- Auto-detect WebKit/iOS and force the WebGL2 backend -------------------
// Chromium and Safari on iOS both use WebKit; their WebGPU path renders text
// with wrong colours in Cranpose. Detect the iOS WebKit UA and inject
// ?backend=gl without asking the user to type anything.
const ua = navigator.userAgent;
const isIOSWebKit =
    /iPad|iPhone|iPod/.test(ua) ||
    (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1); // iPadOS desktop-class UA

if (isIOSWebKit) {
    const url = new URL(window.location.href);
    if (url.searchParams.get('backend') === null ||
        url.searchParams.get('backend') === 'auto') {
        url.searchParams.set('backend', 'gl');
        // replaceState keeps history clean and does not reload the page.
        window.history.replaceState({}, '', url);
    }
}
// -------------------------------------------------------------------------
```

ثم يقرأ `params.get('backend')` القيمة `gl` تلقائياً → `requested_web_backend()` في
`backend_preference.rs` يُرجع `WebBackendPreference::Gl` → `instance_backends()` تُرجع
`wgpu::Backends::GL` فقط. لا WebGPU إطلاقاً على iOS.

**المزايا:** لا تعديل على Rust/wgpu، يعمل مع أي بناء جاهز، يُطبَّق قبل تحميل WASM.
**العيوب:** يعتمد على userAgent (can be spoofed / iPadOS needs the touch trick above).

### Strategy B — تعديل `backend_preference.rs` (الأقوى، يتطلب بناءً جديداً)

غيّر الدالة في `crates/cranpose/src/web/backend_preference.rs` لقراءة تلميح من
الـ shell (مثل `window.__cranposeForceGl`) أو للكشف المباشر من `navigator`:

```rust
pub(super) fn requested_web_backend(window: &web_sys::Window) -> WebBackendPreference {
    // 1) An explicit URL override still wins (highest priority).
    let query = window.location().search().unwrap_or_default();
    for pair in query.trim_start_matches('?').split('&') {
        let Some((key, value)) = pair.split_once('=') else { continue };
        if key == "backend" {
            return match value {
                "webgpu" => WebBackendPreference::WebGpu,
                "gl"     => WebBackendPreference::Gl,
                _        => WebBackendPreference::Auto,
            };
        }
    }

    // 2) Auto: on WebKit/iOS force GL because WebGPU renders text wrongly there.
    let ua = window.navigator().user_agent().unwrap_or_default();
    let is_ios_webkit = ua.contains("iPhone") || ua.contains("iPad") || ua.contains("iPod");
    if is_ios_webkit {
        return WebBackendPreference::Gl;
    }

    WebBackendPreference::Auto
}
```

**المزايا:** لا يعتمد على تعديل الرابط، يعمل حتى لو حُذفت الإضافات من HTML.
**العيوب:** يحتاج rebuilding WASM (أنت تملك المستودع أو تستطيع fork).

## Strategy C — كشف الخلل بدل المنصة (الأدق تقنياً، الأصعب)

بدل افتراض أن كل iOS مُخِل، اختبر فعلياً:
1. ارسم نصاً أبيض على خلفية داكنة بكلا Backend إلى canvas مخفي.
2. اقرأ البكسل بـ `readPixels`/`getImageData`.
3. إن كان النص داكناً (خلل) → أعد التهيئة بـ `Gl`.

هذا هو الحل «الذكي» لكنه يتطلب إعادة تهيئة wgpu device في وقت التشغيل — معقد ولا تدعمه الواجهة الحالية بسهولة. **أنصح بالـ Strategy A أو B.**

## كيف تتحقق أن الحل اشتغل (أوتوماتيكياً)

من Console في Safari iOS ابحث عن سطر Cranpose الرسمي:
```
Web backend preference=Gl, selected backend=Gl, max_texture_dimension_2d=...
```
- `preference=Gl` → الكشف حقن GL ✓
- `selected backend=Gl` → wgpu استخدم WebGL2 فعلاً ✓ (لا BrowserWebGpu)
- إن رأيت `selected backend=BrowserWebGpu` → الكشف فشل، راجع userAgent.

## ملفات مرجعية من المستودع الرسمي
| الملف | الدور |
|---|---|
| `crates/cranpose/src/web/backend_preference.rs` | `requested_web_backend`, `instance_backends`, `auto_instance` |
| `crates/cranpose/src/web/event_loop.rs` | يقرأ التفضيل، يختار adapter، يطبع سطر الـ log |
| `apps/desktop-demo/index.html` | الـ shell الذي يقرأ `backend` قبل تحميل WASM |
| `apps/desktop-demo/WEB_REQUIREMENTS.md` | توثيق `?backend=gl` و`?backend=webgpu` |
| `crates/cranpose-render/common/src/text_mask_gamma.rs` | سبب خلل ألوان النص تحت WebGPU iOS |

## توصية نهائية
**استخدم Strategy A** الآن (لا rebuild، يعمل فوراً مع أي موقع Cranpose): الصق مقطع الكود أعلى `index.html` قبل قراءة `backend`. المستخدم يفتح الرابط العادي في Safari/Chrome على iOS فيُكتشف النظام ويُحقن `gl` تلقائياً — دون أن يضيف هو أي شيء.
