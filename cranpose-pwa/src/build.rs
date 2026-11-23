//! Build-time asset generation.
//!
//! Call this from your application's `build.rs` to generate the three static
//! files a PWA needs (manifest, service worker, bootstrap HTML snippet).
//! The developer writes NO hand-written JS/HTML — only a `build.rs` that calls
//! `emit("assets", config)`, plus a `<link rel="manifest">` in the page head.
//!
//! Note: a browser REQUIRES the manifest and service worker to be real files
//! served over HTTP(S). Rust cannot inject them after page load. This module
//! automates their creation, not their existence.

use std::{fs, path::Path};

use crate::{assets, cache::CachePolicy, manifest::WebAppManifest};

/// Configuration for generated assets.
#[derive(Clone, Debug)]
pub struct PwaBuildConfig {
    /// App name shown in the install prompt.
    pub name: String,
    /// Short name for the home screen.
    pub short_name: String,
    /// Where to write the generated files.
    pub out_dir: String,
    /// URL of the WASM bundle (to precache + preload), e.g. `/pkg/app_bg.wasm`.
    pub wasm_url: String,
    /// URL of the JS glue, e.g. `/pkg/app.js`.
    pub js_url: String,
}

impl Default for PwaBuildConfig {
    fn default() -> Self {
        Self {
            name: "Cranpose App".to_owned(),
            short_name: "Cranpose".to_owned(),
            out_dir: "assets".to_owned(),
            wasm_url: "/pkg/app_bg.wasm".to_owned(),
            js_url: "/pkg/app.js".to_owned(),
        }
    }
}

/// What `emit` generated, for logging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generated {
    pub manifest: String,
    pub service_worker: String,
    pub head_snippet: String,
}

/// Generates `manifest.webmanifest`, `sw.js` and `head-snippet.html` into
/// `config.out_dir`. Idempotent: safe to call on every build.
///
/// Returns the paths of the three written files.
pub fn emit(config: &PwaBuildConfig) -> Result<Generated, String> {
    let dir = Path::new(&config.out_dir);
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;

    let manifest = WebAppManifest::new(config.name.clone(), config.short_name.clone());
    let preload_hint = vec![
        crate::preload::modulepreload_tag(&config.js_url),
        crate::preload::preload_tag(&config.wasm_url, crate::preload::AsKind::Fetch, true),
    ];

    let mut policy = CachePolicy::wasm_app();
    if !policy.precache.iter().any(|p| p == &config.wasm_url) {
        policy.precache.push(config.wasm_url.clone());
    }
    if !policy.precache.iter().any(|p| p == &config.js_url) {
        policy.precache.push(config.js_url.clone());
    }

    let manifest_path = dir.join("manifest.webmanifest");
    let sw_path = dir.join("sw.js");
    let head_path = dir.join("head-snippet.html");

    fs::write(&manifest_path, manifest.to_json())
        .map_err(|e| format!("write {}: {e}", manifest_path.display()))?;
    fs::write(&sw_path, policy.to_service_worker())
        .map_err(|e| format!("write {}: {e}", sw_path.display()))?;
    let head = format!("{}\n{}", assets::IOS_META, preload_hint.join("\n"));
    fs::write(&head_path, &head).map_err(|e| format!("write {}: {e}", head_path.display()))?;

    Ok(Generated {
        manifest: manifest_path.to_string_lossy().into_owned(),
        service_worker: sw_path.to_string_lossy().into_owned(),
        head_snippet: head_path.to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_writes_three_files() {
        let root = std::env::temp_dir().join(format!("cranpose-pwa-test-{}", std::process::id()));
        let cfg = PwaBuildConfig {
            out_dir: root.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let out = emit(&cfg).expect("emit succeeds");
        assert!(std::path::Path::new(&out.manifest).exists());
        assert!(std::path::Path::new(&out.service_worker).exists());
        assert!(std::path::Path::new(&out.head_snippet).exists());
        let sw = fs::read_to_string(&out.service_worker).unwrap();
        assert!(sw.contains("app_bg.wasm"));
        let head = fs::read_to_string(&out.head_snippet).unwrap();
        assert!(head.contains("modulepreload"));
        let _ = fs::remove_dir_all(&root);
    }
}
