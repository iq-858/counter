//! Device and platform detection for PWA decisions.
//!
//! On wasm32 the detection reads the browser's user agent and capabilities.
//! On other targets the crate compiles with typed, honest "unknown" values so
//! it can live in a cross-platform workspace without `cfg` at call sites.

use serde::{Deserialize, Serialize};

/// The family of device a browser is running on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceFamily {
    /// An iPhone or iPod touch.
    IPhone,
    /// An iPad (including desktop-class iPadOS user agents).
    IPad,
    /// An Android phone or tablet.
    Android,
    /// A desktop or laptop computer.
    Desktop,
    /// Unknown (non-web target, or an unrecognised agent).
    Unknown,
}

impl DeviceFamily {
    /// Whether this device runs Apple's WebKit engine.
    ///
    /// Both Safari and Chrome on iOS use WebKit, and their WebGPU path renders
    /// Cranpose text with wrong colours. Callers should force the `gl` backend
    /// when this is true (see `cranpose_pwa::backend::ensure_compatible`).
    pub fn is_webkit(self) -> bool {
        matches!(self, Self::IPhone | Self::IPad)
    }

    /// Whether this device is a mobile form factor.
    pub fn is_mobile(self) -> bool {
        matches!(self, Self::IPhone | Self::IPad | Self::Android)
    }
}

/// A snapshot of everything the shell needs to decide how to present a PWA.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub family: DeviceFamily,
    /// Raw user agent string (empty on non-web targets).
    pub user_agent: String,
    /// Whether `navigator.gpu` exists.
    pub has_webgpu: bool,
    /// Whether a WebGL2 context can be created.
    pub has_webgl2: bool,
    /// Whether a service worker can be registered (HTTPS or localhost / non-web stub).
    pub service_worker_supported: bool,
    /// Whether the page is already running in standalone/installed mode.
    pub standalone: bool,
}

impl Default for DeviceInfo {
    fn default() -> Self {
        Self {
            family: DeviceFamily::Unknown,
            user_agent: String::new(),
            has_webgpu: false,
            has_webgl2: false,
            service_worker_supported: false,
            standalone: false,
        }
    }
}

/// Detect the current device from the browser.
///
/// On non-wasm targets this returns a default `Unknown` snapshot — useful for
/// tests and for code shared between the web shell and a native target.
pub fn detect() -> DeviceInfo {
    #[cfg(target_arch = "wasm32")]
    {
        detect_web()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        DeviceInfo::default()
    }
}

#[cfg(target_arch = "wasm32")]
fn detect_web() -> DeviceInfo {
    use wasm_bindgen::JsCast;
    use web_sys::Window;

    let window = match web_sys::window() {
        Some(w) => w,
        None => return DeviceInfo::default(),
    };
    let navigator = window.navigator();
    let ua = navigator.user_agent().unwrap_or_default();
    let has_webgpu =
        js_sys::Reflect::has(&window, &wasm_bindgen::JsValue::from_str("gpu")).unwrap_or(false);
    let has_webgl2 = window
        .document()
        .and_then(|d| d.create_element("canvas").ok())
        .and_then(|c| c.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .and_then(|c| c.get_context("webgl2").ok().flatten())
        .is_some();
    let service_worker_supported = js_sys::Reflect::has(
        &navigator,
        &wasm_bindgen::JsValue::from_str("serviceWorker"),
    )
    .unwrap_or(false);
    // `(display-mode: standalone)` is the standards-based signal; on iOS the
    // legacy `navigator.standalone` property also reports it.
    let standalone = window
        .match_media("(display-mode: standalone)")
        .ok()
        .flatten()
        .map(|m| m.matches())
        .or_else(|| {
            js_sys::Reflect::get(&navigator, &wasm_bindgen::JsValue::from_str("standalone"))
                .ok()
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false);

    let family = classify(&ua, &window);
    DeviceInfo {
        family,
        user_agent: ua,
        has_webgpu,
        has_webgl2,
        service_worker_supported,
        standalone,
    }
}

#[cfg(target_arch = "wasm32")]
fn classify(ua: &str, window: &web_sys::Window) -> DeviceFamily {
    let _ = window;
    let ua_lower = ua.to_lowercase();
    if ua_lower.contains("iphone") || ua_lower.contains("ipod") {
        DeviceFamily::IPhone
    } else if ua_lower.contains("ipad")
        || (ua_lower.contains("macintosh") && window.navigator().max_touch_points() > 1)
    {
        // iPadOS reports a desktop-class Mac user agent; touch points > 1
        // distinguish it from a real Mac.
        DeviceFamily::IPad
    } else if ua_lower.contains("android") {
        DeviceFamily::Android
    } else if ua_lower.contains("mobile") {
        DeviceFamily::Unknown
    } else {
        DeviceFamily::Desktop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webkit_is_ios_families_only() {
        assert!(DeviceFamily::IPhone.is_webkit());
        assert!(DeviceFamily::IPad.is_webkit());
        assert!(!DeviceFamily::Android.is_webkit());
        assert!(!DeviceFamily::Desktop.is_webkit());
    }

    #[test]
    fn mobile_families() {
        assert!(DeviceFamily::IPhone.is_mobile());
        assert!(DeviceFamily::IPad.is_mobile());
        assert!(DeviceFamily::Android.is_mobile());
        assert!(!DeviceFamily::Desktop.is_mobile());
    }

    #[test]
    fn default_is_unknown_on_native() {
        let info = DeviceInfo::default();
        assert_eq!(info.family, DeviceFamily::Unknown);
        assert!(!info.has_webgpu);
    }
}
