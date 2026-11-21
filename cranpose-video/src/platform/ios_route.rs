//! Observes iOS audio output route changes and arms recovery on all players.
//!
//! Unplugging headphones or connecting Bluetooth can invalidate the audio
//! output stream. AVFoundation reports this through
//! `AVAudioSessionRouteChangeNotification`; a block observer forwards it to the
//! library, which re-attaches each player's audio source on its decode thread.
//! No player handle is needed on the UI side: the session registry in
//! `session.rs` holds weak handles to every live session.

#![expect(
    unsafe_code,
    reason = "observing an AVAudioSession notification crosses an FFI boundary"
)]

use std::ptr::NonNull;

use block2::DynBlock;
use objc2::runtime::AnyObject;
use objc2_avf_audio::AVAudioSessionRouteChangeNotification;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue};

pub(crate) fn install_route_observer() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Some(name) = (unsafe { AVAudioSessionRouteChangeNotification }) else {
            return;
        };
        let block = DynBlock::new(|_: NonNull<NSNotification>| {
            crate::session::PlaybackSession::route_changed_all();
        });
        unsafe {
            let _token = NSNotificationCenter::defaultCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(name),
                    None::<&AnyObject>,
                    None::<&NSOperationQueue>,
                    &block,
                );
            // The token must stay alive for the observer to remain registered.
            // Leaking it is intentional and bounded to one observer per process.
            std::mem::forget(_token);
        }
    });
}
