//! PWA support for Cranpose web apps.
//!
//! This crate is additive and independent of `cranpose-video`. It gives a web
//! shell everything it needs to be an installable, offline-capable PWA and to
//! work around the WebGPU colour bug on iOS WebKit:
//!
//! - [`device`]: detect iOS / Android / desktop and browser capabilities;
//! - [`backend`]: inject `?backend=gl` automatically on WebKit (no user action);
//! - [`install`]: capture `beforeinstallprompt`, track install state, prompt;
//! - [`service_worker`]: register the worker;
//! - [`manifest`]: build a valid `manifest.webmanifest`;
//! - [`assets`]: ready-to-paste service worker / bootstrap JS and iOS meta tags;
//! - [`Pwa`]: one façade that ties detection + install state together.
//!
//! Everything is typed and compiles on native targets too (as honest no-ops),
//! so this crate can live in a cross-platform workspace beside `cranpose-video`.

// The crate is documented at module/type level; serialised data fields use
// self-describing names, so missing_docs is intentionally not enforced.

pub mod assets;
pub mod backend;
pub mod build;
pub mod cache;
pub mod device;
pub mod install;
pub mod manifest;
pub mod preload;
pub mod service_worker;

use device::DeviceInfo;
use install::{InstallAdvice, InstallState, Installability};

/// One façade over detection + install state, owned by the app shell.
#[derive(Clone, Debug, Default)]
pub struct Pwa {
    info: DeviceInfo,
    install: InstallState,
}

impl Pwa {
    /// Detects the device and prepares install handling. On native targets the
    /// info is `Unknown` and all browser actions are no-ops.
    pub fn detect() -> Self {
        let this = Self {
            info: device::detect(),
            install: InstallState::new(),
        };
        // Apply the iOS WebGL2 workaround before the shell reads `?backend`.
        this.apply_backend_fix();
        this
    }

    /// Applies the automatic WebGL2 workaround. Returns whether a change was
    /// made in the browser URL.
    pub fn apply_backend_fix(&self) -> bool {
        backend::apply_in_browser(&self.info)
    }

    /// The detected device snapshot.
    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    /// The install state handle (capture prompts, dismiss, prompt).
    pub fn install(&self) -> &InstallState {
        &self.install
    }

    /// Mutable install state, for the event listener to capture prompts.
    pub fn install_mut(&mut self) -> &mut InstallState {
        &mut self.install
    }

    /// Current installability.
    pub fn installability(&self) -> Installability {
        self.install.installability(&self.info)
    }

    /// What UI to show right now.
    pub fn advice(&self) -> InstallAdvice {
        self.install.advice(&self.info)
    }

    /// Registers the service worker at `sw.js`.
    pub fn register_service_worker(&self) -> service_worker::RegistrationResult {
        service_worker::register("/sw.js")
    }

    /// Registers a `beforeinstallprompt` listener that feeds this Pwa's state.
    /// The returned handle must be kept alive by the caller on wasm (the
    /// listener is intentionally leaked; on native this is a no-op marker).
    pub fn listen_for_install(&mut self) -> bool {
        // We cannot move `self` into the closure; instead we capture a marker
        // and let the caller poll `installability()`, which re-evaluates from
        // the captured prompt stored via `capture_prompt_from_event`.
        install::listen_for_prompt(|prompt| {
            // Store the prompt in a thread-local slot; `capture_prompt()` drains
            // it on the next call. This avoids needing a shared mutable borrow
            // across the 'static event callback.
            crate::runtime::store_prompt(prompt);
        })
    }

    /// Drains a prompt captured by the event listener into this state.
    pub fn capture_prompt(&mut self) -> bool {
        match crate::runtime::take_prompt() {
            Some(prompt) => {
                self.install.capture(prompt);
                true
            }
            None => false,
        }
    }
}

/// Internal storage bridging the `'static` JS event callback to `Pwa`.
mod runtime {
    use crate::install::InstallPrompt;

    #[cfg(target_arch = "wasm32")]
    std::thread_local! {
        static CAPTURED: std::cell::RefCell<Vec<InstallPrompt>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    pub(super) fn store_prompt(prompt: InstallPrompt) {
        #[cfg(target_arch = "wasm32")]
        CAPTURED.with(|slot| slot.borrow_mut().push(prompt));
        #[cfg(not(target_arch = "wasm32"))]
        let _ = prompt;
    }

    pub(super) fn take_prompt() -> Option<InstallPrompt> {
        #[cfg(target_arch = "wasm32")]
        {
            CAPTURED.with(|slot| slot.borrow_mut().pop())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            None
        }
    }
}

/// Convenient imports.
pub mod prelude {
    pub use crate::{
        Pwa,
        assets::{BOOTSTRAP_JS, IOS_META, SERVICE_WORKER_JS},
        backend::{BACKEND_PARAM, GL_VALUE, compatible_url, needs_gl_workaround},
        build::{PwaBuildConfig, emit},
        cache::{CachePolicy, CacheRule, CacheStrategy},
        device::{DeviceFamily, DeviceInfo, detect},
        install::{
            InstallAdvice, InstallPrompt, InstallState, Installability, advice, listen_for_prompt,
        },
        manifest::{DisplayMode, Icon, WebAppManifest},
        preload::{AsKind, modulepreload_tag, preload_tag, wasm_head_bundle},
        service_worker::{RegistrationResult, register},
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use device::DeviceFamily;

    #[test]
    fn native_pwa_is_honest() {
        let pwa = Pwa::detect();
        assert_eq!(pwa.info().family, DeviceFamily::Unknown);
        assert_eq!(pwa.installability(), Installability::NotInstallable);
        assert_eq!(pwa.advice(), InstallAdvice::Hide);
    }

    #[test]
    fn applying_fix_on_native_is_noop() {
        let pwa = Pwa::detect();
        assert!(!pwa.apply_backend_fix());
    }
}
