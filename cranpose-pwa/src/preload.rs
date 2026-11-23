//! Generation of `<link rel="preload">` / `<link rel="modulepreload">` tags.
//!
//! Preloading the WASM bundle at parse time removes a full network round trip
//! from first paint — critical for heavy Cranpose apps on mobile.

/// What a preload link fetches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsKind {
    /// A WebAssembly module (needs `type="application/wasm"`).
    Fetch,
    /// An ES module.
    Script,
    /// A stylesheet.
    Style,
    /// An image.
    Image,
}

impl AsKind {
    fn as_attr(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Script => "script",
            Self::Style => "style",
            Self::Image => "image",
        }
    }
    fn type_attr(self) -> Option<&'static str> {
        match self {
            Self::Fetch => Some("application/wasm"),
            Self::Script => Some("text/javascript"),
            Self::Style => Some("text/css"),
            Self::Image => None,
        }
    }
}

/// Builds a `<link rel="preload">` tag for an asset.
pub fn preload_tag(url: &str, kind: AsKind, crossorigin: bool) -> String {
    let mut tag = format!(
        "<link rel=\"preload\" href=\"{url}\" as=\"{}\"",
        kind.as_attr()
    );
    if let Some(t) = kind.type_attr() {
        tag.push_str(&format!(" type=\"{t}\""));
    }
    if crossorigin {
        tag.push_str(" crossorigin");
    }
    tag.push('>');
    tag
}

/// Builds a `<link rel="modulepreload">` tag for an ES module.
pub fn modulepreload_tag(url: &str) -> String {
    format!("<link rel=\"modulepreload\" href=\"{url}\">")
}

/// The full, ready-to-paste head bundle for a Cranpose WASM app.
pub fn wasm_head_bundle(wasm_url: &str, js_url: &str) -> String {
    format!(
        "{}\n{}",
        modulepreload_tag(js_url),
        preload_tag(wasm_url, AsKind::Fetch, true)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_preload_has_type() {
        let tag = preload_tag("/app.wasm", AsKind::Fetch, true);
        assert!(tag.contains("application/wasm"));
        assert!(tag.contains("crossorigin"));
        assert!(tag.contains("rel=\"preload\""));
    }

    #[test]
    fn modulepreload_is_correct() {
        assert_eq!(
            modulepreload_tag("/app.js"),
            "<link rel=\"modulepreload\" href=\"/app.js\">"
        );
    }

    #[test]
    fn head_bundle_has_both() {
        let bundle = wasm_head_bundle("/app_bg.wasm", "/app.js");
        assert!(bundle.contains("modulepreload"));
        assert!(bundle.contains("app_bg.wasm"));
    }
}
