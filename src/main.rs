//! One window that plays a source, and the buttons that drive it.
//!
//! ```text
//! cargo run -- https://host/index.m3u8
//! cargo run -- C:\clips\a.mp4
//! ```
//!
//! With no argument it plays the clip the crate's tests ship, so a fresh
//! checkout shows a picture without a file of its own. The source may be a
//! file, a URL the platform decodes itself, or an HLS playlist this crate
//! joins and decodes.

use std::{env, error::Error, sync::OnceLock};

use cranpose::{AppLauncher, prelude::*};
use cranpose_video::VideoWithControls;

/// What the player opens when it is given no argument.
const DEFAULT_SOURCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/clip.mp4");

/// The source the window plays, set once before the window opens.
static SOURCE: OnceLock<String> = OnceLock::new();

fn main() -> Result<(), Box<dyn Error>> {
    let source = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_SOURCE.to_owned());
    let _ = SOURCE.set(source);
    AppLauncher::new()
        .with_title("Cranpose video")
        .with_size(960, 600)
        .try_run(Player)?;
    Ok(())
}

/// The screen: one video, filling the window.
#[composable]
fn Player() {
    VideoWithControls(
        SOURCE.get().cloned().unwrap_or_default(),
        Modifier::empty().fill_max_size(),
    );
}
