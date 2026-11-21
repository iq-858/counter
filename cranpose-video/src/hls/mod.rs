//! HLS: a playlist of media fragments, played through the platform decoder.
//!
//! No platform media stack opens an HLS address on its own: Media Foundation
//! refuses the byte stream, and AVFoundation reads the playlist but not the
//! fragmented MP4 a live stream names. So this module fetches the playlist and
//! the fragments itself, joins a window of them into one fragmented MP4, and
//! hands that file to the platform decoder the crate already carries.
//!
//! A fragment states its own time, counted from wherever the stream was cut.
//! Each window is moved back so that its first fragment starts at zero, and the
//! player's clock starts at the position in the stream the window holds. The
//! first fragmented-MP4 window is one segment long so decoding can start
//! sooner. Later windows hold two segments, long enough that the decoder and
//! the network keep ahead of the clock, short enough that opening the next one
//! is cheap. MPEG-TS keeps two segments in its first window because it has no
//! separate initialization section.

mod playlist;
mod tfdt;

use std::{
    collections::HashMap,
    io::{self, Read},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
use crate::PlaybackSession;
use crate::{
    DecodedFrame, PixelFormat, VideoDecoder, VideoError, VideoInfo, VideoSource, YuvColor, diag,
    diagnostics::redact_for_report,
    hls::playlist::{
        AudioRendition, Media, Range, ResourceKind, Segment, Variant, accepts_content_type,
    },
    temp_file::TempFile,
};

/// How many segments a window holds. The same count is used from the first
/// window on: shrinking only the first one would trade a smaller startup for
/// more rollovers, so it is deliberately not done.
const WINDOW: usize = 2;

/// How far behind the live edge a live stream starts, in segments.
const LIVE_BACK: usize = 3;

/// The shortest wait between live playlist polls. A target shorter than this
/// would make the reader poll faster than the network can carry new segments.
const POLL_MIN: Duration = Duration::from_millis(500);

/// The longest wait between live playlist polls while segments keep arriving.
const POLL_MAX: Duration = Duration::from_secs(3);

/// The longest wait when the playlist has not moved, so a stream that stopped
/// adding segments is not hammered.
const WAIT_MAX: Duration = Duration::from_secs(4);

/// The shortest time a live stream may go without a new segment before the
/// read fails, even when its target duration is tiny.
const STALL_MIN: Duration = Duration::from_secs(15);

/// The longest time a live stream may go without a new segment before the
/// read fails, even when its target duration is long.
const STALL_MAX: Duration = Duration::from_secs(45);

/// The most a playlist may be. Playlists are text, usually a few kilobytes.
const MAX_PLAYLIST: u64 = 16 * 1024 * 1024;

/// The most one media fragment may be. Even a long 4K fragment stays far
/// below this; the ceiling stops a wrong address from exhausting memory.
const MAX_MEDIA: u64 = 256 * 1024 * 1024;

/// The most a body is reserved up front from a stated length. A server that
/// overstates the length must not make the reader allocate it all at once.
const MAX_RESERVE: usize = 8 * 1024 * 1024;

/// How long a playlist request may take.
const PLAYLIST_DEADLINE: Duration = Duration::from_secs(20);

/// The least time one media fragment request may take, even for a short one on
/// a slow link. A fragment is fetched over a live connection, so it is allowed
/// longer than a playlist.
const MEDIA_DEADLINE_MIN: Duration = Duration::from_secs(20);

/// The most time one media fragment request may take, however long the segment
/// states itself to be. This is a safety bound, not a content limit: a request
/// that never answers must not hold a close indefinitely.
const MEDIA_DEADLINE_MAX: Duration = Duration::from_secs(120);

/// How many times one request is tried before its failure is reported.
const ATTEMPTS: usize = 3;

/// How many fragments of one window are fetched at once. A window is short, so
/// this is a small bound; it lets the segments after the first arrive in
/// parallel without opening an unbounded number of connections to the host.
const PARALLEL_FETCH: usize = 4;

/// The longest a `Retry-After` is honoured, even when a server asks for more.
const RETRY_CAP: Duration = Duration::from_secs(5);

/// The share each new measurement takes in the running estimate of the link.
///
/// One slow fragment, from a lost packet or a host under load, moves the
/// estimate part way rather than all the way, so a rendition switch follows a
/// sustained change instead of a single bad request.
const BANDWIDTH_WEIGHT: f64 = 0.4;

/// The tallest rendition a master playlist may offer and the crate still picks.
///
/// There is no fixed ceiling: the decoder reads the real resolution and frame
/// rate from the media itself, so an 8K, 4K or any other rendition a playlist
/// names is eligible. The tallest one available is chosen, which is the one the
/// stream was authored to look best at.
const MAX_HEIGHT: u32 = u32::MAX;

/// Opens an HLS address by joining its fragments for the platform decoder.
///
/// This only remembers the address. The playlist, its fragments and the first
/// window are opened on the first read or seek, which runs on the player's
/// decode thread: neither the composition thread nor `info` touches the network.
pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn VideoDecoder>, VideoError> {
    let Some(url) = source.as_url() else {
        return Err(VideoError::UnsupportedOperation {
            operation: "opening an HLS source that is not a URL",
        });
    };
    Ok(Box::new(Stream::new(url)))
}

/// One window of segments, open in the platform decoder.
struct Window {
    /// The decoder holds the file open, so it must go before the file does.
    decoder: Box<dyn VideoDecoder>,
    /// The joined segments, removed when the window goes. Nothing reads the
    /// bytes again; the field exists so the file outlives the decoder that
    /// opened it.
    #[allow(dead_code)]
    file: TempFile,
    /// The joined audio fragments when audio lives in a separate playlist.
    /// Kept alive alongside `file` so the backend can read it, and dropped
    /// only after the audio has been detached.
    #[cfg(feature = "native-audio")]
    #[allow(dead_code)]
    audio_file: Option<TempFile>,
    /// How many segments the window holds, counted from the index it was
    /// opened at.
    count: usize,
    /// The position in the stream the window starts at.
    start: Duration,
    /// The first presentation time the decoder reports, which is subtracted so
    /// the window starts at `start` however the decoder numbers its pictures.
    base: Option<Duration>,
}

/// Identifies one fragment in the prefetch cache.
///
/// Keyed by address rather than playlist index: a live playlist slides its
/// segments and renumbers them, but the address of a fragment that is still to
/// play stays the same, so bytes fetched for the upcoming window remain valid
/// across a refresh.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FragmentKey {
    uri: String,
    range: Option<Range>,
}

impl FragmentKey {
    fn new(segment: &Segment) -> Self {
        Self {
            uri: segment.uri.clone(),
            range: segment.range,
        }
    }
}

/// Fragments fetched ahead of the player, shared with the short-lived prefetch
/// thread.
struct Prefetch {
    /// Bumped whenever the cache is invalidated (a seek, a new window, a close).
    /// An in-flight fetch only stores its result when its generation still
    /// matches, so bytes for an abandoned window cannot repopulate the cache.
    generation: u64,
    /// The raw fragment bytes, before timestamp rebasing. Rebasing is applied
    /// afresh when the bytes are consumed, so caching the raw form is safe.
    fragments: HashMap<FragmentKey, Vec<u8>>,
    /// What the network delivered, measured on every fragment. The decode
    /// thread and the background worker both record here, and the rendition
    /// choice reads it, so the estimate follows the whole stream rather than
    /// only the requests the decode thread happened to make.
    bandwidth: Bandwidth,
}

/// A running estimate of the download throughput, in bits per second.
#[derive(Debug, Default)]
struct Bandwidth {
    bits_per_second: Option<f64>,
}

impl Bandwidth {
    /// Adds one completed window's slowest transfer.
    fn record(&mut self, bits_per_second: f64) {
        if !bits_per_second.is_finite() || bits_per_second <= 0.0 {
            return;
        }
        self.bits_per_second = Some(match self.bits_per_second {
            Some(previous) => previous + BANDWIDTH_WEIGHT * (bits_per_second - previous),
            None => bits_per_second,
        });
    }

    /// The measured rate, once a transfer has been timed.
    fn estimate(&self) -> Option<f64> {
        self.bits_per_second
    }
}

/// The renditions a master playlist offered, so the stream can move to another
/// one when the link cannot carry the one it plays.
struct Master {
    /// The picture renditions, in the order the playlist stated them.
    variants: Vec<Variant>,
    /// The audio renditions offered apart from the picture.
    audios: Vec<AudioRendition>,
    /// The place of the rendition in play.
    chosen: usize,
}

/// What reading a playlist settled on.
struct Opened {
    /// The media playlist to play.
    url: String,
    /// Its segments, read once.
    media: Media,
    /// The separate audio playlist the master named for the rendition, when it
    /// named one.
    audio_url: Option<String>,
    /// The renditions the stream may move between, absent for a media playlist
    /// that states segments itself.
    master: Option<Master>,
}

