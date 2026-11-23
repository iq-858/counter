//! Persistent flags in `localStorage`, so the install banner is not shown
//! again after the user dismissed it or installed the app.
//!
//! On non-wasm targets this is an in-memory no-op so the crate stays usable in
//! a native workspace and in tests.

/// Key used to remember that the user declined the install banner.
pub const KEY_DISMISSED: &str = "cranpose_pwa_install_dismissed";
/// Key used to remember that the app was installed from our prompt.
pub const KEY_PROMPTED: &str = "cranpose_pwa_install_prompted";

/// Reads a boolean flag from localStorage (wasm) or memory (native).
pub fn get(key: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let window = match web_sys::window() {
            Some(w) => w,
            None => return false,
        };
        let storage = match window.local_storage() {
            Ok(Some(s)) => s,
            _ => return false,
        };
        storage.get_item(key).ok().flatten().as_deref() == Some("1")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        false
    }
}

/// Writes a boolean flag to localStorage (wasm) or ignores it (native).
pub fn set(key: &str, value: bool) {
    #[cfg(target_arch = "wasm32")]
    {
        let window = match web_sys::window() {
            Some(w) => w,
            None => return,
        };
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = if value {
                storage.set_item(key, "1")
            } else {
                storage.remove_item(key)
            };
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_persist_is_noop() {
        set(KEY_DISMISSED, true);
        assert!(!get(KEY_DISMISSED));
    }
}
