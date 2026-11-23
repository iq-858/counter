//! Automatic WebGL2 selection on browsers whose WebGPU is known-broken.
//!
//! Cranpose's `WebBackendPreference::Auto` picks WebGPU on any browser that
//! exposes `navigator.gpu`. On Apple WebKit (Safari AND Chrome on iOS) the
//! WebGPU path renders text with wrong colours. This module injects
//! `?backend=gl` into the URL before the WASM shell reads it — the user never
//! has to type anything.

use crate::device::DeviceInfo;

/// The query parameter Cranpose reads in `backend_preference.rs`.
pub const BACKEND_PARAM: &str = "backend";
/// The value that forces the WebGL2 backend.
pub const GL_VALUE: &str = "gl";

/// Whether a device needs the WebGPU workaround.
///
/// `[SOURCE-VERIFIED]`: both Safari and Chrome on iOS use WebKit (Apple's
/// requirement); their WebGPU backend renders text dark-on-dark in Cranpose.
/// WebGL2 is correct there. Other platforms keep their native choice.
pub fn needs_gl_workaround(info: &DeviceInfo) -> bool {
    info.family.is_webkit()
        // Only force GL when the browser actually offers WebGPU — if it does
        // not, Cranpose's own `auto_instance` already falls back to GL.
        && info.has_webgpu
}

/// Computes the URL that should be used for the current device.
///
/// Returns `None` when nothing needs changing (the caller keeps its URL).
/// On native targets always returns `None` — there is no URL to rewrite.
pub fn compatible_url(info: &DeviceInfo, current: &str) -> Option<String> {
    if !needs_gl_workaround(info) {
        return None;
    }
    set_query_param(current, BACKEND_PARAM, GL_VALUE)
}

/// Sets a query parameter on a URL string, preserving all other parameters.
/// Pure function so it can be unit-tested on any target.
pub fn set_query_param(url: &str, key: &str, value: &str) -> Option<String> {
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, q),
        None => (url, ""),
    };
    let mut pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            Some((k.to_string(), v.to_string()))
        })
        .filter(|(k, _)| k != key)
        .collect();
    pairs.push((key.to_string(), value.to_string()));
    let joined = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    Some(format!("{base}?{joined}"))
}

/// Applies the workaround in the browser (wasm32 only).
///
/// Uses `history.replaceState` so the address bar stays clean and no reload
/// happens — the shell then reads `backend=gl` naturally. On native targets
/// this is a no-op.
pub fn apply_in_browser(info: &DeviceInfo) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        if !needs_gl_workaround(info) {
            return false;
        }
        let window = match web_sys::window() {
            Some(w) => w,
            None => return false,
        };
        let location = window.location();
        let href = match location.href() {
            Ok(h) => h,
            Err(_) => return false,
        };
        let current_backend = location
            .search()
            .ok()
            .and_then(|s| {
                web_sys::UrlSearchParams::new_with_str(&s)
                    .ok()
                    .and_then(|p| p.get(BACKEND_PARAM))
            })
            .unwrap_or_default();
        // Respect an explicit `webgpu` override, but fill in `auto`/absent.
        if current_backend == "webgpu" {
            return false;
        }
        if let Some(next) = compatible_url(info, &href) {
            // web-sys 0.3.107 exposes replaceState(data, title) only, but the
            // DOM signature is (data, unused, url). Call it through Reflect so
            // the URL is actually rewritten without a reload.
            if let Ok(history) = window.history() {
                let args = js_sys::Array::new();
                args.push(&wasm_bindgen::JsValue::NULL);
                args.push(&wasm_bindgen::JsValue::from_str(""));
                args.push(&wasm_bindgen::JsValue::from_str(&next));
                if let Ok(f) = js_sys::Reflect::get(&history, &wasm_bindgen::JsValue::from_str("replaceState")) {
                    if let Some(fun) = f.dyn_ref::<js_sys::Function>() {
                        return fun.apply(&history, &args).is_ok();
                    }
                }
            }
            return false;
        }
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = info;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceFamily;

    fn webkit_info() -> DeviceInfo {
        DeviceInfo {
            family: DeviceFamily::IPhone,
            has_webgpu: true,
            ..DeviceInfo::default()
        }
    }

    #[test]
    fn webgpu_webkit_needs_gl() {
        assert!(needs_gl_workaround(&webkit_info()));
    }

    #[test]
    fn non_webkit_keeps_its_backend() {
        let info = DeviceInfo {
            family: DeviceFamily::Android,
            has_webgpu: true,
            ..DeviceInfo::default()
        };
        assert!(!needs_gl_workaround(&info));
    }

    #[test]
    fn no_webgpu_needs_no_workaround() {
        let info = DeviceInfo {
            family: DeviceFamily::IPhone,
            has_webgpu: false,
            ..DeviceInfo::default()
        };
        assert!(!needs_gl_workaround(&info));
    }

    #[test]
    fn injects_backend_param() {
        let next = compatible_url(&webkit_info(), "https://example.com/demo?tab=home").unwrap();
        assert!(next.contains("backend=gl"));
        assert!(next.contains("tab=home"));
    }

    #[test]
    fn no_change_for_desktop() {
        let info = DeviceInfo {
            family: DeviceFamily::Desktop,
            ..DeviceInfo::default()
        };
        assert!(compatible_url(&info, "https://example.com/demo").is_none());
    }

    #[test]
    fn set_query_param_replaces_existing() {
        let out = set_query_param("https://x/y?backend=webgpu&a=1", "backend", "gl").unwrap();
        assert_eq!(out, "https://x/y?a=1&backend=gl");
    }
}