/// A stream played from HLS by fetching its fragments.
struct Stream {
    /// The address the application opened. `choose` replaces `url` with the
    /// chosen rendition, but a recovery must start from this original address
    /// so an expired signed playlist can be re-resolved into fresh fragment URLs.
    source_url: String,
    /// The media playlist, after a master playlist has chosen a rendition.
    url: String,
    /// Whether the address has been read and a rendition chosen. Opening the
    /// player only stores the address; the first decode-thread read loads it.
    loaded: bool,
    /// The renditions the master playlist offered, absent when the address
    /// named one media playlist. A stream with them measures its link and
    /// moves to the rendition that link can carry.
    master: Option<Master>,
    /// The playlist as it was last read.
    media: Media,
    /// Fragments for the upcoming window, fetched in the background while the
    /// current window plays so a rollover does not stall on the network.
    prefetch: Arc<Mutex<Prefetch>>,
    /// The open window of segments, when there is one.
    window: Option<Window>,
    /// The index in `media` the next window starts at.
    next: usize,
    /// Whether a window has opened successfully yet. The initial fragmented
    /// MP4 window can start with one segment; later windows keep the normal
    /// two-segment buffer.
    first_window: bool,
    /// What the player's clock reads at the start of the next window. A live
    /// playlist states no fixed origin, so the stream counts from zero and
    /// keeps adding the segments it plays.
    elapsed: Duration,
    /// Whether the stream is still adding segments.
    live: bool,
    /// Whether the playlist states a fixed length, which is what a seek needs.
    seekable: bool,
    /// When the playlist was last fetched.
    polled: Instant,
    /// The initialization section last fetched for the picture, with the
    /// address and range it came from. It rarely changes, so keeping it saves a
    /// request on every live rollover.
    init: Option<(String, Option<Range>, Vec<u8>)>,
    /// What the decoder states about the window that is open, with the length
    /// of the whole playlist in place of the window's.
    info: VideoInfo,
    /// Whether the player is running. A window opened later starts its audio
    /// at the same instant as its pictures.
    #[cfg(feature = "native-audio")]
    playing: bool,
    /// The rate the player runs at, so a window opened later starts its decoder
    /// owned audio at the same rate.
    #[cfg(feature = "native-audio")]
    speed: f32,
    /// Whether sound is silenced, so a window opened later stays silenced.
    #[cfg(feature = "native-audio")]
    muted: bool,
    /// The sound level, so a window opened later keeps the same level.
    #[cfg(feature = "native-audio")]
    volume: f32,
    /// Best-effort audio for the window that is open, when this build carries
    /// one. It reads the joined file the window prepared for it, which is the
    /// picture's own file when audio is muxed into it.
    #[cfg(feature = "native-audio")]
    audio: Option<crate::audio::Audio>,
    /// The separate audio playlist a master playlist named, when there is one.
    #[cfg(feature = "native-audio")]
    audio_url: Option<String>,
    /// The separate audio playlist as it was last read.
    #[cfg(feature = "native-audio")]
    audio_media: Option<Media>,
    /// When the separate audio playlist was last fetched.
    #[cfg(feature = "native-audio")]
    audio_polled: Instant,
    /// The initialization section last fetched for the separate audio, kept
    /// the same way as the picture's.
    #[cfg(feature = "native-audio")]
    audio_init: Option<(String, Option<Range>, Vec<u8>)>,
    /// Where a seek put the stream, until the next window's audio has moved to
    /// match it. The video window starts at a segment boundary; the sound can
    /// start exactly where the seek landed.
    #[cfg(feature = "native-audio")]
    seeked_to: Option<Duration>,
    /// The one-way close signal shared with every blocking fetch and retry
    /// wait. The player's flag is observed when it is attached; a local flag
    /// lets a direct `close()` stop a background prefetch as well.
    stop: Stop,
    /// The player's playback session, when this build and player carry one.
    /// HLS failures are reported here instead of duplicating the generic
    /// audio layer's reports for its local TempFile source.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    session: Option<PlaybackSession>,
    /// The session generation at which an HLS failure was last reported, and
    /// the stage it named. A generation changes when the worker begins a new
    /// recovery attempt, so one failure is reported at most once per attempt.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    reported_failure: Option<(u64, &'static str)>,
    /// Set when the address answered with a permanent status (403/410). Such a
    /// source will not recover by re-reading the playlist, so recovery must stop
    /// instead of reloading it forever and flooding the log.
    permanent_failure: bool,
}

impl Stream {
    /// Remembers the address without touching the network.
    ///
    /// The player asks for `info` and the output format before its decode
    /// thread starts, so neither can depend on a playlist fetch. Everything
    /// that reads bytes happens in `ensure_loaded` on the first decode read.
    fn new(url: &str) -> Self {
        // Build the pre-load placeholder directly, never by parsing. A playlist
        // with no segments is invalid input and `playlist::media` rejects it, so
        // parsing a fake segment-less playlist here panicked on the composition
        // thread before the decode thread could load the real one. Every reader
        // tolerates the empty segment list until `ensure_loaded` replaces it.
        let media = playlist::Media::empty();
        Self {
            source_url: url.to_owned(),
            url: url.to_owned(),
            loaded: false,
            master: None,
            media,
            prefetch: Arc::new(Mutex::new(Prefetch {
                generation: 0,
                fragments: HashMap::new(),
                bandwidth: Bandwidth::default(),
            })),
            window: None,
            next: 0,
            first_window: true,
            elapsed: Duration::ZERO,
            live: false,
            // Optimistic until the playlist is read: a fixed-length stream is
            // seekable, and a live one refuses the seek after `ensure_loaded`.
            seekable: true,
            polled: Instant::now(),
            init: None,
            info: VideoInfo {
                width: 0,
                height: 0,
                duration: None,
                frame_rate: None,
                rotation: Default::default(),
            },
            #[cfg(feature = "native-audio")]
            playing: false,
            #[cfg(feature = "native-audio")]
            speed: 1.0,
            #[cfg(feature = "native-audio")]
            muted: false,
            #[cfg(feature = "native-audio")]
            volume: 1.0,
            #[cfg(feature = "native-audio")]
            audio: Some(crate::audio::Audio::new()),
            #[cfg(feature = "native-audio")]
            audio_url: None,
            #[cfg(feature = "native-audio")]
            audio_media: None,
            #[cfg(feature = "native-audio")]
            audio_polled: Instant::now(),
            #[cfg(feature = "native-audio")]
            audio_init: None,
            #[cfg(feature = "native-audio")]
            seeked_to: None,
            stop: Stop::default(),
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            session: None,
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            reported_failure: None,
            permanent_failure: false,
        }
    }

    /// Loads the playlist on first use, on the decode thread.
    ///
    /// A separate audio playlist is best effort: a stream that serves it badly
    /// still plays its pictures in silence. Its failures are intentionally not
    /// reported as HLS failures, matching the generic audio layer's ownership
    /// of its own local source.
    fn ensure_loaded(&mut self) -> Result<(), VideoError> {
        if self.loaded {
            return Ok(());
        }
        let began = Instant::now();
        let Opened {
            url,
            media,
            audio_url,
            master,
        } = Self::choose(&self.url, &self.stop)?;
        let live = !media.ended;
        let next = if live {
            media.segments.len().saturating_sub(LIVE_BACK)
        } else {
            0
        };
        self.url = url;
        self.master = master;
        self.media = media;
        self.next = next;
        self.live = live;
        self.seekable = !live;
        self.polled = Instant::now();
        self.loaded = true;
        #[cfg(feature = "native-audio")]
        {
            // A reload must not keep the previous rendition's separate audio
            // playlist or init section: their fragment addresses belong to the
            // old playlist and would join the wrong sound into the new window.
            self.audio_url = audio_url;
            self.audio_media = None;
            self.audio_init = None;
            self.audio_polled = Instant::now();
            if let Some(audio_url) = self.audio_url.clone() {
                if let Ok(text) = text_inner(&audio_url, &self.stop) {
                    if let Ok(media) = playlist::media(&text, &audio_url) {
                        self.audio_media = Some(media);
                    }
                }
            }
        }
        #[cfg(not(feature = "native-audio"))]
        let _ = audio_url;
        // What the playlist settled on, so a log states which rendition was
        // chosen and whether its sound lives in a playlist of its own.
        #[cfg(feature = "native-audio")]
        let separate_audio = self.audio_url.is_some();
        #[cfg(not(feature = "native-audio"))]
        let separate_audio = false;
        crate::diag(
            "INFO",
            "hls.open",
            &format!(
                "playlist {} holds {} segment(s): live={live} separate_audio={separate_audio} start_index={next}, chosen in {:?}",
                self.url,
                self.media.segments.len(),
                began.elapsed()
            ),
        );
        Ok(())
    }

    /// The media playlist, following a master playlist to the rendition to play.
    fn choose(url: &str, stop: &Stop) -> Result<Opened, VideoError> {
        let page = text(url, stop)?;
        if !playlist::is_master(&page) {
            return Ok(Opened {
                media: playlist::media(&page, url)?,
                url: url.to_owned(),
                audio_url: None,
                master: None,
            });
        }
        let variants = playlist::variants(&page, url)?;
        let audios = playlist::audios(&page, url);
        let chosen = playlist::chosen_index(&variants, MAX_HEIGHT).ok_or_else(|| {
            VideoError::Source("the HLS master playlist offers no rendition".to_owned())
        })?;
        let audio_url = playlist::choose_audio(&audios, &variants[chosen])
            .map(|rendition| rendition.uri.clone());
        let variant_url = variants[chosen].uri.clone();
        let body = text(&variant_url, stop)?;
        let media = playlist::media(&body, &variant_url)?;
        Ok(Opened {
            url: variant_url,
            media,
            audio_url,
            master: Some(Master {
                variants,
                audios,
                chosen,
            }),
        })
    }

    /// Fetches a playlist again and lines the next segment up with it.
    ///
    /// A live playlist drops the segments that are too old to play and numbers
    /// the rest, so the index the stream holds is only good for the playlist it
    /// was read from. The sequence number, which keeps counting, is what ties
    /// the two together.
    fn refresh(&mut self, force: bool) -> Result<(), VideoError> {
        self.refresh_inner(force).map_err(|failure| failure.error)
    }

    /// The same fetch, keeping whether the failure may pass.
    fn refresh_inner(&mut self, force: bool) -> Result<(), Failure> {
        if !force && self.polled.elapsed() < self.poll_interval() {
            return Ok(());
        }
        let body = text_inner(&self.url, &self.stop)?;
        let media = playlist::media(&body, &self.url).map_err(Failure::permanent)?;
        let wanted = self.media.sequence.saturating_add(self.next as u64);
        self.media = media;
        self.polled = Instant::now();
        let index = wanted.saturating_sub(self.media.sequence);
        self.next = usize::try_from(index)
            .unwrap_or(usize::MAX)
            .min(self.media.segments.len());
        Ok(())
    }

    /// How often the live playlist is asked for new segments. A stream that
    /// states long segments is asked less often than one with short ones.
    fn poll_interval(&self) -> Duration {
        playlist_poll(self.media.target)
    }

    /// How long the live playlist may stay unchanged before the stream fails.
    fn stall_timeout(&self) -> Duration {
        playlist_stall(self.media.target)
    }

    /// Whether the player has signalled a close. Checked between the blocking
    /// steps of a fragment read so closing does not wait on the network.
    fn shutting_down(&self) -> bool {
        self.stop.is_stopped()
    }

    /// Starts fragmented MP4 from the smallest complete window. MPEG-TS has
    /// no init section and keeps its existing two-segment startup buffer.
    fn window_size(&self) -> usize {
        if self.first_window && self.media.map.is_some() {
            1
        } else {
            WINDOW
        }
    }

