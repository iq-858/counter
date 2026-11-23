# cranpose-pwa

PWA support for [Cranpose](https://github.com/samoylenkodmitry/Cranpose) web apps.
Additive, independent of `cranpose-video`, and compiles on native targets too.

## What it provides

| Module | Purpose |
|---|---|
| `device` | Detect iPhone / iPad / Android / desktop, WebGPU / WebGL2 / SW support, standalone mode |
| `backend` | Auto-inject `?backend=gl` on iOS WebKit (fixes wrong text colours from WebGPU) |
| `install` | Capture `beforeinstallprompt`, track installability, prompt, manual iOS instructions |
| `service_worker` | Register the worker |
| `manifest` | Build a valid `manifest.webmanifest` |
| `assets` | Ready-to-paste service worker JS, bootstrap JS, iOS meta tags |
| `Pwa` | One façade tying detection + install together |

## Use in a Cranpose web shell

```rust,ignore
use cranpose_pwa::prelude::*;

let mut pwa = Pwa::detect();       // detects device, applies backend=gl on iOS
pwa.listen_for_install();          // capture beforeinstallprompt
let _ = pwa.register_service_worker();
pwa.capture_prompt();              // drain captured prompt into state
match pwa.advice() {
    InstallAdvice::ShowPrompt => { /* show your install button; call pwa.install().prompt_install() */ }
    InstallAdvice::ShowManualInstructions => { /* "Add to Home Screen" hint */ }
    InstallAdvice::Hide => {}
}
```

Paste `assets::BOOTSTRAP_JS` before the shell reads `?backend`, and `assets::IOS_META`
in `<head>`. Write `assets::SERVICE_WORKER_JS` to `sw.js` and `WebAppManifest::new(..).to_json()`
to `manifest.webmanifest`.

## Demo site

`demo/` is a standalone, installable PWA that displays live detection results.
It works with pure JavaScript even before the WASM probe is built, so you can
test on iOS immediately. See `demo/index.html`.

## Integration into upstream Cranpose

This crate is intentionally separate so anyone can drop it into
`samoylenkodmitry/Cranpose` as a workspace member:
```toml
# in the upstream root Cargo.toml
members = ["crates/cranpose-pwa"]
```
No upstream file needs to change to use it; the web shell opts in by calling
`Pwa::detect()`.

## Tests
`cargo test --manifest-path cranpose-pwa/Cargo.toml --lib` — 25 passing.
