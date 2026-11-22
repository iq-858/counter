# دليل الويب في Cranpose: اختيار WebGL2 وكشف الرسم

> مصادر كلها من المستودع الرسمي `samoylenkodmitry/Cranpose` (فرع main)،
> ليس النسخة المحلية. الوسوم `[SOURCE-VERIFIED]` = مقروء من المصدر.

## 1. كيف يكتب موقع Cranpose (البنية)

من `apps/desktop-demo/WEB_REQUIREMENTS.md` و`crates/cranpose-platform/web/README.md`:

```
apps/desktop-demo/
├── index.html       ← الـ shell: canvas + تحميل الـ WASM
├── pkg/             ← ناتج wasm-pack (.wasm + .js)
├── src/             ← كود التطبيق (Rust)
└── Cargo.toml
```

**خطوات البناء الرسمية:**
```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
just web                      # يبني pkg/ تحت apps/desktop-demo
cd apps/desktop-demo
python3 -m http.server 8080   # قدّم المجلد
# افتح http://localhost:8080
```

`package-web.sh <output-dir>` يغلّف WASM + index.html في موقع static content-addressed.

## 2. كيف يكشف Cranpose الـ backend (الكود الرسمي)

### أ) قراءة التفضيل من الرابط — `crates/cranpose/src/web/backend_preference.rs`
```rust
pub(super) fn requested_web_backend(window: &web_sys::Window) -> WebBackendPreference {
    let query = window.location().search().unwrap_or_default();
    for pair in query.trim_start_matches('?').split('&') {
        let Some((key, value)) = pair.split_once('=') else { continue };
        if key != "backend" { continue }
        return match value {
            "webgpu" => WebBackendPreference::WebGpu,
            "gl"     => WebBackendPreference::Gl,
            _        => WebBackendPreference::Auto,
        };
    }
    WebBackendPreference::Auto
}
```

### ب) تحويل التفضيل إلى backends في wgpu — نفس الملف
```rust
pub(super) fn instance_backends(preference: WebBackendPreference) -> wgpu::Backends {
    match preference {
        WebBackendPreference::Auto   => wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
        WebBackendPreference::WebGpu => wgpu::Backends::BROWSER_WEBGPU,
        WebBackendPreference::Gl     => wgpu::Backends::GL,   // ← WebGL2 فقط
    }
}
```

### ج) التطبيق في event_loop — `crates/cranpose/src/web/event_loop.rs`
```rust
let backend_preference = requested_web_backend(&window);
instance_desc.backends = instance_backends(backend_preference);
let instance = match backend_preference {
    WebBackendPreference::Auto => auto_instance(instance_desc).await,
    WebBackendPreference::WebGpu | WebBackendPreference::Gl => { /* instance على النمط */ }
};
let (adapter, device, queue) = ...request_adapter...
let adapter_info = adapter.get_info();
log::info!(
    "Web backend preference={:?}, selected backend={:?}, max_texture_dimension_2d={}",
    backend_preference, adapter_info.backend, adapter.limits().max_texture_dimension_2d
);
```

**الثلاث قيم (من WEB_REQUIREMENTS.md):**
| الرابط | النتيجة |
|---|---|
| بدون معامل | WebGPU حيث يتوفر، وإلا WebGL2 |
| `?backend=gl` | WebGL2 إجباري |
| `?backend=webgpu` | WebGPU إجباري |

## 3. كيف تتحقق أن الرسم سيعمل (قبل الالتزام)

### أ) من JavaScript في الـ shell (صفحة index.html الرسمية)
```js
const backend = params.get('backend') || 'auto';
const hasWebGpu = 'gpu' in navigator;
const hasWebGl2 = !!document.createElement('canvas').getContext('webgl2');
if (backend === 'webgpu' && !hasWebGpu) throw new Error('WebGPU unavailable');
if (backend === 'gl' && !hasWebGl2) throw new Error('WebGL2 unavailable');
if (!hasWebGpu && !hasWebGl2) throw new Error('No WebGPU or WebGL2');
```

### ب) من سجل Cranpose نفسه (أصدق مؤشر)
عند الإقلاع يطبع Cranpose:
```
Web backend preference=Gl, selected backend=Gl, max_texture_dimension_2d=...
```
- `preference=Gl` يعني أن الرابط فَعّل WebGL2.
- `selected backend=Gl` يعني أن wgpu فعلاً استخدم WebGL2 (لا WebGPU).
- إن رأيت `selected backend=BrowserWebGpu` فأنت على المسار الذي يسبب مشكلة الألوان على iOS.

### ج) تحقق من الصورة نفسها (canvas)
```js
const canvas = document.getElementById('cranpose-canvas');
const gl = canvas.getContext('webgl2');       // ليس null → WebGL2 متاح
const err = gl && gl.getError();              // 0 → لا خطأ GPU
```

### د) مقارنة بصرية (الطريقة العملية التي استخدمتها أنت)
```
افتح الرابط بدون معامل → قارن الصورة
افتح ?backend=gl          → قارن الصورة
```
إن اختلفت الصورتان → WebGPU هو الخلل؛ وإن تطابقتا → لا مشكلة.

## 4. تثبيت WebGL2 كخيار افتراضي في موقعك

### الطريقة 1 (لا كود): أضف `?backend=gl` في الرابط — ما جرّبته وworks.

### الطريقة 2 (تعديل كود الموقع): في `backend_preference.rs` غيّر الافتراضي:
```rust
// بدل WebBackendPreference::Auto في نهاية requested_web_backend:
WebBackendPreference::Gl     // يصبح دائماً WebGL2 ما لم يطلب الرابط غيره
```
أو غيّر `instance_backends(Auto)` إلى `wgpu::Backends::GL`.

### الطريقة 3 (بناء بـ feature): فعّل `cranpose_first_screen_glsl`
```rust
#[cfg(cranpose_first_screen_glsl)]
{ super::first_screen_glsl::auto_instance(descriptor).await }
```
هذا يختار GL من أول شاشة (مفيد للأجهزة التي تظهر فيها مشكلة الـ first screen).

## 5. لماذا WebGPU يسبب ألواناً خاطئة على iOS (تشخيصك)

- `crates/cranpose-render/common/src/text_mask_gamma.rs` يصحّح أقنعة النص بافتراض أن الوجهة لها سطوع معاكس، ويُطبّق في sRGB مُشفّر.
- على iOS (WebKit، Safari وChrome معاً) WebGPU يدمج بطريقة مختلفة → النص الفاتح يُرسم داكناً.
- WebGL2 على iOS سليم → `?backend=gl` يحلها.

## 6. خلاصة الفحص قبل نشر موقعك

1. ✅ `cargo check -p cranpose --target wasm32-unknown-unknown`
2. ✅ افتح على iOS بلا معامل — قارن الألوان
3. ✅ افتح بـ `?backend=gl` — يجب أن يطابق الحاسوب
4. ✅ اقرأ سجل `Web backend preference=..., selected backend=...` في Console
5. ✅ إن اختلفت الصورتان → ثبّت `Gl` بالطريقة 2، أو انشر رابطاً بـ `?backend=gl`