    /// Opens the window at `next`: the init section, then up to `segments`
    /// fragments, with every fragment moved back to the window's first time.
    fn open_window(&mut self) -> Result<(), VideoError> {
        let began = Instant::now();
        if self.live {
            self.refresh(false)?;
        }
        let segments = self.window_size();
        if self.next >= self.media.segments.len() {
            return Err(VideoError::Source(
                "the HLS playlist names no segment left to play".to_owned(),
            ));
        }
        let end = (self.next + segments.max(1)).min(self.media.segments.len());
        let count = end - self.next;
        let start = if self.live {
            self.elapsed
        } else {
            self.media.start(self.next)
        };

        let mut bytes = Vec::new();
        if let Some(map) = self.media.map.clone() {
            match init_bytes(&mut self.init, &map, &self.stop) {
                Ok(init) => bytes.extend_from_slice(&init),
                Err(error) => {
                    diag(
                        "ERROR",
                        "hls.stage",
                        &format!(
                            "init section failed at index {} ({}/{} segments, live={}): {}",
                            self.next,
                            count,
                            self.media.segments.len(),
                            self.live,
                            error
                        ),
                    );
                    return Err(error);
                }
            }
        }
        // Fetch the fragments together so a window's network time is the
        // slowest segment, not their sum. Bytes the background prefetch already
        // brought in are used directly; only misses reach the network, still in
        // parallel. The work queue is bounded, so a long window never opens
        // more connections than the host can reuse.
        let segments: Vec<Segment> = self.media.segments[self.next..end].to_vec();
        let fragments = match self.fetch_window(&segments) {
            Ok(fragments) => fragments,
            Err(error) => {
                diag(
                    "ERROR",
                    "hls.stage",
                    &format!(
                        "fragment fetch failed at index {} ({}/{} segments, live={}): {}",
                        self.next,
                        count,
                        self.media.segments.len(),
                        self.live,
                        error
                    ),
                );
                return Err(error);
            }
        };
        // One base per track, captured from the first fragment. Video and audio
        // state their times in different timescales, so a single delta applied
        // to both would start them at different instants and drift. Rebasing is
        // done after the fetch, in playlist order, so no state is shared
        // between the workers.
        let mut deltas: Vec<u64> = Vec::new();
        for (segment, mut fragment) in segments.iter().zip(fragments) {
            if self.media.map.is_some() {
                // A discontinuity restarts the media timeline. Capture this
                // fragment's own bases as the new origin so its time begins at
                // zero again, and later fragments follow this new origin.
                if deltas.is_empty() || segment.discontinuity {
                    deltas = tfdt::first_bases(&fragment);
                }
                tfdt::rebase_each(&mut fragment, &deltas);
            }
            bytes.extend_from_slice(&fragment);
        }

        // When the audio lives in its own playlist, join the fragments that
        // cover the same span of the stream as the pictures just fetched. This
        // is best effort: a failure here leaves the window without sound rather
        // than failing the video.
        // A separate audio rendition is best effort: a failed or malformed
        // playlist must leave the picture playable, not fail the whole window.
        #[cfg(feature = "native-audio")]
        let audio_file = self.open_audio_window(self.next, count).ok().flatten();

        let file = TempFile::write(&bytes)?;
        let decoder = crate::platform::open_hls_window(file.path())?;
        // How long the window took, split from the playlist read above: a field
        // report about a slow start needs to say whether the network or the
        // decoder is the part that is slow.
        let joined = began.elapsed();
        // Which track owns the sound of this window, so a log can tell a stream
        // whose audio plays through the window decoder from one whose sound was
        // joined separately.
        #[cfg(feature = "native-audio")]
        let window_audio = if audio_file.is_some() {
            "separate playlist"
        } else if decoder.carries_audio() {
            "decoder-owned"
        } else {
            "none"
        };
        #[cfg(not(feature = "native-audio"))]
        let window_audio = "disabled";
        crate::diag(
            "INFO",
            "hls.window",
            &format!(
                "joined {count} segment(s) from index {} at {start:?} ({} bytes), audio={window_audio}, ready in {joined:?}",
                self.next,
                bytes.len()
            ),
        );
        let mut info = decoder.info();
        info.duration = if self.live {
            None
        } else {
            Some(self.media.total())
        };
        self.info = info;
        self.window = Some(Window {
            decoder,
            file,
            #[cfg(feature = "native-audio")]
            audio_file,
            count,
            start,
            base: None,
        });
        self.first_window = false;
        // A window opened after play, or after a seek, must inherit the player's
        // control state. This matters most for a decoder that owns its audio on
        // one clock: without it a live rollover would silently drop the sound.
        #[cfg(feature = "native-audio")]
        self.apply_window_controls();
        // While this window plays, bring in the fragments the next one needs.
        // The fetch is bounded and cooperative, so it neither holds the decode
        // thread nor outlives a close.
        self.schedule_prefetch(count);
        Ok(())
    }

    /// Takes the fragments a window needs from the prefetch cache, fetching any
    /// misses in parallel and returning the bytes in playlist order.
    ///
    /// Consuming an entry removes it, so the cache holds at most the upcoming
    /// window and stale bytes cannot accumulate across windows.
    fn fetch_window(&mut self, segments: &[Segment]) -> Result<Vec<Vec<u8>>, VideoError> {
        let mut prefetch = self.prefetch.lock().unwrap_or_else(|e| e.into_inner());
        let mut cached: Vec<Option<Vec<u8>>> = Vec::with_capacity(segments.len());
        let mut misses: Vec<Segment> = Vec::new();
        for segment in segments {
            match prefetch.fragments.remove(&FragmentKey::new(segment)) {
                Some(bytes) => cached.push(Some(bytes)),
                None => {
                    cached.push(None);
                    misses.push(segment.clone());
                }
            }
        }
        drop(prefetch);

        let (fetched, rate) = fetch_many(&misses, &self.stop)?;
        self.note_bandwidth(rate);
        let mut fetched = fetched.into_iter();
        let mut fragments = Vec::with_capacity(segments.len());
        for slot in cached {
            fragments.push(match slot {
                Some(bytes) => bytes,
                None => fetched.next().ok_or_else(|| {
                    VideoError::Source("an HLS fragment fetch returned no bytes".to_owned())
                })?,
            });
        }
        Ok(fragments)
    }

    /// Spawns a short-lived fetch of the window that follows the one just
    /// opened, into the shared cache. A generation stamp keeps an abandoned
    /// fetch from writing its result back after a seek or the next window.
    fn schedule_prefetch(&mut self, count: usize) {
        let start = self.next.saturating_add(count);
        let end = start.saturating_add(WINDOW).min(self.media.segments.len());
        if start >= end {
            // Nothing follows this window yet (the live edge, or end of file).
            return;
        }
        #[cfg_attr(not(feature = "native-audio"), expect(unused_mut))]
        let mut segments: Vec<Segment> = self.media.segments[start..end].to_vec();
        // Bring in the separate audio rendition for the same window too. Without
        // this its fragments would be fetched on the decode thread at rollover,
        // where the wait is heard as a gap. Both sets share one cache, keyed by
        // address, so a later audio playlist refresh only costs a network
        // fallback for the fragments it actually changed.
        #[cfg(feature = "native-audio")]
        if let Some((_, audio)) = self.audio_segments_for(start, end - start) {
            segments.extend(audio);
        }
        if self.stop.is_stopped() {
            return;
        }
        // A clone watches the same local and player flags: a close stops the
        // worker between fragments, and the retry waits inside each fetch.
        let stop = self.stop.clone();
        let generation = {
            let mut prefetch = self.prefetch.lock().unwrap_or_else(|e| e.into_inner());
            // Anything still cached here belongs to a window we skipped, so it
            // is stale. A new generation also invalidates an older in-flight
            // fetch that has not yet written its result.
            prefetch.fragments.clear();
            prefetch.generation = prefetch.generation.wrapping_add(1);
            prefetch.generation
        };
        let prefetch = Arc::clone(&self.prefetch);
        let _ = std::thread::Builder::new()
            .name("hls-prefetch".to_owned())
            .spawn(move || {
                let Ok((fragments, rate)) = fetch_many(&segments, &stop) else {
                    return;
                };
                let mut prefetch = prefetch.lock().unwrap_or_else(|e| e.into_inner());
                // The measurement is valid however the generation stands: the
                // link did deliver those bytes, even if the window they were
                // fetched for is no longer the one about to play.
                if let Some(rate) = rate {
                    prefetch.bandwidth.record(rate);
                }
                if prefetch.generation != generation {
                    return;
                }
                for (segment, bytes) in segments.iter().zip(fragments) {
                    prefetch.fragments.insert(FragmentKey::new(segment), bytes);
                }
            });
    }

    /// Invalidates any in-flight or cached prefetch, e.g. on a seek or close.
    fn invalidate_prefetch(&mut self) {
        let mut prefetch = self.prefetch.lock().unwrap_or_else(|e| e.into_inner());
        prefetch.fragments.clear();
        prefetch.generation = prefetch.generation.wrapping_add(1);
    }

    /// Mirrors the player's control state onto the open window's decoder.
    ///
    /// The paired audio backend keeps its own state, but a decoder that carries
    /// audio (Windows Media Foundation) only learns of play, speed, mute and
    /// volume through these calls. Replaying them here covers a window that was
    /// opened after the command was first issued.
    #[cfg(feature = "native-audio")]
    fn apply_window_controls(&mut self) {
        let playing = self.playing;
        let speed = self.speed;
        let muted = self.muted;
        let volume = self.volume;
        if let Some(window) = self.window.as_mut() {
            let decoder = &mut *window.decoder;
            decoder.set_speed(speed);
            decoder.set_muted(muted);
            decoder.set_volume(volume);
            if playing {
                decoder.play();
            } else {
                decoder.pause();
            }
        }
    }

    /// The audio fragments covering the video window at `first`, when a
    /// separate audio playlist is available.
    ///
    /// The audio playlist has its own media sequence, so video indices are
    /// matched to audio ones by their absolute sequence numbers. That stays
    /// lined up as a live playlist slides, even when the two lists slide at
    /// slightly different times. The window opener and the prefetch worker both
    /// call this so they ask for exactly the same fragments.
    #[cfg(feature = "native-audio")]
    fn audio_segments_for(
        &self,
        first: usize,
        count: usize,
    ) -> Option<(Option<Segment>, Vec<Segment>)> {
        let audio_media = self.audio_media.as_ref()?;
        let offset = self.media.sequence as i64 - audio_media.sequence as i64;
        let mut indices = Vec::with_capacity(count);
        for index in first..first.saturating_add(count) {
            let audio_index = index as i64 + offset;
            if audio_index >= 0 && (audio_index as usize) < audio_media.segments.len() {
                indices.push(audio_index as usize);
            }
        }
        if indices.is_empty() {
            return None;
        }
        let segments = indices
            .iter()
            .map(|&index| audio_media.segments[index].clone())
            .collect();
        Some((audio_media.map.clone(), segments))
    }

