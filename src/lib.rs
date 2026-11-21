//! Shared application library for every target (desktop, iOS, Android).
//!
//! The UI lives in `app_ui` and is identical on all platforms. The desktop
//! binary (`main.rs`) and the mobile entry points call [`launch`] so both
//! targets run the same composition. This crate also declares the Android
//! entry point; the macro expands to nothing on other targets.

pub mod app_diag;
pub mod app_model;
pub mod app_ui;

use cranpose::prelude::*;

/// The clip selected when the run names none. Kept compatible with the
/// original one-clip harness; the Reels section maps it to a starting page.
const DEFAULT_CLIP: &str = "assets/videos/bunny.mp4";

/// Initializes diagnostics and the library's diagnostic sink.
fn init_diagnostics() {
    app_diag::init();
    // Forward the library's deliberately swallowed audio/decoder failures into
    // the same file. The callback is thread-safe and must stay quick: it runs
    // on the decode thread, not on the UI thread.
    cranpose_video::set_diagnostic_sink(Some(Box::new(|level, target, message| {
        app_diag::log(level, target, message);
    })));
}

/// Builds the launcher shared by desktop and mobile. Mobile targets use the
/// native screen size; the desktop window keeps its explicit size.
pub fn launch() -> AppLauncher {
    let clip = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from(DEFAULT_CLIP));
    let _ = app_ui::INITIAL_CLIP.set(clip);
    let launcher = AppLauncher::new().with_title("cranpose-video · Reels Lab");
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let launcher = launcher.with_size(960, 720);
    launcher
}

/// Runs the application. The desktop `main.rs` uses this on desktop and iOS.
///
/// On Android the `android_main!` macro below is the entry point and
/// `AppLauncher` exposes `run(app, content)` (not `try_run`) there, so this
/// helper is compiled out for that target.
#[cfg(not(target_os = "android"))]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_diagnostics();
    let result = launch().try_run(app_ui::AppRoot);
    match &result {
        Ok(()) => app_diag::info("main", "window closed normally"),
        Err(error) => app_diag::error("main", format!("try_run failed: {error}")),
    }
    result?;
    Ok(())
}

/// iOS entry point. The desktop `main.rs` also serves as the iOS entry in the
/// Cranpose guide; this helper is provided so the library can host the same
/// composition when a host calls into it.
#[cfg(target_os = "ios")]
pub fn run_ios() {
    init_diagnostics();
    let _ = launch().try_run(app_ui::AppRoot);
}

// Android native activity entry point, expanded by Cranpose. Off Android the
// macro expands to nothing, so this crate builds normally on desktop.
cranpose::android_main! {
    launcher: {
        init_diagnostics();
        launch()
    },
    content: app_ui::AppRoot,
}
