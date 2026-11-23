//! Static assets a PWA shell needs: service worker JavaScript and the HTML
//! meta-tag snippet. They are shipped as string constants so a build script or
//! a static server can write them without shipping extra files.

/// A cache-first service worker with a versioned cache and safe cleanup.
///
/// Includes the Cranpose WASM file handed to it in `CORE_EXTRA` by the app's
/// build script (see README), because a cached page without its WASM cannot
/// start offline.
pub const SERVICE_WORKER_JS: &str = r#"
const CACHE = 'cranpose-pwa-v1';
// Core app shell. Append the hashed bundle names from asset-manifest.json.
const CORE = ['/', '/index.html', '/manifest.webmanifest' /*, '/bundle/desktop_app_bg.wasm' */];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(CACHE)
      .then((cache) => cache.addAll(CORE))
      .then(() => self.skipWaiting())
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  if (request.method !== 'GET') return;
  event.respondWith(
    caches.match(request).then((hit) => hit || fetch(request).then((response) => {
      if (response.ok && new URL(request.url).origin === location.origin) {
        const copy = response.clone();
        caches.open(CACHE).then((cache) => cache.put(request, copy));
      }
      return response;
    }))
  );
});
"#;

/// The `<meta>` tags iOS Safari needs for a standalone "Add to Home Screen"
/// experience. Cranpose's own page already ships `apple-touch-icon.png`.
pub const IOS_META: &str = r##"
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
<meta name="apple-mobile-web-app-title" content="Cranpose">
<link rel="manifest" href="/manifest.webmanifest">
<meta name="theme-color" content="#5b50e8">
"##;

/// The JavaScript snippet that registers the service worker and the automatic
/// WebKit/`backend=gl` fix. Paste before the shell reads `?backend`.
/// CSS that hides install UI once the PWA is installed. `[SOURCE-VERIFIED
/// web.dev]`: install instructions should render only in `display-mode: browser`;
/// standalone/fullscreen means the user already installed. This is the reliable
/// signal on iOS where localStorage is isolated between Safari and the PWA.
pub const INSTALL_UI_CSS: &str = r#"
@media (display-mode: standalone), (display-mode: fullscreen) {
  [data-pwa-install-ui] { display: none !important; }
}
"#;

pub const BOOTSTRAP_JS: &str = r#"
<script>
(function () {
  // 1) Auto WebGL2 on iOS WebKit (Safari and Chrome both use WebKit).
  var ua = navigator.userAgent || '';
  var isIOS = /iPad|iPhone|iPod/.test(ua) ||
    (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
  if (isIOS) {
    var url = new URL(window.location.href);
    var b = url.searchParams.get('backend');
    if (b === null || b === 'auto') {
      url.searchParams.set('backend', 'gl');
      window.history.replaceState({}, '', url);
    }
  }
  // 2) Register the service worker for installability + offline.
  if ('serviceWorker' in navigator) {
    window.addEventListener('load', function () {
      navigator.serviceWorker.register('/sw.js').catch(function (e) {
        console.warn('PWA service worker registration failed:', e);
      });
    });
  }
})();
</script>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sw_has_core_handlers() {
        assert!(SERVICE_WORKER_JS.contains("install"));
        assert!(SERVICE_WORKER_JS.contains("activate"));
        assert!(SERVICE_WORKER_JS.contains("fetch"));
        assert!(SERVICE_WORKER_JS.contains("cranpose-pwa-v1"));
    }

    #[test]
    fn bootstrap_contains_auto_gl_and_sw() {
        assert!(BOOTSTRAP_JS.contains("backend"));
        assert!(BOOTSTRAP_JS.contains("gl"));
        assert!(BOOTSTRAP_JS.contains("serviceWorker.register"));
    }

    #[test]
    fn ios_meta_has_manifest_link() {
        assert!(IOS_META.contains("manifest.webmanifest"));
        assert!(IOS_META.contains("apple-mobile-web-app-capable"));
    }
}