    /// Joins the audio fragments that cover the same span as the video window
    /// at `first`, into a file the audio backend can open.
    ///
    /// Fragments already brought in by the background prefetch are consumed
    /// first; only misses reach the network here. Keeping this fetch off the
    /// decode thread at a rollover is what stops the audible gap at the join.
    #[cfg(feature = "native-audio")]
    fn open_audio_window(
        &mut self,
        first: usize,
        count: usize,
    ) -> Result<Option<TempFile>, VideoError> {
        let Some(audio_url) = self.audio_url.clone() else {
            return Ok(None);
        };
        let poll = self
            .audio_media
            .as_ref()
            .map_or(self.media.target, |media| media.target);
        if self.live && self.audio_polled.elapsed() >= playlist_poll(poll) {
            if let Ok(text) = text_inner(&audio_url, &self.stop) {
                if let Ok(media) = playlist::media(&text, &audio_url) {
                    self.audio_media = Some(media);
                }
            }
            self.audio_polled = Instant::now();
        }
        let Some((map, segments)) = self.audio_segments_for(first, count) else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        let has_map = map.is_some();
        if let Some(map) = map {
            let Ok(init) = init_bytes(&mut self.audio_init, &map, &self.stop) else {
                return Ok(None);
            };
            bytes.extend_from_slice(&init);
        }
        let fragments = match self.fetch_window(&segments) {
            Ok(fragments) => fragments,
            Err(_) => return Ok(None),
        };
        let mut deltas: Vec<u64> = Vec::new();
        for (segment, mut fragment) in segments.iter().zip(fragments) {
            if has_map {
                if deltas.is_empty() || segment.discontinuity {
                    deltas = tfdt::first_bases(&fragment);
                }
                tfdt::rebase_each(&mut fragment, &deltas);
            }
            bytes.extend_from_slice(&fragment);
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        TempFile::write(&bytes).map(Some)
    }

    /// Opens audio on the window that is now current, when it is not already
    /// the one playing.
    ///
    /// When the audio lives in its own playlist, the window holds a joined file
    /// of just the audio fragments: both decoders then start at the same span of
    /// the stream. Otherwise they read the same joined file, with the audio
    /// carried inside it. A source with no audio track simply leaves the player
    /// ready false and is not tried again.
    #[cfg(feature = "native-audio")]
    fn sync_audio(&mut self) {
        // A window is already reopened on rollover, which picks up most device
        // changes. Polling here also catches a headphone switch in the middle
        // of a long window. It is a no-op until a source has been prepared.
        if let Some(audio) = self.audio.as_mut() {
            audio.observe_output_route();
        }
        let Some(window) = self.window.as_ref() else {
            return;
        };
        // Media Foundation reads the audio and video tracks of the joined file
        // together on one clock. Pairing the file a second time here would play
        // the same sound twice.
        if window.decoder.carries_audio() {
            return;
        }
        let path = window.audio_file.as_ref().unwrap_or(&window.file).path();
        let uri = crate::audio::uri_for_path(path);
        if self
            .audio
            .as_ref()
            .is_some_and(|audio| audio.is_source(&uri))
        {
            return;
        }
        if let Some(audio) = self.audio.as_mut() {
            crate::diag(
                "INFO",
                "hls.audio",
                &format!("attached a separate audio window for {uri}"),
            );
            audio.attach(&uri);
            if self.playing {
                audio.play();
            }
        }
    }

    /// Moves the separate audio to the point a seek landed on, once the window
    /// that covers it is open and its file is attached.
    ///
    /// The picture window begins at a segment boundary before the seek target.
    /// Without this the sound would restart there too, up to a segment early.
    #[cfg(feature = "native-audio")]
    fn seek_audio(&mut self) {
        let Some(position) = self.seeked_to else {
            return;
        };
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.decoder.carries_audio() {
            self.seeked_to = None;
            return;
        }
        let local = position.saturating_sub(window.start);
        if let Some(audio) = self.audio.as_mut() {
            audio.seek(local);
        }
        self.seeked_to = None;
    }

    /// Moves to the rendition the measured link can carry.
    ///
    /// Called between windows, where the path is already free. A switch that
    /// cannot read its playlist leaves the stream on the rendition it has: a
    /// wrong measurement must never stop the picture.
    fn adapt(&mut self) {
        let Some(master) = self.master.as_ref() else {
            return;
        };
        let measured = self
            .prefetch
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bandwidth
            .estimate();
        let Some(measured) = measured else {
            return;
        };
        let Some(chosen) =
            playlist::choose_for_bandwidth(&master.variants, measured, master.chosen)
        else {
            return;
        };
        let url = master.variants[chosen].uri.clone();
        let audio_url = playlist::choose_audio(&master.audios, &master.variants[chosen])
            .map(|rendition| rendition.uri.clone());
        let _ = self.switch_to(chosen, &url, audio_url);
    }

    /// Opens the next window on another rendition, lined up with the position
    /// the stream has reached.
    ///
    /// Every cached fragment belongs to the rendition being left, and the new
    /// one's init section carries its own codec configuration, so both are
    /// dropped. A live stream rejoins at the live edge it starts with; a fixed
    /// playlist keeps the exact place, so a switch never skips or repeats
    /// content there.
    fn switch_to(
        &mut self,
        index: usize,
        url: &str,
        audio_url: Option<String>,
    ) -> Result<(), VideoError> {
        let page = text(url, &self.stop)?;
        let media = playlist::media(&page, url)?;
        if media.segments.is_empty() {
            return Err(VideoError::Source(
                "the HLS rendition names no segment".to_owned(),
            ));
        }
        let next = if self.live {
            media.segments.len().saturating_sub(LIVE_BACK)
        } else {
            media.index_of(self.media.start(self.next))
        };
        self.invalidate_prefetch();
        self.init = None;
        self.url = url.to_owned();
        self.media = media;
        self.next = next;
        if let Some(master) = self.master.as_mut() {
            master.chosen = index;
        }
        #[cfg(feature = "native-audio")]
        {
            self.audio_url = audio_url;
            self.audio_media = None;
            self.audio_init = None;
            self.audio_polled = Instant::now();
            if let Some(audio) = self.audio.as_mut() {
                audio.detach();
            }
        }
        #[cfg(not(feature = "native-audio"))]
        let _ = audio_url;
        Ok(())
    }

    /// Adds a measured transfer rate to the estimate the rendition choice
    /// reads. The estimate lives with the prefetch cache because that is the
    /// one piece of network state the decode thread and the background worker
    /// already share.
    fn note_bandwidth(&self, bits_per_second: Option<f64>) {
        let Some(bits_per_second) = bits_per_second else {
            return;
        };
        self.prefetch
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bandwidth
            .record(bits_per_second);
    }

    /// Steps to the window after the one just played.
    ///
    /// Returns false at the end of the media. A live stream that ran out of
    /// segments waits at the live edge for the next one instead.
    fn advance(&mut self, count: usize) -> Result<bool, VideoError> {
        let end = (self.next + count).min(self.media.segments.len());
        let played: f64 = self.media.segments[self.next..end]
            .iter()
            .map(|segment| segment.duration)
            .sum();
        self.next += count;
        if self.live {
            self.elapsed = self.elapsed.saturating_add(Duration::from_secs_f64(played));
        }
        // The decoder for this window is closed and the next one has to be
        // opened anyway, so this is the one moment a rendition switch costs no
        // stall. A fixed playlist that just played its last segment has nothing
        // left to line a switch up with, and a switch there would replay the
        // end.
        if self.live || self.next < self.media.segments.len() {
            self.adapt();
        }
        if self.next < self.media.segments.len() {
            return Ok(true);
        }
        if !self.live {
            return Ok(false);
        }

        let began = Instant::now();
        let stall = self.stall_timeout();
        let mut wait = self.poll_interval();
        let mut last_error = None;
        loop {
            if self.shutting_down() {
                return Ok(false);
            }
            match self.refresh_inner(true) {
                Ok(()) => {
                    if self.next < self.media.segments.len() {
                        // The player's clock kept running while the decode
                        // thread waited here for the live edge. Move the media
                        // timeline forward by the same wall time, so the next
                        // window is born at the play head instead of behind it,
                        // where every frame would be dropped as late. Added
                        // once, on the exit from the wait, not per poll.
                        self.elapsed = self.elapsed.saturating_add(began.elapsed());
                        return Ok(true);
                    }
                    if self.media.ended {
                        self.live = false;
                        return Ok(false);
                    }
                    // The playlist answered but did not move. Wait longer
                    // before asking again, so a slow stream keeps its bandwidth
                    // for the segments it does have.
                    wait = (wait * 2).min(WAIT_MAX);
                }
                Err(failure) if failure.transient => {
                    // A live edge may be behind a flaky network. Keep trying
                    // until the stall limit, then report the last failure,
                    // which says more than a bare timeout.
                    last_error = Some(failure.error);
                    wait = (wait * 2).min(WAIT_MAX);
                }
                Err(failure) => return Err(failure.error),
            }
            if began.elapsed() >= stall {
                return Err(last_error.unwrap_or_else(|| {
                    VideoError::Source("the live HLS stream sent no new segment".to_owned())
                }));
            }
            // Poll the close signal while waiting at the live edge: a player
            // closed during a long wait must not stay in this loop until the
            // stall timeout expires.
            if sleep_or_stop(&self.stop, wait) {
                return Ok(false);
            }
        }
    }
}

impl VideoDecoder for Stream {
    fn info(&self) -> VideoInfo {
        self.info
    }

    fn is_live(&self) -> bool {
        self.live
    }

    fn is_seekable(&self) -> bool {
        self.seekable
    }

    #[cfg(feature = "native-audio")]
    fn play(&mut self) {
        self.playing = true;
        if let Some(audio) = self.audio.as_mut() {
            audio.play();
        }
        if let Some(window) = self.window.as_mut() {
            window.decoder.play();
        }
    }

    #[cfg(feature = "native-audio")]
    fn pause(&mut self) {
        self.playing = false;
        if let Some(audio) = self.audio.as_mut() {
            audio.pause();
        }
        if let Some(window) = self.window.as_mut() {
            window.decoder.pause();
        }
    }

    #[cfg(feature = "native-audio")]
    fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
        if let Some(audio) = self.audio.as_mut() {
            audio.set_speed(speed);
        }
        if let Some(window) = self.window.as_mut() {
            window.decoder.set_speed(speed);
        }
    }

