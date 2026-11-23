//! WASM probe that exposes the PWA detection as JS-callable functions.
//! The demo page calls these to display device info, installability and the
//! automatic WebGL2 workaround status.

use cranpose_pwa::Pwa;
use wasm_bindgen::prelude::*;

/// Runs detection + backend fix and returns a JSON report.
#[wasm_bindgen]
pub fn probe() -> String {
    let pwa = Pwa::detect();
    let fix_applied = pwa.apply_backend_fix();
    let info = pwa.info();
    let mut report = serde_json::json!({
        "device_family": format!("{:?}", info.family),
        "user_agent": info.user_agent,
        "has_webgpu": info.has_webgpu,
        "has_webgl2": info.has_webgl2,
        "service_worker_supported": info.service_worker_supported,
        "standalone": info.standalone,
        "webkit": info.family.is_webkit(),
        "needs_gl_workaround": cranpose_pwa::backend::needs_gl_workaround(info),
        "fix_applied": fix_applied,
        "installability": format!("{:?}", pwa.installability()),
        "advice": format!("{:?}", pwa.advice()),
        "dismissed": pwa.install().is_dismissed(),
        "was_prompted": cranpose_pwa::install::InstallState::was_prompted(),
    });
    if let Some(url) = cranpose_pwa::backend::compatible_url(
        info,
        &web_sys::window().and_then(|w| w.location().href().ok()).unwrap_or_default(),
    ) {
        report["recommended_url"] = serde_json::Value::String(url);
    }
    report.to_string()
}
