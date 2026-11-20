//! The web engine: the browser owns the clock and the pictures, so the player
//! decodes on the thread that draws.
//!
//! `wasm32-unknown-unknown` has no threads. The browser's media pipeline still
//! decodes in real time on its own, so the player keeps the decoder in a mutex
//! and reads from it inside [`VideoPlayer::take_frame`], where the UI already
//! is.

use std::{sync::Arc, time::Duration};

use parking_lot::Mutex;

use super::{Command, Shared, Slot, presentation_time};
use crate::{DecodedFrame, VideoDecoder, VideoError, VideoFrame};

/// The most pictures one `take_frame` decodes, so a tab that was hidden does
/// not spend its whole frame catching up on a video it cannot catch up with.
const BATCH: usize = 4;

struct Inline {
    decoder: Box<dyn VideoDecoder>,
    pending: Option<Waiting>,
    scratch: Vec<u8>,
    last_pts: Option<Duration>,
    adopted: bool,
    ended: bool,
    // A picture to read while the clock stands still: the one a seek lands on.
    refresh: bool,
}

/// A picture decoded before its time, which the slot cannot show yet.
struct Waiting {
    frame: DecodedFrame,
    pixels: Vec<u8>,
}

/// The decoder, kept for the thread that draws.
pub(crate) struct Engine {
    shared: Arc<Shared>,
    slot: Arc<Mutex<Slot>>,
    inline: Mutex<Inline>,
}

impl Engine {
    pub(crate) fn start(
        decoder: Box<dyn VideoDecoder>,
        shared: Arc<Shared>,
        slot: Arc<Mutex<Slot>>,
    ) -> Result<Self, VideoError> {
        Ok(Self {
            shared,
            slot,
            inline: Mutex::new(Inline {
                decoder,
                pending: None,
                scratch: Vec::new(),
                last_pts: None,
                adopted: false,
                ended: false,
                refresh: true,
            }),
        })
    }

    pub(crate) fn dispatch(&self, command: Command) -> Result<(), VideoError> {
        let mut inline = self.inline.lock();
        match command {
            Command::Play => {
                inline.ended = false;
                inline.decoder.play();
            }
            Command::Pause => inline.decoder.pause(),
            Command::SetSpeed(speed) => inline.decoder.set_speed(speed),
            Command::SetMuted(muted) => inline.decoder.set_muted(muted),
            Command::SetVolume(volume) => inline.decoder.set_volume(volume),
            Command::Stop => return self.rewind(&mut inline, Duration::ZERO),
            Command::Seek(position) => return self.rewind(&mut inline, position),
            Command::Shutdown => inline.decoder.close(),
        }
        Ok(())
    }

    /// Decodes the pictures whose time has come, and keeps the next one
    /// waiting.
    pub(crate) fn pump(&self) {
        let playing = self.shared.is_playing();
        let mut inline = self.inline.lock();
        if !playing {
            if !inline.refresh {
                return;
            }
            inline.refresh = false;
        }
        let budget = if playing { BATCH } else { 1 };
        for _ in 0..budget {
            if inline.ended {
                break;
            }
            if inline.pending.is_none() && !self.decode(&mut inline) {
                break;
            }
            let Some(pending) = inline.pending.take() else {
                break;
            };
            if pending.frame.pts > self.shared.position() {
                // Decoded ahead of its time: hold it for the next draw.
                inline.pending = Some(pending);
                break;
            }
            self.publish(&mut inline, pending);
        }
    }

    pub(crate) fn shutdown(&mut self) {
        let _ = self.dispatch(Command::Shutdown);
        let mut inline = self.inline.lock();
        inline.pending = None;
        inline.scratch = Vec::new();
    }

    fn rewind(&self, inline: &mut Inline, position: Duration) -> Result<(), VideoError> {
        inline.ended = false;
        inline.pending = None;
        inline.last_pts = None;
        inline.refresh = true;
        match inline.decoder.seek(position) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.shared.fail(error.clone());
                Err(error)
            }
        }
    }

    /// Returns false when the decoder has nothing to give right now.
    fn decode(&self, inline: &mut Inline) -> bool {
        if !inline.decoder.ready() {
            return false;
        }
        if !inline.decoder.has_frame() {
            return false;
        }
        let mut buffer = std::mem::take(&mut inline.scratch);
        if buffer.is_empty() {
            buffer = self.slot.lock().spare();
        }
        let frame = match inline.decoder.read(&mut buffer) {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                inline.scratch = buffer;
                inline.ended = true;
                self.shared.finish();
                return false;
            }
            Err(error) => {
                inline.scratch = buffer;
                self.shared.fail(error);
                return false;
            }
        };

        if buffer.len() != frame.width as usize * frame.height as usize * 4 {
            let length = buffer.len();
            inline.scratch = buffer;
            self.shared.fail(VideoError::Decode(format!(
                "the decoder filled {length} bytes for a {}x{} frame",
                frame.width, frame.height
            )));
            return false;
        }

        if !inline.adopted {
            inline.adopted = true;
            let info = inline.decoder.info();
            if info.width > 0 && info.height > 0 {
                self.shared.adopt_info(info);
            }
        }

        let pts = presentation_time(inline.last_pts, frame.pts, inline.decoder.frame_rate());
        inline.pending = Some(Waiting {
            frame: DecodedFrame { pts, ..frame },
            pixels: buffer,
        });
        true
    }

    fn publish(&self, inline: &mut Inline, pending: Waiting) {
        let frame = pending.frame;
        if let Some(published) =
            VideoFrame::from_rgba8(frame.width, frame.height, frame.pts, pending.pixels)
        {
            self.slot.lock().publish(published);
            self.shared.note_frame();
            self.shared.set_position_monotonic(frame.pts);
        } else {
            // A picture the frame constructor rejects must not disappear in
            // silence: that leaves a screen with no picture and a player that
            // claims nothing went wrong. Treat it like any other bad decode.
            self.shared.fail(VideoError::Decode(format!(
                "the decoder described an invalid {}x{} frame",
                frame.width, frame.height
            )));
        }
        inline.last_pts = Some(frame.pts);
    }
}