    #[cfg(feature = "native-audio")]
    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(audio) = self.audio.as_mut() {
            audio.set_muted(muted);
        }
        if let Some(window) = self.window.as_mut() {
            window.decoder.set_muted(muted);
        }
    }

    #[cfg(feature = "native-audio")]
    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if let Some(audio) = self.audio.as_mut() {
            audio.set_volume(volume);
        }
        if let Some(window) = self.window.as_mut() {
            window.decoder.set_volume(volume);
        }
    }

    #[cfg(feature = "native-audio")]
    fn is_muted(&self) -> bool {
        self.muted
    }

    #[cfg(feature = "native-audio")]
    fn volume(&self) -> f32 {
        self.volume
    }

    #[cfg(feature = "native-audio")]
    fn sync_clock(&mut self, position: Duration) {
        if let Some(window) = self.window.as_mut() {
            // `position` is the picture time published to the player, shifted
            // forward by this window's absolute start. A decoder that carries
            // its own audio numbers its PCM samples from the window's own
            // origin, so give it the window-local time back. Without this, a
            // live rollover would leave the audio head past its PCM buffer.
            let start = window.start;
            window.decoder.sync_clock(position.saturating_sub(start));
        }
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        // A seek may be the first decode-thread operation after `open`, so the
        // playlist has to be loaded before its length can decide seekability.
        if let Err(error) = self.ensure_loaded() {
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            self.report_failure("hls.open", &error.to_string());
            return Err(error);
        }
        if !self.seekable {
            return Err(VideoError::UnsupportedOperation {
                operation: "seeking a live HLS stream",
            });
        }
        self.next = self.media.index_of(position);
        // Bytes fetched for the old position are no longer the ones ahead of
        // the play head. Drop them and invalidate any in-flight fetch.
        self.invalidate_prefetch();
        #[cfg(feature = "native-audio")]
        {
            self.seeked_to = Some(position);
            if let Some(audio) = self.audio.as_mut() {
                audio.detach();
            }
        }
        if let Some(mut window) = self.window.take() {
            // Release the decoder's file handles before the TempFile drops, so
            // Windows can delete it. A seek abandons this window for good.
            window.decoder.close();
        }
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        self.next_picture(false, rgba)
    }

    fn read_nv12(&mut self, nv12: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        self.next_picture(true, nv12)
    }

    /// The layout the decoder inside the open window fills.
    ///
    /// A window is a platform decoder like any other, so the stream reports
    /// what that decoder produces rather than a layout of its own. Without
    /// this the player would ask a planar decoder for RGBA and the window would
    /// fail on its first picture — which is what a 1080p live stream did before
    /// the stream forwarded the format.
    fn output_format(&self) -> PixelFormat {
        self.window
            .as_ref()
            .map_or(PixelFormat::Rgba8, |window| window.decoder.output_format())
    }

    fn nv12_color(&self) -> YuvColor {
        self.window
            .as_ref()
            .map_or_else(YuvColor::default, |window| window.decoder.nv12_color())
    }

    fn carries_audio(&self) -> bool {
        self.window
            .as_ref()
            .is_some_and(|window| window.decoder.carries_audio())
    }

    fn close(&mut self) {
        // Signal every blocking fetch and retry wait first. A close must not
        // wait on an unresponsive socket, and the background prefetch shares
        // this stop so it leaves its queue as well.
        self.stop.signal();
        self.invalidate_prefetch();
        #[cfg(feature = "native-audio")]
        if let Some(audio) = self.audio.as_mut() {
            audio.detach();
        }
        if let Some(mut window) = self.window.take() {
            window.decoder.close();
        }
    }

    fn observe_shutdown(&mut self, flag: Arc<AtomicBool>) {
        self.stop.observe(flag);
    }

    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn attach_session(&mut self, session: PlaybackSession) {
        self.session = Some(session);
    }

    /// Rebuilds the reader on the decode thread after a stream failure.
    ///
    /// An HLS window cannot resume once its playlist or fragments failed, so a
    /// retry starts from a fresh load on the next read. Returning true lets the
    /// worker schedule that read; the public failure stays visible until a new
    /// window actually publishes a picture. Nothing here reports success to the
    /// session directly — the worker owns that transition.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn recover(&mut self) -> bool {
        if self.permanent_failure {
            // The address itself is unusable. Do not reload it again.
            return false;
        }
        self.loaded = false;
        // Start from the address the application opened, not the rendition URL
        // stored by the last load: a signed playlist's fragment URLs expire, and
        // re-requesting them only repeats the 403/410 forever. Re-read the
        // original so `choose` can obtain fresh addresses.
        self.url = self.source_url.clone();
        if let Some(mut window) = self.window.take() {
            window.decoder.close();
        }
        self.init = None;
        #[cfg(feature = "native-audio")]
        {
            self.audio_media = None;
            self.audio_init = None;
            if let Some(audio) = self.audio.as_mut() {
                audio.detach();
            }
        }
        self.invalidate_prefetch();
        diag(
            "INFO",
            "hls",
            "will reload the playlist and window on the next read",
        );
        true
    }
}

impl Stream {
    /// A 403/410 address is dead: reloading the playlist cannot revive it, and
    /// retrying only floods the log with "permanent failure" lines. Remember it
    /// so recovery stops and the public failure stays truthful until the user
    /// picks another channel.
    fn note_failure(&mut self, error: &VideoError) {
        let text = error.to_string();
        if text.contains("answered 403") || text.contains("answered 410") {
            self.permanent_failure = true;
        }
    }

    /// Reports an HLS failure once per recovery generation.
    ///
    /// One failed read can surface through several layers, but the session's
    /// generation changes only when the worker starts or finishes a recovery
    /// attempt. Keeping the last reported generation here yields one session
    /// failure per attempt. The reason is redacted again so no caller has to
    /// remember that a live address can carry a token in its query.
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    fn report_failure(&mut self, stage: &'static str, reason: &str) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let generation = session.generation();
        if self
            .reported_failure
            .is_some_and(|(seen, _)| seen == generation)
        {
            return;
        }
        self.reported_failure = Some((generation, stage));
        session.failed(stage, redact_for_report(reason));
    }

    /// One picture from the open window, in the layout the window produces.
    ///
    /// The stream opens the next window when this one runs out, so a long
    /// source is one continuous read at the player's side however many files
    /// the fragments were joined into. `native` selects the planar read, which
    /// is what a decoder that reports [`PixelFormat::Nv12`] fills.
    fn next_picture(
        &mut self,
        native: bool,
        buffer: &mut Vec<u8>,
    ) -> Result<Option<DecodedFrame>, VideoError> {
        // The playlist is loaded here, on the decode thread, never in `open`,
        // `info` or `output_format` on the composition thread.
        if let Err(error) = self.ensure_loaded() {
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            self.report_failure("hls.open", &error.to_string());
            self.note_failure(&error);
            // A permanent status must not re-arm the recovery loop forever.
            return if self.permanent_failure {
                Err(VideoError::Decode(error.to_string()))
            } else {
                Err(error)
            };
        }
        loop {
            if self.shutting_down() {
                return Ok(None);
            }
            if self.window.is_none() {
                // A close during a fetch is a clean end, not an error to report.
                if let Err(error) = self.open_window() {
                    if self.shutting_down() {
                        return Ok(None);
                    }
                    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
                    self.report_failure("hls.window", &error.to_string());
                    self.note_failure(&error);
                    return if self.permanent_failure {
                        Err(VideoError::Decode(error.to_string()))
                    } else {
                        Err(error)
                    };
                }
            }
            #[cfg(feature = "native-audio")]
            {
                self.sync_audio();
                self.seek_audio();
            }
            let mut window = self
                .window
                .take()
                .expect("a window was opened just above this line");
            let read = if native {
                window.decoder.read_nv12(buffer)
            } else {
                window.decoder.read(buffer)
            };
            // The window decoder can block while the player is being closed.
            // A close that arrived during that read is a clean end, not a
            // picture to publish and not an error to report. Release the file
            // before the window drops so Windows can unlink it.
            if self.shutting_down() {
                #[cfg(feature = "native-audio")]
                if let Some(audio) = self.audio.as_mut() {
                    audio.detach();
                }
                window.decoder.close();
                return Ok(None);
            }
            match read {
                Ok(Some(mut frame)) => {
                    let base = *window.base.get_or_insert(frame.pts);
                    frame.pts = frame.pts.saturating_sub(base).saturating_add(window.start);
                    self.window = Some(window);
                    return Ok(Some(frame));
                }
                Ok(None) => {
                    let count = window.count;
                    // The audio backend holds this file open. Release it before
                    // the window drops the file, or Windows cannot delete it.
                    #[cfg(feature = "native-audio")]
                    if let Some(audio) = self.audio.as_mut() {
                        audio.detach();
                    }
                    // Close the decoder before its TempFile drops, so Windows
                    // can unlink the file it still has open.
                    window.decoder.close();
                    drop(window);
                    if !self.advance(count)? {
                        return Ok(None);
                    }
                }
                Err(error) => {
                    // The window was taken above; put it back so a transient
                    // decode error does not discard a still-usable window and
                    // force a network re-fetch on the next read.
                    self.window = Some(window);
                    return Err(error);
                }
            }
        }
    }
}

/// One segment's bytes, with its byte range when it names one.
fn fetch(segment: &Segment, stop: &Stop) -> Result<Vec<u8>, VideoError> {
    fetch_timed(segment, stop).map(|(bytes, _)| bytes)
}

/// The same fetch, with the time it took, so the stream can measure the link it
/// plays over. The time covers the retries a transient failure needed, so the
/// rate is never flattered by a request that only succeeded on its second try.
fn fetch_timed(segment: &Segment, stop: &Stop) -> Result<(Vec<u8>, Duration), VideoError> {
    let began = Instant::now();
    let bytes = fetch_bytes(
        &segment.uri,
        segment.range,
        MAX_MEDIA,
        media_deadline(segment),
        ResourceKind::Media,
        stop,
    )
    .map_err(|failure| failure.error)?;
    Ok((bytes, began.elapsed()))
}

/// Keeps the slowest rate of a window.
///
/// The slowest fragment is the rate the link can be relied on for, which is what
/// a rendition choice needs. A transfer too short to time measures the host's
/// cache rather than the network, so it states nothing.
fn note_rate(slowest: &mut Option<f64>, bytes: usize, elapsed: Duration) {
    let seconds = elapsed.as_secs_f64();
    if bytes == 0 || seconds < 0.002 {
        return;
    }
    let rate = bytes as f64 * 8.0 / seconds;
    if rate.is_finite() {
        *slowest = Some(slowest.map_or(rate, |previous: f64| previous.min(rate)));
    }
}

