//! A Reels-style native window that exercises `cranpose-video`.
//!
//! The window has four sections: an offline vertical pager where only the
//! settled page opens a decoder, a separate Live pager over the opt-in Free-TV
//! IPTV channels, a full clip library, and an in-app paced decode test console.
//!
//! ```sh
//! cargo run --release
//! cargo run --release -- assets/videos/sintel.mp4
//! ```

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    // Android enters through the `android_main!` macro in lib.rs; the shared
    // `run()` helper is compiled out there.
    #[cfg(not(target_os = "android"))]
    {
        counter::run()
    }
    #[cfg(target_os = "android")]
    {
        Ok(())
    }
}
