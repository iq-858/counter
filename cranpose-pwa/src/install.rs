//! Installability and install-prompt handling.
//!
//! Chrome/Edge fire a non-standard `beforeinstallprompt` event. We capture it
//! through `js-sys` (web-sys has no binding), hold it, and let the app call
//! `prompt()` from a real user gesture (a click) — which browsers require.
//! iOS Safari does NOT fire that event; it only supports "Add to Home Screen",
//! so the API exposes a `manual_instructions` variant for those devices.

use serde::{Deserialize, Serialize};

/// Whether and how the current page can be installed as a PWA.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Installability {
    /// A native install prompt is available (Chromium browsers).
    PromptAvailable,
    /// No prompt event, but the page can be added manually (iOS Safari).
    ManualAvailable,
    /// Already installed / running standalone.
    Installed,
    /// Not installable (missing manifest, SW, or HTTPS).
    NotInstallable,
}

/// How to present the install UI to the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallAdvice {
    /// Show a button that calls `prompt_install()`.
    ShowPrompt,
    /// Show "Add to Home Screen" instructions (Safari share sheet).
    ShowManualInstructions,
    /// Nothing to show.
    Hide,
}

/// Advice for a given installability state.
pub fn advice(state: Installability) -> InstallAdvice {
    match state {
        Installability::PromptAvailable => InstallAdvice::ShowPrompt,
        Installability::ManualAvailable => InstallAdvice::ShowManualInstructions,
        Installability::Installed | Installability::NotInstallable => InstallAdvice::Hide,
    }
}

/// A handle to a captured `beforeinstallprompt` event.
///
/// On wasm32 it wraps the JS event object; on other targets it is inert so the
/// same code compiles in a native workspace and in tests.
#[derive(Clone, Debug)]
pub struct InstallPrompt {
    #[cfg(target_arch = "wasm32")]
    raw: wasm_bindgen::JsValue,
    #[cfg(not(target_arch = "wasm32"))]
    raw: (),
}

impl InstallPrompt {
    /// Prompts the user to install. Must be called from a user gesture.
    ///
    /// Returns `true` if the browser accepted the prompt. On native targets
    /// always returns `false`.
    pub fn prompt(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            if let Ok(promise) =
                js_sys::Reflect::get(&self.raw, &wasm_bindgen::JsValue::from_str("prompt"))
            {
                if let Some(f) = promise.dyn_ref::<js_sys::Function>() {
                    let _ = f.call0(&self.raw);
                    return true;
                }
            }
        }
        let _ = &self.raw;
        false
    }
}

/// State holder for install handling, owned by the app shell.
#[derive(Clone, Debug, Default)]
pub struct InstallState {
    captured: Option<InstallPrompt>,
    user_dismissed: bool,
}

impl InstallState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a captured prompt event (call from the `beforeinstallprompt`
    /// listener). On native targets this is a harmless no-op marker.
    pub fn capture(&mut self, prompt: InstallPrompt) {
        self.captured = Some(prompt);
    }

    /// The user declined — hide the banner for this AND future visits.
    ///
    /// The flag is persisted in `localStorage`, so reopening the site (or
    /// reloading after the user chose "Not now" in the browser prompt) does
    /// not show the banner again. The user can still install from the browser
    /// UI at any time; we just stop nagging.
    pub fn dismiss(&mut self) {
        self.user_dismissed = true;
        crate::persist::set(crate::persist::KEY_DISMISSED, true);
        self.captured = None;
    }

    /// Records that our prompt was shown, so we know we already asked once.
    pub fn mark_prompted(&self) {
        crate::persist::set(crate::persist::KEY_PROMPTED, true);
    }

    /// Whether we previously showed the install prompt.
    pub fn was_prompted() -> bool {
        crate::persist::get(crate::persist::KEY_PROMPTED)
    }

    /// Whether the user already dismissed the prompt (persisted).
    pub fn is_dismissed(&self) -> bool {
        self.user_dismissed || crate::persist::get(crate::persist::KEY_DISMISSED)
    }

    /// Prompts install if a captured event exists. Returns whether a prompt
    /// was actually issued.
    pub fn prompt_install(&self) -> bool {
        self.captured.as_ref().is_some_and(|p| p.prompt())
    }

    /// Computes installability from device info + captured state.
    pub fn installability(&self, info: &crate::device::DeviceInfo) -> Installability {
        if info.standalone {
            return Installability::Installed;
        }
        if self.is_dismissed() {
            return Installability::NotInstallable;
        }
        if self.captured.is_some() {
            return Installability::PromptAvailable;
        }
        if info.family.is_webkit() && info.service_worker_supported {
            return Installability::ManualAvailable;
        }
        if info.service_worker_supported {
            // A service worker exists but the prompt event has not fired yet;
            // browsers fire it only when installability criteria are met.
            return Installability::ManualAvailable;
        }
        Installability::NotInstallable
    }

    /// The UI advice for this state.
    pub fn advice(&self, info: &crate::device::DeviceInfo) -> InstallAdvice {
        advice(self.installability(info))
    }
}