/// How long a fragment may take, from the time the playlist states. A segment
/// should arrive well within the time it plays; the bounds keep a short one on
/// a slow link and a long one on a broken link both reasonable. There is no
/// fixed content limit here — only a bound on how long a request may hang.
fn media_deadline(segment: &Segment) -> Duration {
    let seconds = segment.duration * 4.0;
    if !seconds.is_finite() || seconds <= 0.0 {
        return MEDIA_DEADLINE_MIN;
    }
    Duration::from_secs_f64(seconds.min(MEDIA_DEADLINE_MAX.as_secs_f64())).max(MEDIA_DEADLINE_MIN)
}

/// The close signal a stream shares with its blocking reads.
///
/// The player hands over its own flag before the decode thread starts. The
/// stream also owns a local flag so a direct `close()` can stop an in-flight
/// prefetch that has no player flag to watch. Both are one-way: once signalled,
/// every clone stays signalled.
#[derive(Clone, Default)]
struct Stop {
    local: Arc<AtomicBool>,
    external: Option<Arc<AtomicBool>>,
}

impl Stop {
    /// Watches the player's close flag as well as the stream's own.
    fn observe(&mut self, external: Arc<AtomicBool>) {
        self.external = Some(external);
    }

    /// Signals every reader that shares this stop.
    fn signal(&self) {
        self.local.store(true, Ordering::Relaxed);
        if let Some(external) = self.external.as_ref() {
            external.store(true, Ordering::Relaxed);
        }
    }

    /// Whether either a close or the player's shutdown was signalled.
    fn is_stopped(&self) -> bool {
        self.local.load(Ordering::Relaxed)
            || self
                .external
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }
}

/// Sleeps for `duration`, waking early when a close is signalled. Returns true
/// when the wait was cut short by a close.
fn sleep_or_stop(stop: &Stop, duration: Duration) -> bool {
    const STEP: Duration = Duration::from_millis(50);
    let mut left = duration;
    loop {
        if stop.is_stopped() {
            return true;
        }
        if left.is_zero() {
            return false;
        }
        let slice = left.min(STEP);
        std::thread::sleep(slice);
        left -= slice;
    }
}

/// The failure a cancelled read reports. It is permanent: a closed player will
/// never come back to the request.
fn cancelled_error() -> VideoError {
    VideoError::Source("playback closed while reading an HLS fragment".to_owned())
}

fn cancelled() -> Failure {
    Failure::permanent(cancelled_error())
}

/// Fetches a window's fragments concurrently, in playlist order, and reports
/// the slowest rate the window arrived at in bits per second.
///
/// A window's network time becomes the slowest segment rather than the sum of
/// them. Workers pull from one bounded counter, so at most `PARALLEL_FETCH`
/// requests are in flight and a close stops the queue before it starts more
/// work. Results land in their playlist slots, so the joined file keeps its
/// order however the requests finish.
fn fetch_many(
    segments: &[Segment],
    stop: &Stop,
) -> Result<(Vec<Vec<u8>>, Option<f64>), VideoError> {
    if segments.is_empty() {
        return Ok((Vec::new(), None));
    }
    if stop.is_stopped() {
        return Err(cancelled_error());
    }
    let workers = PARALLEL_FETCH.min(segments.len());
    if workers == 1 {
        let mut fragments = Vec::with_capacity(segments.len());
        let mut slowest = None;
        for segment in segments {
            let (bytes, elapsed) = fetch_timed(segment, stop)?;
            note_rate(&mut slowest, bytes.len(), elapsed);
            fragments.push(bytes);
        }
        return Ok((fragments, slowest));
    }
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<Result<(Vec<u8>, Duration), VideoError>>>> =
        (0..segments.len()).map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let next = &next;
            let slots = &slots;
            scope.spawn(move || {
                loop {
                    if stop.is_stopped() {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= segments.len() {
                        break;
                    }
                    let result = fetch_timed(&segments[index], stop);
                    *slots[index].lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                }
            });
        }
    });
    if stop.is_stopped() {
        return Err(cancelled_error());
    }
    let mut slowest = None;
    let mut fragments = Vec::with_capacity(segments.len());
    for slot in slots {
        match slot.into_inner().unwrap_or_else(|e| e.into_inner()) {
            Some(Ok((bytes, elapsed))) => {
                note_rate(&mut slowest, bytes.len(), elapsed);
                fragments.push(bytes);
            }
            Some(Err(error)) => return Err(error),
            None => return Err(cancelled_error()),
        }
    }
    Ok((fragments, slowest))
}

/// A playlist's text.
fn text(url: &str, stop: &Stop) -> Result<String, VideoError> {
    text_inner(url, stop).map_err(|failure| failure.error)
}

/// The same fetch, keeping whether the failure may pass and checking for a
/// close between attempts.
fn text_inner(url: &str, stop: &Stop) -> Result<String, Failure> {
    let bytes = fetch_bytes(
        url,
        None,
        MAX_PLAYLIST,
        PLAYLIST_DEADLINE,
        ResourceKind::Playlist,
        stop,
    )?;
    let text = String::from_utf8_lossy(&bytes);
    // A UTF-8 BOM is legal at the head of a playlist but is not whitespace, so
    // the first `#EXTM3U` would otherwise not be recognised and the playlist
    // would be rejected. Strip it before parsing.
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
}

/// The initialization section of a fragmented stream, kept between windows.
///
/// A live stream names the same section in every playlist it serves. Fetching
/// it once and reusing it saves a request on each rollover and, more
/// importantly, keeps a rollover from failing because the section is briefly
/// unreachable while the segments after it are not.
fn init_bytes(
    cache: &mut Option<(String, Option<Range>, Vec<u8>)>,
    map: &Segment,
    stop: &Stop,
) -> Result<Vec<u8>, VideoError> {
    if let Some((uri, range, bytes)) = cache.as_ref() {
        if *uri == map.uri && *range == map.range {
            return Ok(bytes.clone());
        }
    }
    let bytes = fetch(map, stop)?;
    *cache = Some((map.uri.clone(), map.range, bytes.clone()));
    Ok(bytes)
}

/// How often a live playlist is polled, from the target duration it states.
fn playlist_poll(target: f64) -> Duration {
    let seconds = (target * 0.5).clamp(POLL_MIN.as_secs_f64(), POLL_MAX.as_secs_f64());
    Duration::from_secs_f64(seconds)
}

/// How long a live playlist may stay unchanged before the stream fails, from
/// the target duration it states.
fn playlist_stall(target: f64) -> Duration {
    let seconds = (target * 5.0).clamp(STALL_MIN.as_secs_f64(), STALL_MAX.as_secs_f64());
    Duration::from_secs_f64(seconds)
}

/// A failed fetch, with whether trying again could succeed.
#[derive(Debug)]
struct Failure {
    error: VideoError,
    /// Whether the same request may work on a later attempt.
    transient: bool,
    /// The wait the server asked for, when it asked for one.
    retry_after: Option<Duration>,
}

impl Failure {
    /// A failure that repeating the request cannot fix.
    fn permanent(error: VideoError) -> Self {
        Self {
            error,
            transient: false,
            retry_after: None,
        }
    }
}

/// The bytes an address holds, or the part of them a range names.
///
/// A transient failure is tried twice more with a growing wait, so a reset
/// connection on a live stream costs a moment rather than the whole stream. A
/// permanent failure, such as an address that answered 404, is reported at
/// once: trying it again only delays the error the caller needs to see.
fn fetch_bytes(
    url: &str,
    range: Option<Range>,
    limit: u64,
    deadline: Duration,
    kind: ResourceKind,
    stop: &Stop,
) -> Result<Vec<u8>, Failure> {
    let mut attempt = 1;
    loop {
        if stop.is_stopped() {
            return Err(cancelled());
        }
        let failure = match get_once(url, range, limit, deadline, kind, stop) {
            Ok(bytes) => return Ok(bytes),
            Err(failure) => failure,
        };
        let wait = next_retry(failure, attempt, stop)?;
        if sleep_or_stop(stop, wait) {
            return Err(cancelled());
        }
        attempt += 1;
    }
}

/// Decides whether a failed request gets another attempt.
///
/// A permanent failure is returned at once, a transient one carries the wait the
/// server asked for or a growing backoff, and a close cancels both cases without
/// waiting. Keeping the decision pure makes the retry policy testable without a
/// network.
fn next_retry(failure: Failure, attempt: usize, stop: &Stop) -> Result<Duration, Failure> {
    if stop.is_stopped() {
        return Err(cancelled());
    }
    if !failure.transient || attempt >= ATTEMPTS {
        let cause = if failure.transient {
            "the attempt limit"
        } else {
            "a permanent failure"
        };
        // Neither the reason nor the address is logged: either can name a live
        // URL whose query carries a token. The policy outcome is enough to
        // explain why the read stopped.
        diag(
            "WARN",
            "hls",
            &format!("an HLS request stopped after attempt {attempt} because of {cause}"),
        );
        return Err(failure);
    }
    Ok(failure.retry_after.unwrap_or_else(|| retry_gap(attempt)))
}

