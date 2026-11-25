use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use crate::{PixelFormat, VideoError, VideoInfo, YuvColor};

/// One picture a decoder produced, and where in the media it belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedFrame {
    /// The frame's position in the media.
    pub pts: Duration,
    /// The frame's width in pixels.
    pub width: u32,
    /// The frame's height in pixels.
    pub height: u32,
}

/// A source of decoded pictures for [`VideoPlayer`](crate::VideoPlayer).
///
/// The crate implements this for every platform it ships a decoder on. An
/// application implements it to read a format the platform does not, or to
/// build on a decoder the crate does not carry. The player owns the thread that
/// calls, so an implementation never runs on the thread that draws, and it may
/// block until the next picture is ready.
///
/// `read` writes into a buffer the player owns and reuses, so a decoder copies
/// pixels instead of allocating a picture per frame.
pub trait VideoDecoder: Send {
    /// What the decoder learned when it opened the source.
    fn info(&self) -> VideoInfo;

    /// Moves to `position`. The next `read` returns the first picture at or
    /// after it, and the platform decides how close that is to the frame the
    /// caller asked for.
    ///
    /// Returns [`VideoError::UnsupportedOperation`] for a stream that only
    /// plays forward.
    fn seek(&mut self, position: Duration) -> Result<(), VideoError>;

    /// Decodes the next picture into `rgba` as top-down RGBA8 rows, resizing
    /// the buffer to `width * height * 4`.
    ///
    /// Returns `Ok(None)` at the end of the media. A player that receives
    /// `Ok(None)` stops reading until a seek or a restart.
    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError>;

    /// The explicit outcome of a read, which distinguishes a terminal end of
    /// media from a temporary stall or a decoder waiting for more input.
    ///
    /// Default: derives from `read()`, so existing backends keep their exact
    /// behaviour. A live or adaptive backend should override this to return
    /// `NeedMoreInput` / `NoPictureYet` instead of `EndOfStream` while it is
    /// still able to produce pictures.
    #[cfg(not(target_arch = "wasm32"))]
    fn read_outcome(
        &mut self,
        rgba: &mut Vec<u8>,
    ) -> Result<crate::decode_result::ReadOutcome, VideoError> {
        self.read(rgba)
            .map(crate::decode_result::ReadOutcome::from_legacy)
    }

    /// The pixel layout [`read`](Self::read) or [`read_nv12`](Self::read_nv12)
    /// fills.
    ///
    /// The player asks once, after opening, and calls the matching read for
    /// every picture. The default is RGBA8, so a decoder that produces its own
    /// pictures keeps its behaviour and pays nothing for the planar path.
    fn output_format(&self) -> PixelFormat {
        PixelFormat::Rgba8
    }

    /// Decodes the next picture into `nv12` as unpadded 4:2:0 planes.
    ///
    /// `nv12` receives `width * height` luma bytes followed by `width * height / 2`
    /// interleaved chroma bytes. A decoder whose platform hands it NV12
    /// overrides this and reports [`PixelFormat::Nv12`] from
    /// [`output_format`](Self::output_format): the picture is 1.5 bytes a pixel
    /// instead of 4, and the conversion then happens once across every core, or
    /// not at all when the screen samples the planes itself.
    ///
    /// A decoder that states [`PixelFormat::Nv12`] but leaves this at its
    /// default reports [`VideoError::UnsupportedOperation`], which is a
    /// distinct answer from the end of the media. The player treats it as a
    /// declaration it cannot honour: it turns the planar path off for that
    /// decoder and reads pictures through [`read`](Self::read) instead. The
    /// default used to report the end of the media, so the same decoder showed
    /// one black picture and then looked finished.
    fn read_nv12(&mut self, nv12: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        let _ = nv12;
        Err(VideoError::UnsupportedOperation {
            operation: "NV12 decoding",
        })
    }

    /// The colour space [`read_nv12`](Self::read_nv12) writes.
    ///
    /// Only a planar backend needs this. The default is BT.709 studio swing,
    /// which is what a high-definition track means when it states nothing.
    fn nv12_color(&self) -> YuvColor {
        YuvColor::default()
    }

    /// The frame rate the decoder actually produces, when the container states
    /// a rate that differs from the frames it carries.
    fn frame_rate(&self) -> Option<crate::FrameRate> {
        self.info().frame_rate
    }

    /// Whether a picture is ready to read now.
    ///
    /// The player's thread blocks in `read`, so a backend that can block leaves
    /// this at the default. It exists for the browser, where reading cannot
    /// block and the next picture arrives when the browser has drawn it.
    fn has_frame(&self) -> bool {
        true
    }

    /// Whether the backend is set up enough to read.
    ///
    /// The default is ready. The browser overrides it while the script that
    /// plays a stream it cannot play itself is still loading.
    fn ready(&self) -> bool {
        true
    }

    /// Whether the source keeps adding media as it plays.
    ///
    /// A live stream states no length and plays forward only. The player reads
    /// this once, when it opens the decoder, to refuse a seek it cannot honour.
    fn is_live(&self) -> bool {
        false
    }

    /// Whether the source can be seeked, which a live stream cannot.
    fn is_seekable(&self) -> bool {
        true
    }

    /// Transport hints for a backend whose platform owns the clock.
    ///
    /// The player keeps its own clock and states playback into it, so a backend
    /// that decodes on demand leaves these at their defaults. The browser plays
    /// the media itself, so it mirrors the player's state onto the element there:
    /// without it, a paused player would find the element a minute further on.
    fn play(&mut self) {}

    /// Stops the platform's own clock, as [`play`](Self::play) starts it.
    fn pause(&mut self) {}