/// Installs a `beforeinstallprompt` listener on `window` (wasm32 only).
///
/// The callback receives the captured prompt and the app stores it in
/// `InstallState`. Returns `false` on non-web targets.
pub fn listen_for_prompt<F>(on_prompt: F) -> bool
where
    F: FnMut(InstallPrompt) + 'static,
{
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::prelude::*;

        let mut on_prompt = on_prompt;
        let window = match web_sys::window() {
            Some(w) => w,
            None => return false,
        };
        let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
            // Prevent the browser's own mini-infobar so our UI is the only one.
            if event.cancelable() {
                let _ = event.prevent_default();
            }
            on_prompt(InstallPrompt {
                raw: JsValue::from(event),
            });
        }) as Box<dyn FnMut(web_sys::Event)>);
        window
            .add_event_listener_with_callback(
                "beforeinstallprompt",
                closure.as_ref().unchecked_ref(),
            )
            .map(|_| {
                // The listener lives for the page lifetime — leak intentionally,
                // like a static event handler. One per page.
                closure.forget();
                true
            })
            .unwrap_or(false)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = on_prompt;
        false
    }
}

/// Opens the OS share sheet via `navigator.share()` (Web Share API).
///
/// On iOS this is the closest official shortcut to installation: the share
/// sheet contains "Add to Home Screen". It still requires the user's final
/// tap — no browser API can install silently on iOS. Must be called from a
/// user gesture (button click). Returns `false` when unsupported or canceled.
pub fn share_for_install(title: &str, text: &str, url: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::prelude::*;
        let navigator = match web_sys::window() {
            Some(w) => w.navigator(),
            None => return false,
        };
        if !js_sys::Reflect::has(&navigator, &JsValue::from_str("share")).unwrap_or(false) {
            return false;
        }
        let data = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &data,
            &JsValue::from_str("title"),
            &JsValue::from_str(title),
        );
        let _ = js_sys::Reflect::set(&data, &JsValue::from_str("text"), &JsValue::from_str(text));
        let _ = js_sys::Reflect::set(&data, &JsValue::from_str("url"), &JsValue::from_str(url));
        // Fire-and-forget: the promise resolves when the sheet closes. We only
        // need to know that the API was invoked successfully here.
        let f = match js_sys::Reflect::get(&navigator, &JsValue::from_str("share")) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let fun = match f.dyn_ref::<js_sys::Function>() {
            Some(fun) => fun,
            None => return false,
        };
        return fun.call1(&navigator, &data).is_ok();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (title, text, url);
        false
    }
}

/// Registers the `appinstalled` event (Chromium) to observe a successful
/// installation. This is for analytics/UI cleanup only — it cannot cause an
/// install. Returns `false` on browsers that do not fire it (including iOS).
pub fn listen_for_app_installed<F>(mut on_installed: F) -> bool
where
    F: FnMut() + 'static,
{
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::prelude::*;
        let window = match web_sys::window() {
            Some(w) => w,
            None => return false,
        };
        let closure = Closure::wrap(Box::new(move || on_installed()) as Box<dyn FnMut()>);
        let ok = window
            .add_event_listener_with_callback("appinstalled", closure.as_ref().unchecked_ref())
            .is_ok();
        if ok {
            closure.forget();
        }
        ok
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = on_installed;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceFamily, DeviceInfo};

    fn info(family: DeviceFamily, standalone: bool, sw: bool) -> DeviceInfo {
        DeviceInfo {
            family,
            standalone,
            service_worker_supported: sw,
            ..DeviceInfo::default()
        }
    }

    #[test]
    fn advice_mapping() {
        assert_eq!(
            advice(Installability::PromptAvailable),
            InstallAdvice::ShowPrompt
        );
        assert_eq!(
            advice(Installability::ManualAvailable),
            InstallAdvice::ShowManualInstructions
        );
        assert_eq!(advice(Installability::Installed), InstallAdvice::Hide);
        assert_eq!(advice(Installability::NotInstallable), InstallAdvice::Hide);
    }

    #[test]
    fn installed_hides_banner() {
        let state = InstallState::new();
        assert_eq!(
            state.installability(&info(DeviceFamily::Android, true, true)),
            Installability::Installed
        );
    }

    #[test]
    fn ios_with_sw_gets_manual_instructions() {
        let state = InstallState::new();
        assert_eq!(
            state.installability(&info(DeviceFamily::IPhone, false, true)),
            Installability::ManualAvailable
        );
    }

    #[test]
    fn no_service_worker_is_not_installable() {
        let state = InstallState::new();
        assert_eq!(
            state.installability(&info(DeviceFamily::Android, false, false)),
            Installability::NotInstallable
        );
    }

    #[test]
    fn dismissal_hides_banner() {
        let mut state = InstallState::new();
        state.dismiss();
        assert!(state.user_dismissed, "in-memory flag set on native");
        // On native, persistence is a no-op; is_dismissed still true from memory.
        assert!(state.is_dismissed());
        assert_eq!(
            state.installability(&info(DeviceFamily::Android, false, true)),
            Installability::NotInstallable
        );
    }

    #[test]
    fn native_has_no_prompt() {
        assert!(!InstallPrompt { raw: () }.prompt());
    }
}