/// One request, with the range and size checks that keep it honest.
fn get_once(
    url: &str,
    range: Option<Range>,
    limit: u64,
    deadline: Duration,
    kind: ResourceKind,
    stop: &Stop,
) -> Result<Vec<u8>, Failure> {
    if stop.is_stopped() {
        return Err(cancelled());
    }
    // Every error this function builds carries the address so a failure is
    // attributable. The query and user-info of a live URL hold its token, so
    // redact once here and use the safe form throughout.
    let safe_url = redact_for_report(url);
    if let Some(range) = range {
        if range.length > limit {
            return Err(Failure::permanent(VideoError::Source(format!(
                "the HLS resource at {safe_url} states {} bytes, larger than this reader accepts",
                range.length
            ))));
        }
    }
    let mut request = agent().get(url).timeout(deadline);
    if let Some(range) = range {
        request = request.set("Range", &format!("bytes={}-{}", range.offset, range.end()));
    }
    let response = request.call().map_err(|error| status_failure(url, error))?;
    let status = response.status();
    // A range request is answered with a slice. A server that ignores the
    // range may still answer the whole resource when the slice starts at zero;
    // for any other offset the bytes would be wrong, so it is refused.
    let partial = status == 206;
    let whole = status == 200 && range.is_some_and(|range| range.offset == 0);
    if let Some(range) = range {
        if !partial && !whole {
            return Err(Failure::permanent(VideoError::Source(format!(
                "the HLS server answered {status} for a byte range of {safe_url}"
            ))));
        }
        if partial {
            let Some((start, end)) = response.header("Content-Range").and_then(content_range)
            else {
                return Err(Failure::permanent(VideoError::Source(format!(
                    "the HLS server sent no usable Content-Range for {safe_url}"
                ))));
            };
            if start != range.offset {
                return Err(Failure::permanent(VideoError::Source(format!(
                    "the HLS server sent bytes from {start} instead of {} for {safe_url}",
                    range.offset
                ))));
            }
            if end != range.end() {
                return Err(Failure::permanent(VideoError::Source(format!(
                    "the HLS server sent bytes {start}-{end} instead of {}-{} for {safe_url}",
                    range.offset,
                    range.end()
                ))));
            }
        }
    } else if status != 200 {
        return Err(Failure::permanent(VideoError::Source(format!(
            "the HLS server answered {status} for {safe_url}"
        ))));
    }
    // A 200 carrying an HTML error page (or any document that plainly is not
    // media) must not reach the playlist parser or the platform decoder. The
    // playlist helper decides what is plausible for this resource; a missing
    // header is accepted, so servers that omit one still work. This does not
    // trust the `.m3u8` suffix, which a redirect can keep while serving HTML.
    if !accepts_content_type(kind, response.header("Content-Type")) {
        diag(
            "WARN",
            "hls",
            &format!("refusing a non-media Content-Type for {}", kind.label()),
        );
        return Err(Failure::permanent(VideoError::Source(format!(
            "the HLS server answered {status} with a non-media Content-Type for {safe_url}"
        ))));
    }

    let wanted = match range {
        Some(range) => range.length,
        None => limit,
    };
    let stated = response
        .header("Content-Length")
        .and_then(|value| value.trim().parse::<u64>().ok());
    // A whole-resource answer to a range at zero carries more than the slice
    // needs; only its head is read, so its stated size is not a reason to fail.
    if !whole && stated.is_some_and(|stated| stated > wanted) {
        return Err(Failure::permanent(VideoError::Source(format!(
            "the HLS resource at {safe_url} states more bytes than this reader accepts"
        ))));
    }
    let reserve = stated.unwrap_or(0).min(MAX_RESERVE as u64) as usize;
    let mut bytes = Vec::with_capacity(reserve);
    let mut reader = response.into_reader().take(wanted.saturating_add(1));
    let mut chunk = [0u8; 64 * 1024];
    loop {
        // A large fragment streams for a while. Check the close signal between
        // chunks so a player going away does not have to finish the body.
        if stop.is_stopped() {
            return Err(cancelled());
        }
        let read = reader
            .read(&mut chunk)
            .map_err(|error| body_failure(url, &error))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    if whole {
        if (bytes.len() as u64) < wanted {
            return Err(Failure::permanent(VideoError::Source(format!(
                "the HLS server sent {} of {wanted} bytes for {safe_url}",
                bytes.len()
            ))));
        }
        bytes.truncate(wanted as usize);
        return Ok(bytes);
    }
    if range.is_some() && bytes.len() as u64 != wanted {
        return Err(Failure::permanent(VideoError::Source(format!(
            "the HLS server sent {} of {wanted} bytes for {safe_url}",
            bytes.len()
        ))));
    }
    if bytes.len() as u64 > limit {
        return Err(Failure::permanent(VideoError::Source(format!(
            "the HLS resource at {safe_url} is larger than this reader accepts"
        ))));
    }
    Ok(bytes)
}

/// The HTTP client every request shares, so connections are reused.
fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .user_agent(concat!("cranpose-video/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Duration::from_secs(10))
            .redirects(5)
            // A window fetches several fragments from the same host. Keep a
            // few connections idle so they are reused instead of renegotiated
            // on every segment rollover.
            .max_idle_connections_per_host(8)
            .build()
    })
}

/// What an HTTP failure says, with the address that failed, and whether the
/// status is one that often clears on its own.
fn status_failure(url: &str, error: ureq::Error) -> Failure {
    match error {
        ureq::Error::Status(code, response) => {
            let transient = transient_status(code);
            let retry_after = if transient {
                response.header("Retry-After").and_then(retry_after_limit)
            } else {
                None
            };
            Failure {
                error: VideoError::Source(format!(
                    "the HLS server answered {code} for {}",
                    redact_for_report(url)
                )),
                transient,
                retry_after,
            }
        }
        error => {
            let transient = matches!(
                error.kind(),
                ureq::ErrorKind::Dns | ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Io
            );
            Failure {
                error: VideoError::Source(format!(
                    "cannot read {}: {error}",
                    redact_for_report(url)
                )),
                transient,
                retry_after: None,
            }
        }
    }
}

/// Whether a status is likely to clear on a later request.
fn transient_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// The wait a `Retry-After` in whole seconds asks for, never longer than the
/// reader is willing to wait. The date form is ignored; a retry follows after
/// the normal growing wait instead.
fn retry_after_limit(value: &str) -> Option<Duration> {
    let seconds: u64 = value.trim().parse().ok()?;
    Some(Duration::from_secs(seconds).min(RETRY_CAP))
}

/// The wait before retry number `attempt`, growing with each attempt and
/// scattered a little so many readers do not return at the same instant.
fn retry_gap(attempt: usize) -> Duration {
    let step = 250u64.saturating_mul(1u64 << attempt.saturating_sub(1).min(4));
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| u64::from(elapsed.subsec_millis()) % 250);
    Duration::from_millis(step + jitter)
}

/// Whether a failed body read may succeed when the request is repeated.
fn body_failure(url: &str, error: &io::Error) -> Failure {
    let transient = matches!(
        error.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::TimedOut
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::Interrupted
    );
    Failure {
        error: VideoError::Source(format!("cannot read {}: {error}", redact_for_report(url))),
        transient,
        retry_after: None,
    }
}

