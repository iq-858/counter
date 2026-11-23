//! Service worker registration helpers.
//!
//! The service worker file itself is generated as a static asset (see
//! `assets::service_worker_js`). This module registers it in the browser and
//! reports the result. On non-web targets registration is a typed no-op.

/// Result of a service worker registration attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationResult {
    Registered,
    AlreadyControlled,
    Unsupported,
    Failed,
}

impl RegistrationResult {
    pub fn is_ok(self) -> bool {
        matches!(self, Self::Registered | Self::AlreadyControlled)
    }
}

/// Registers a service worker at `sw.js` (or a custom URL) on wasm32.
pub fn register(path: &str) -> RegistrationResult {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;
        use wasm_bindgen_futures::JsFuture;

        let window = match web_sys::window() {
            Some(w) => w,
            None => return RegistrationResult::Unsupported,
        };
        let navigator = window.navigator();
        // `service_worker()` returns the container directly; check the property
        // to know whether the browser supports service workers at all.
        if !js_sys::Reflect::has(&navigator, &JsValue::from_str("serviceWorker")).unwrap_or(false) {
            return RegistrationResult::Unsupported;
        }
        let container = navigator.service_worker();
        // If already controlled, nothing to do.
        if container.controller().is_some() {
            return RegistrationResult::AlreadyControlled;
        }
        // register() returns a Promise; fire-and-forget is fine for a PWA shell
        // — installability will be reported by the beforeinstallprompt event.
        // `register()` returns a Promise directly (no Result wrapper here).
        let _ = JsFuture::from(container.register(path));
        RegistrationResult::Registered
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = path;
        RegistrationResult::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_states() {
        assert!(RegistrationResult::Registered.is_ok());
        assert!(RegistrationResult::AlreadyControlled.is_ok());
        assert!(!RegistrationResult::Unsupported.is_ok());
        assert!(!RegistrationResult::Failed.is_ok());
    }

    #[test]
    fn native_register_is_unsupported() {
        assert_eq!(register("/sw.js"), RegistrationResult::Unsupported);
    }
}
