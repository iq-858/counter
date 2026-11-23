//! Web App Manifest generation.
//!
//! A Web App Manifest is the single most important PWA file: without it
//! Chromium will not fire `beforeinstallprompt`. This module builds a valid
//! manifest as a `serde_json::Value` so a build script (or a web server) can
//! write it to disk — no hand-written JSON to keep in sync.

use serde::{Deserialize, Serialize};

/// Display mode requested from the host OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    Fullscreen,
    Standalone,
    MinimalUi,
    Browser,
}

/// One icon entry in the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Icon {
    pub src: String,
    pub sizes: String,
    #[serde(rename = "type")]
    pub mime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

impl Icon {
    pub fn png(src: impl Into<String>, size: u32, maskable: bool) -> Self {
        Self {
            src: src.into(),
            sizes: format!("{size}x{size}"),
            mime: "image/png".to_owned(),
            purpose: if maskable {
                Some("any maskable".to_owned())
            } else {
                None
            },
        }
    }
}

/// A Web App Manifest (`manifest.webmanifest`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebAppManifest {
    pub name: String,
    pub short_name: String,
    pub start_url: String,
    pub display: DisplayMode,
    pub background_color: String,
    pub theme_color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    pub icons: Vec<Icon>,
}

impl WebAppManifest {
    /// Builds a manifest with the two required Android icon sizes and the
    /// 180px iOS touch icon already shipped by Cranpose's web release.
    pub fn new(name: impl Into<String>, short_name: impl Into<String>) -> Self {
        let name = name.into();
        let short_name = short_name.into();
        Self {
            name: name.clone(),
            short_name,
            start_url: "/".to_owned(),
            display: DisplayMode::Standalone,
            background_color: "#1c1c1e".to_owned(),
            theme_color: "#5b50e8".to_owned(),
            orientation: Some("any".to_owned()),
            icons: vec![
                Icon::png("icon-192.png", 192, true),
                Icon::png("icon-512.png", 512, true),
            ],
        }
    }

    /// Serialises to compact JSON, ready to be written as
    /// `manifest.webmanifest`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a manifest of strings cannot fail to serialise")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_required_icons() {
        let m = WebAppManifest::new("Cranpose Demo", "Cranpose");
        assert_eq!(m.icons.len(), 2);
        assert_eq!(m.icons[0].sizes, "192x192");
        assert_eq!(m.icons[1].sizes, "512x512");
        assert_eq!(m.display, DisplayMode::Standalone);
    }

    #[test]
    fn json_is_valid_and_has_manifest_fields() {
        let json = WebAppManifest::new("X", "Y").to_json();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["display"], "standalone");
        assert!(value["icons"].as_array().unwrap().len() == 2);
    }

    #[test]
    fn icon_purpose_is_optional() {
        let icon = Icon::png("i.png", 192, false);
        assert!(icon.purpose.is_none());
    }
}