/// The first and last byte a `Content-Range` header states.
fn content_range(value: &str) -> Option<(u64, u64)> {
    let (unit, rest) = value.trim().split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let bounds = rest.trim_start().split('/').next()?;
    let (start, end) = bounds.split_once('-')?;
    Some((start.trim().parse().ok()?, end.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(seconds: f64) -> Segment {
        Segment {
            uri: "http://example.invalid/segment".to_owned(),
            range: None,
            duration: seconds,
            discontinuity: false,
        }
    }

    /// A stream with a short, parsed playlist but no network and no decoder.
    /// Enough to exercise the prefetch cache in isolation.
    fn test_stream() -> Stream {
        let url = "http://example.invalid/master.m3u8";
        let body = concat!(
            "#EXTM3U\n",
            "#EXT-X-VERSION:6\n",
            "#EXT-X-TARGETDURATION:4\n",
            "#EXT-X-MEDIA-SEQUENCE:0\n",
            "#EXTINF:4.0,\n",
            "seg0.ts\n",
            "#EXTINF:4.0,\n",
            "seg1.ts\n",
            "#EXTINF:4.0,\n",
            "seg2.ts\n",
            "#EXT-X-ENDLIST\n",
        );
        Stream {
            source_url: url.to_owned(),
            url: url.to_owned(),
            loaded: true,
            master: None,
            media: playlist::media(body, url).unwrap(),
            prefetch: Arc::new(Mutex::new(Prefetch {
                generation: 0,
                fragments: HashMap::new(),
                bandwidth: Bandwidth::default(),
            })),
            window: None,
            next: 0,
            first_window: true,
            elapsed: Duration::ZERO,
            live: false,
            seekable: true,
            polled: Instant::now(),
            init: None,
            info: VideoInfo {
                width: 0,
                height: 0,
                duration: None,
                frame_rate: None,
                rotation: Default::default(),
            },
            #[cfg(feature = "native-audio")]
            playing: false,
            #[cfg(feature = "native-audio")]
            speed: 1.0,
            #[cfg(feature = "native-audio")]
            muted: false,
            #[cfg(feature = "native-audio")]
            volume: 1.0,
            #[cfg(feature = "native-audio")]
            audio: Some(crate::audio::Audio::new()),
            #[cfg(feature = "native-audio")]
            audio_url: None,
            #[cfg(feature = "native-audio")]
            audio_media: None,
            #[cfg(feature = "native-audio")]
            audio_polled: Instant::now(),
            #[cfg(feature = "native-audio")]
            audio_init: None,
            #[cfg(feature = "native-audio")]
            seeked_to: None,
            stop: Stop::default(),
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            session: None,
            #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
            reported_failure: None,
            permanent_failure: false,
        }
    }

    #[test]
    fn opening_an_hls_address_never_panics_before_the_playlist_is_loaded() {
        // Regression: `Stream::new` used to parse a fake segment-less playlist
        // and `.expect()` it. `playlist::media` rejects playlists with no
        // segments, so constructing a live source panicked on the composition
        // thread before the decode thread could load the real playlist. The
        // placeholder is now built directly and stays empty until then.
        let stream = Stream::new("http://example.invalid/live/index.m3u8");
        assert!(!stream.loaded);
        assert!(stream.media.segments.is_empty());
        assert_eq!(stream.media.total(), Duration::ZERO);
        assert!(stream.media.map.is_none());
        assert!(!stream.media.ended);
        assert_eq!(stream.media.sequence, 0);
        // The public entry point must also be panic-free and network-free.
        let decoder = open(&VideoSource::url("http://example.invalid/live/index.m3u8"))
            .expect("opening only remembers the address");
        assert!(!decoder.is_live());
        assert_eq!(decoder.info().width, 0);
        assert_eq!(decoder.info().height, 0);
    }

    #[test]
    fn a_fragment_key_is_its_address_and_range() {
        let a = segment(4.0);
        let mut b = segment(4.0);
        assert_eq!(FragmentKey::new(&a), FragmentKey::new(&b));
        b.uri.push('?');
        assert_ne!(FragmentKey::new(&a), FragmentKey::new(&b));
        b.uri.pop();

        let mut ranged = segment(4.0);
        ranged.range = Some(Range {
            offset: 0,
            length: 10,
        });
        assert_ne!(FragmentKey::new(&a), FragmentKey::new(&ranged));
        let mut same_range = ranged.clone();
        same_range.range = Some(Range {
            offset: 10,
            length: 10,
        });
        assert_ne!(FragmentKey::new(&ranged), FragmentKey::new(&same_range));
    }

    #[test]
    fn a_window_consumes_cached_fragments_without_the_network() {
        let mut stream = test_stream();
        let segments = stream.media.segments.clone();
        let two = &segments[..2];
        {
            let mut prefetch = stream.prefetch.lock().unwrap();
            prefetch
                .fragments
                .insert(FragmentKey::new(&two[0]), b"one".to_vec());
            prefetch
                .fragments
                .insert(FragmentKey::new(&two[1]), b"two".to_vec());
        }
        let got = stream.fetch_window(two).unwrap();
        assert_eq!(got, vec![b"one".to_vec(), b"two".to_vec()]);
        assert!(
            stream.prefetch.lock().unwrap().fragments.is_empty(),
            "consumed fragments must leave the cache"
        );
    }

    #[test]
    fn fragmented_mp4_starts_with_one_segment_and_keeps_two_afterward() {
        let mut stream = test_stream();
        stream.media.map = Some(segment(4.0));

        assert_eq!(stream.window_size(), 1);
        stream.first_window = false;
        assert_eq!(stream.window_size(), WINDOW);

        stream.first_window = true;
        stream.media.map = None;
        assert_eq!(stream.window_size(), WINDOW);
    }

    #[test]
    fn invalidating_a_prefetch_clears_bytes_and_generation() {
        let mut stream = test_stream();
        let segments = stream.media.segments.clone();
        {
            let mut prefetch = stream.prefetch.lock().unwrap();
            prefetch
                .fragments
                .insert(FragmentKey::new(&segments[0]), b"stale".to_vec());
        }
        let before = stream.prefetch.lock().unwrap().generation;
        stream.invalidate_prefetch();
        let prefetch = stream.prefetch.lock().unwrap();
        assert!(prefetch.fragments.is_empty());
        assert_eq!(prefetch.generation, before.wrapping_add(1));
    }

    #[test]
    fn a_window_at_the_end_schedules_no_prefetch() {
        let mut stream = test_stream();
        // The last segment has nothing after it, so no fetch is scheduled and
        // the generation (which would invalidate an older fetch) is untouched.
        stream.next = stream.media.segments.len() - 1;
        let before = stream.prefetch.lock().unwrap().generation;
        stream.schedule_prefetch(1);
        assert_eq!(stream.prefetch.lock().unwrap().generation, before);
    }

    #[cfg(feature = "native-audio")]
    #[test]
    fn audio_segments_line_up_by_absolute_sequence() {
        let mut stream = test_stream();
        // Video starts at sequence 10; the audio playlist starts one earlier.
        // Video index 0 must therefore map to audio segment 11, not 10.
        stream.media.sequence = 10;
        let audio_url = "http://example.invalid/audio.m3u8";
        let body = concat!(
            "#EXTM3U\n",
            "#EXT-X-VERSION:6\n",
            "#EXT-X-TARGETDURATION:4\n",
            "#EXT-X-MEDIA-SEQUENCE:9\n",
            "#EXTINF:4.0,\n",
            "a9.ts\n",
            "#EXTINF:4.0,\n",
            "a10.ts\n",
            "#EXTINF:4.0,\n",
            "a11.ts\n",
            "#EXT-X-ENDLIST\n",
        );
        stream.audio_media = Some(playlist::media(body, audio_url).unwrap());

        let (map, two) = stream
            .audio_segments_for(0, 2)
            .expect("audio segments exist");
        assert!(map.is_none());
        assert_eq!(two.len(), 2);
        assert!(two[0].uri.ends_with("a10.ts"));
        assert!(two[1].uri.ends_with("a11.ts"));

        // A window that runs past the audio playlist simply has fewer segments.
        let (_, tail) = stream
            .audio_segments_for(1, 3)
            .expect("audio segments exist");
        assert_eq!(tail.len(), 1, "only a11 is still in range");
    }

    #[test]
    fn a_finished_playlist_switches_no_rendition_at_its_end() {
        let mut stream = test_stream();
        let master = concat!(
            "#EXTM3U\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\n",
            "low.m3u8\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\n",
            "high.m3u8\n",
        );
        let variants = playlist::variants(master, "http://example.invalid/master.m3u8").unwrap();
        stream.master = Some(Master {
            variants,
            audios: Vec::new(),
            chosen: 1,
        });
        // A link that cannot carry the rendition in play: without the guard
        // this would open the last segment again from the smaller rendition.
        stream
            .prefetch
            .lock()
            .unwrap()
            .bandwidth
            .record(1_000_000.0);
        let count = stream.media.segments.len();
        assert!(
            !stream.advance(count).unwrap(),
            "the playlist is at its end"
        );
        assert_eq!(stream.next, count, "a switch must not replay the end");
        assert_eq!(stream.master.as_ref().unwrap().chosen, 1);
    }

    #[test]
    fn a_fragments_deadline_follows_its_duration() {
        // Short fragments get a floor and long ones scale to their length. A
        // broken duration falls back to the floor instead of waiting forever or
        // panicking while a Duration is built.
        assert_eq!(media_deadline(&segment(1.0)), MEDIA_DEADLINE_MIN);
        assert_eq!(media_deadline(&segment(10.0)), Duration::from_secs(40));
        assert_eq!(media_deadline(&segment(100.0)), MEDIA_DEADLINE_MAX);
        assert_eq!(media_deadline(&segment(f64::NAN)), MEDIA_DEADLINE_MIN);
        assert_eq!(media_deadline(&segment(0.0)), MEDIA_DEADLINE_MIN);
    }

    #[test]
    fn a_close_stops_the_wait_and_the_fetch_queue() {
        let mut stop = Stop::default();
        assert!(!stop.is_stopped());
        stop.observe(Arc::new(AtomicBool::new(true)));
        assert!(stop.is_stopped());

        let clear = Stop::default();
        assert!(!clear.is_stopped());

        // A signalled close wakes a wait at once instead of sleeping it out.
        let began = Instant::now();
        assert!(sleep_or_stop(&stop, Duration::from_secs(60)));
        assert!(began.elapsed() < Duration::from_secs(1));

        // No request is started once a close is signalled.
        let segments = vec![segment(4.0), segment(4.0), segment(4.0)];
        assert!(fetch_many(&segments, &stop).is_err());
        assert!(fetch_many(&[], &stop).unwrap().0.is_empty());
    }

    #[test]
    fn reads_both_bounds_of_a_content_range() {
        assert_eq!(content_range("bytes 0-1023/146515"), Some((0, 1023)));
        assert_eq!(content_range("Bytes 2048-4095/10000"), Some((2048, 4095)));
        // A range the server could not satisfy names no usable bounds.
        assert_eq!(content_range("bytes */146515"), None);
        assert_eq!(content_range("items 0-1/2"), None);
        assert_eq!(content_range("bytes 0-x/10"), None);
    }

    #[test]
    fn only_statuses_that_clear_are_retried() {
        assert!(transient_status(408));
        assert!(transient_status(429));
        assert!(transient_status(503));
        assert!(!transient_status(200));
        assert!(!transient_status(404));
        assert!(!transient_status(416));
        // [REPORT-ONLY] An expired token or a deleted rendition comes back as
        // 403/410. Retrying those burns the window and the retry budget without
        // ever succeeding: the URL has to be re-resolved, not re-requested.
        assert!(
            !transient_status(403),
            "a forbidden/expired token is permanent, not a transient retry"
        );
        assert!(!transient_status(410), "a gone rendition is permanent");
    }

    #[test]
    fn a_retry_after_is_capped() {
        assert_eq!(retry_after_limit("2"), Some(Duration::from_secs(2)));
        assert_eq!(retry_after_limit("3600"), Some(RETRY_CAP));
        assert_eq!(retry_after_limit("Wed, 21 Oct 2015 07:28:00 GMT"), None);
    }

    #[test]
    fn a_streams_target_paces_its_polls_and_its_patience() {
        assert_eq!(playlist_poll(6.0), Duration::from_secs(3));
        assert_eq!(playlist_poll(1.0), POLL_MIN);
        assert_eq!(playlist_poll(60.0), POLL_MAX);
        assert_eq!(playlist_stall(1.0), STALL_MIN);
        assert_eq!(playlist_stall(6.0), Duration::from_secs(30));
        assert_eq!(playlist_stall(60.0), STALL_MAX);
    }

    fn retryable(transient: bool, retry_after: Option<Duration>) -> Failure {
        Failure {
            error: VideoError::Source("a test failure".to_owned()),
            transient,
            retry_after,
        }
    }

    #[test]
    fn a_permanent_failure_is_not_retried() {
        let stop = Stop::default();
        // A server-asked wait must not keep a permanent failure alive: it is
        // only meaningful when another request is going to be made.
        let error =
            next_retry(retryable(false, Some(Duration::from_secs(30))), 1, &stop).unwrap_err();
        assert!(error.error.to_string().contains("a test failure"));
    }

    #[test]
    fn a_transient_failure_retries_until_its_attempt_limit() {
        let stop = Stop::default();
        assert_eq!(
            next_retry(retryable(true, Some(RETRY_CAP)), 1, &stop).unwrap(),
            RETRY_CAP
        );
        assert!(
            !next_retry(retryable(true, None), 2, &stop)
                .unwrap()
                .is_zero()
        );
        assert!(
            next_retry(retryable(true, Some(RETRY_CAP)), ATTEMPTS, &stop).is_err(),
            "the last attempt must surface its error"
        );
    }

    #[test]
    fn a_close_cancels_a_retry_before_it_waits() {
        let stop = Stop::default();
        stop.signal();
        let error =
            next_retry(retryable(true, Some(Duration::from_secs(30))), 1, &stop).unwrap_err();
        assert!(error.error.to_string().contains("playback closed"));
    }

    #[test]
    fn a_direct_close_signals_the_players_flag() {
        let mut stream = test_stream();
        let external = Arc::new(AtomicBool::new(false));
        stream.stop.observe(Arc::clone(&external));
        let worker = stream.stop.clone();
        stream.close();
        assert!(stream.stop.is_stopped());
        assert!(worker.is_stopped(), "a running fetch shares the same stop");
        assert!(external.load(Ordering::Relaxed));
    }

    #[test]
    fn an_loaded_stream_does_not_touch_the_network_again() {
        let mut stream = test_stream();
        let url = stream.url.clone();
        assert!(stream.ensure_loaded().is_ok());
        assert_eq!(stream.url, url);
        assert_eq!(stream.next, 0);
    }
}