    /// Plays at another rate on a backend that owns the clock. The player
    /// applies the rate to its own clock in every build.
    fn set_speed(&mut self, _speed: f32) {}

    /// Silences the audio of a backend that owns it.
    ///
    /// The default is a no-op. The native decoders carry pictures only, so
    /// sound is paired by the application there; a browser that plays the
    /// media itself overrides this because its element can carry sound.
    fn set_muted(&mut self, _muted: bool) {}

    /// Sets the volume of a backend that owns the audio, from 0.0 to 1.0.
    ///
    /// The default is a no-op; see [`set_muted`](Self::set_muted).
    fn set_volume(&mut self, _volume: f32) {}

    /// Assigns this player's audio group. Players in the same group are
    /// mutually exclusive: when a newer one starts, the older ones mute so a
    /// feed never plays two clips at once. Players in different groups keep
    /// their own sound and may play together.
    ///
    /// The default (group 0) gives the common single-active-player feed. An app
    /// that deliberately plays two clips with sound puts them in distinct
    /// groups. The default is a no-op for decoders without this notion.
    fn set_audio_group(&mut self, _group: u64) {}

    /// Selects an audio track by the backend's own index, or turns sound off
    /// with `None`.
    ///
    /// The default ignores it, which is right for a decoder that carries one
    /// track. A backend with a demuxer overrides this and reports the tracks it
    /// found through [`media`](Self::media).
    fn set_audio_track(&mut self, _index: Option<usize>) {}

    /// Selects a subtitle track by the backend's own index, or turns subtitles
    /// off with `None`.
    ///
    /// The default ignores it. A backend that renders or exposes timed text
    /// overrides it.
    fn set_subtitle_track(&mut self, _index: Option<usize>) {}

    /// Repeats only `range` instead of the whole source, or the whole source
    /// again with `None`.
    ///
    /// The player already rewinds itself when the play head passes the range's
    /// end, so a backend only needs this when its own demuxer can loop more
    /// cheaply. The default ignores it.
    fn set_loop_range(&mut self, _range: Option<crate::loop_range::LoopRange>) {}

    /// How far the backend has filled its window, for a spinner or a ring.
    ///
    /// The default reports idle, which is right for a decoder that reads a file
    /// as fast as the player asks. A network backend overrides this so a screen
    /// can show progress instead of an endless spinner.
    fn buffering(&self) -> crate::buffer::Buffering {
        crate::buffer::Buffering::new()
    }

    /// What the backend learned about the container, its codecs and its tracks.
    ///
    /// The default states nothing, which is right for a decoder that reads one
    /// elementary stream. A backend with a demuxer overrides this so an about
    /// screen and a track menu have something to show.
    fn media(&self) -> crate::media::MediaInfo {
        crate::media::MediaInfo::default()
    }

    /// Whether the backend currently plays no sound.
    ///
    /// The player reads this once when it opens, so a browser that starts an
    /// element muted to allow autoplay states that honestly. The default is
    /// unmuted, which is right for a decoder with no audio of its own.
    fn is_muted(&self) -> bool {
        false
    }

    /// The volume the backend plays at, from 0.0 to 1.0.
    fn volume(&self) -> f32 {
        1.0
    }

    /// Whether this decoder decodes and plays the source's audio itself.
    ///
    /// The default is false: a decoder that only produces pictures needs the
    /// application to pair sound from the same source (the crate's
    /// `native-audio` path does that on most targets). A backend that reads both
    /// streams from one source on one shared clock overrides this to `true` so
    /// the paired path steps aside and cannot play the same sound twice. This is
    /// an add-only hook; existing decoders keep their behaviour unchanged.
    fn carries_audio(&self) -> bool {
        false
    }

    /// Tells a decoder that owns the platform clock which media time the player
    /// just published for a picture.
    ///
    /// The player re-anchors its own clock at every frame, so a backend that
    /// renders sound on another thread uses this to keep its play head on the
    /// same timeline. The default is a no-op; decoders that let the player own
    /// the clock need nothing.
    fn sync_clock(&mut self, _position: Duration) {}

    /// Releases the platform objects this decoder holds.
    ///
    /// The player calls this once, before the decoder is dropped, so a backend
    /// that must tear down in a particular order can do it on the decode
    /// thread. Dropping is enough for a backend that keeps no such order.
    fn close(&mut self) {}

    /// Hands a decoder that can block in [`read`](Self::read) a one-way signal
    /// that the player is closing.
    ///
    /// A decoder that fetches media from a network checks the flag inside its
    /// blocking reads so a stalled request cannot hold up
    /// [`VideoPlayer::close`](crate::VideoPlayer::close). The default is to
    /// ignore it, which is right for a decoder whose reads return promptly.
    /// This is an add-only hook; existing decoders keep their behaviour.
    fn observe_shutdown(&mut self, _flag: Arc<AtomicBool>) {}

    /// Hands the decoder the player's playback session.
    ///
    /// A decoder that owns sound, or that can lose it through a device or route
    /// change, reports failures into this handle so the player's recovery policy
    /// can bring the sound back. The default ignores it, which is right for a
    /// decoder with no audio of its own. This is an add-only hook; existing
    /// decoders keep their behaviour.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn attach_session(&mut self, _session: crate::PlaybackSession) {}

    /// Re-establishes sound after a failure reported to the session.
    ///
    /// The player calls this on its decode thread when the recovery cooldown
    /// has elapsed, so no device query or stream rebuild ever runs on the
    /// composition thread. Return `true` when sound is usable again and `false`
    /// when another attempt should be scheduled. The default reports success:
    /// a decoder with nothing to restore needs no recovery.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn recover(&mut self) -> bool {
        true
    }
}
