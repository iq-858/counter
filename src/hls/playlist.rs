//! The playlist format: the renditions a master playlist offers and the
//! segments a media playlist names.
//!
//! A playlist is lines of text. A line starting with `#` is a tag, an empty
//! line is nothing, and any other line is an address, taken against the
//! playlist's own address. The parser keeps the two kinds apart and states
//! which tags this crate cannot honour, so a stream it cannot play fails at
//! open instead of halfway.

use std::time::Duration;

use crate::VideoError;

/// The most segments a playlist may name. Real playlists stay far below this;
/// the ceiling keeps a broken or hostile list from exhausting memory.
const MAX_SEGMENTS: usize = 200_000;

/// The longest one segment may be. A day is far beyond any real segment, and
/// the ceiling keeps a broken `#EXTINF` from overflowing the timeline.
const MAX_SEGMENT: f64 = 86_400.0;

/// A byte range inside the resource a segment names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Range {
    /// The first byte of the range.
    pub(crate) offset: u64,
    /// How many bytes the range holds.
    pub(crate) length: u64,
}

impl Range {
    /// The last byte of the range, which is what an HTTP `Range` header states.
    pub(crate) fn end(&self) -> u64 {
        self.offset.saturating_add(self.length).saturating_sub(1)
    }
}

/// One partial segment (`#EXT-X-PART`) of a low-latency stream.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Part {
    /// Resolved URI of the part.
    pub(crate) uri: String,
    /// Declared duration in seconds.
    pub(crate) duration: f64,
    /// Whether this part ends the segment it belongs to.
    pub(crate) independent: bool,
}

/// The low-latency facts a media playlist declares.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LowLatency {
    /// `EXT-X-PART-INF`: the target part duration in seconds.
    pub(crate) part_target: Option<f64>,
    /// `EXT-X-SERVER-CONTROL`: the server's hold-back in seconds.
    pub(crate) hold_back: Option<f64>,
    /// `EXT-X-SERVER-CONTROL`: whether blocking playlist reload is offered.
    pub(crate) can_block_reload: bool,
    /// `EXT-X-SERVER-CONTROL`: whether delta playlist updates are offered.
    pub(crate) can_skip_dates: bool,
    /// Parts of the most recent segment, in playlist order.
    pub(crate) parts: Vec<Part>,
    /// `EXT-X-PRELOAD-HINT`: the URI the server says to fetch next.
    pub(crate) preload_hint: Option<String>,
}

/// One segment of a media playlist.
#[derive(Debug, Clone)]
pub(crate) struct Segment {
    /// Where the segment is, as an absolute address.
    pub(crate) uri: String,
    /// The part of the resource to read, when the segment is one range of it.
    pub(crate) range: Option<Range>,
    /// How long the segment plays, in seconds.
    pub(crate) duration: f64,
    /// Whether a `#EXT-X-DISCONTINUITY` precedes this segment. The media
    /// timeline restarts here, so a window must rebase this fragment afresh
    /// instead of continuing the previous fragment's decode times.
    pub(crate) discontinuity: bool,
}

/// A media playlist: the parts that make a stream and where it ends.
#[derive(Debug, Clone)]
pub(crate) struct Media {
    /// Low-latency facts, empty for a classic playlist.
    pub(crate) low_latency: LowLatency,
    /// The segments, in the order they play.
    pub(crate) segments: Vec<Segment>,
    /// The initialization segment, when the stream names one. A fragmented
    /// MP4 stream carries the codec configuration there and nowhere else.
    pub(crate) map: Option<Segment>,
    /// Whether the playlist states that no more segments follow.
    pub(crate) ended: bool,
    /// The media sequence number of the first segment, which numbers the rest
    /// even as the playlist slides forward.
    pub(crate) sequence: u64,
    /// The `#EXT-X-TARGETDURATION` the playlist states, in seconds: the longest
    /// a segment should take. A live reader paces its polls by this.
    pub(crate) target: f64,
    /// Where every segment starts, as prefix sums of their lengths. Kept beside
    /// the segments so a seek stays fast on a playlist with many thousands.
    starts: Vec<Duration>,
}

impl Media {
    /// The low-latency facts this playlist declares.
    pub(crate) fn low_latency(&self) -> &LowLatency {
        &self.low_latency
    }

    /// How long the whole playlist plays.
    pub(crate) fn total(&self) -> Duration {
        let last = self.segments.last().map_or(0.0, |segment| segment.duration);
        self.starts
            .last()
            .copied()
            .unwrap_or_default()
            .saturating_add(Duration::from_secs_f64(last))
    }

    /// Where the segment at `index` starts.
    pub(crate) fn start(&self, index: usize) -> Duration {
        self.starts
            .get(index)
            .copied()
            .unwrap_or_else(|| self.total())
    }

    /// The empty starting state before a playlist is loaded on the decode
    /// thread. This is built directly and is deliberately **not** passed through
    /// [`media`]: a real playlist with no segments is invalid input and is
    /// rejected there, but the pre-load placeholder is expected to be empty.
    /// Every reader of it tolerates an empty segment list, and every path that
    /// needs real segments calls `ensure_loaded` first.
    pub(crate) fn empty() -> Self {
        Self {
            low_latency: LowLatency::default(),
            segments: Vec::new(),
            map: None,
            ended: false,
            sequence: 0,
            target: 6.0,
            starts: Vec::new(),
        }
    }

    /// The index of the segment that holds `position`.
    pub(crate) fn index_of(&self, position: Duration) -> usize {
        // `starts` is sorted, so the segment is the last one whose start is at
        // or before the position. A position past the end lands on the last.
        let found = self.starts.partition_point(|&start| start <= position);
        found
            .saturating_sub(1)
            .min(self.segments.len().saturating_sub(1))
    }
}

/// One rendition a master playlist offers.
#[derive(Debug, Clone)]
pub(crate) struct Variant {
    /// Where its media playlist is.
    pub(crate) uri: String,
    /// The bits per second it states.
    pub(crate) bandwidth: u64,
    /// The picture height it states, when it states one.
    pub(crate) height: Option<u32>,
    /// The audio group it points at, when it points at one.
    pub(crate) audio: Option<String>,
}

/// One audio rendition a master playlist offers apart from the picture.
#[derive(Debug, Clone)]
pub(crate) struct AudioRendition {
    /// Where its media playlist is, as an absolute address.
    pub(crate) uri: String,
    /// The group it belongs to, which ties it to a video rendition.
    pub(crate) group: Option<String>,
    /// Whether the playlist names it as the one to use by default.
    pub(crate) default: bool,
}

/// Whether the playlist offers renditions rather than naming segments.
pub(crate) fn is_master(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_start().starts_with("#EXT-X-STREAM-INF:"))
}

/// The kind of resource a request is reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceKind {
    /// A playlist, which names fragments or other playlists.
    Playlist,
    /// A media fragment, which carries samples for the decoder.
    Media,
}

impl ResourceKind {
    /// A short name for diagnostics, with no address that could carry a token.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Playlist => "HLS playlist",
            Self::Media => "HLS media fragment",
        }
    }
}

/// Whether a `Content-Type` header is plausible for `kind`.
///
/// A missing or empty header is accepted: plenty of servers omit one, and the
/// playlist parser and the platform decoder decide whether the bytes are real.
/// A header that names a different kind of document, such as an HTML error page
/// served with status 200, is refused before its bytes are trusted.
pub(crate) fn accepts_content_type(kind: ResourceKind, header: Option<&str>) -> bool {
    let Some(header) = header else {
        return true;
    };
    let base = header
        .split_once(';')
        .map_or(header, |(base, _)| base)
        .trim();
    if base.is_empty() {
        return true;
    }
    match kind {
        ResourceKind::Playlist => matches!(
            base.to_ascii_lowercase().as_str(),
            "application/vnd.apple.mpegurl"
                | "application/x-mpegurl"
                | "application/mpegurl"
                | "audio/mpegurl"
                | "audio/x-mpegurl"
                | "text/m3u8"
                | "text/plain"
                | "application/octet-stream"
        ),
        ResourceKind::Media => {
            let lower = base.to_ascii_lowercase();
            lower.starts_with("video/")
                || lower.starts_with("audio/")
                || matches!(
                    lower.as_str(),
                    "application/octet-stream" | "binary/octet-stream" | "application/mp4"
                )
        }
    }
}

/// The renditions a master playlist offers, in the order it states them.
pub(crate) fn variants(text: &str, base: &str) -> Result<Vec<Variant>, VideoError> {
    let mut out = Vec::new();
    let mut pending: Option<(u64, Option<u32>, Option<String>)> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(body) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let bandwidth = attribute(body, "BANDWIDTH")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let height = attribute(body, "RESOLUTION")
                .and_then(|value| value.split_once(['x', 'X']).map(|(_, h)| h.to_owned()))
                .and_then(|height| height.parse().ok());
            let audio = attribute(body, "AUDIO");
            pending = Some((bandwidth, height, audio));
        } else if !line.is_empty() && !line.starts_with('#') {
            if let Some((bandwidth, height, audio)) = pending.take() {
                out.push(Variant {
                    uri: resolve(base, line),
                    bandwidth,
                    height,
                    audio,
                });
            }
        }
    }
    Ok(out)
}

/// The rendition to play when nothing has been measured yet: the largest one
/// that fits `max_height`, the smallest when all of them are taller, and the
/// fastest when none states a size.
///
/// A stream keeps the renditions it was offered, so it keeps the place of the
/// one it plays to move away from it once it has measured its link.
pub(crate) fn chosen_index(variants: &[Variant], max_height: u32) -> Option<usize> {
    let mut fits: Option<usize> = None;
    for (index, variant) in variants.iter().enumerate() {
        if let Some(height) = variant.height {
            if height <= max_height
                && fits.is_none_or(|best| height > variants[best].height.unwrap_or(0))
            {
                fits = Some(index);
            }
        }
    }
    if fits.is_some() {
        return fits;
    }
    if let Some((index, _)) = variants
        .iter()
        .enumerate()
        .filter(|(_, variant)| variant.height.is_some())
        .min_by_key(|(_, variant)| variant.height)
    {
        return Some(index);
    }
    variants
        .iter()
        .enumerate()
        .max_by_key(|(_, variant)| variant.bandwidth)
        .map(|(index, _)| index)
}

/// The share of the measured throughput a rendition may claim.
///
/// A playlist's `BANDWIDTH` states the average of the encoded stream. The link
/// also carries the container's overhead and the bursts inside a fragment, so a
/// rendition is only trusted when a quarter of the measured rate stays spare.
const BANDWIDTH_HEADROOM: f64 = 0.75;

/// The rendition the measured link can carry, when it differs from the one in
/// play.
///
/// A rendition that states no bandwidth cannot be measured against, so this
/// never chooses one. The stream moves up only when the taller rendition fits
/// inside the headroom, and down as soon as the one in play does not fit at
/// all: a fragment that cannot arrive before it is due is heard as a gap and
/// seen as a freeze, which a smaller picture avoids.
pub(crate) fn choose_for_bandwidth(
    variants: &[Variant],
    bits_per_second: f64,
    current: usize,
) -> Option<usize> {
    if variants.len() < 2 || !bits_per_second.is_finite() || bits_per_second <= 0.0 {
        return None;
    }
    let current_bandwidth = variants.get(current)?.bandwidth;
    if current_bandwidth == 0 {
        return None;
    }
    let affordable = bits_per_second * BANDWIDTH_HEADROOM;
    let best = |limit: f64| {
        variants
            .iter()
            .enumerate()
            .filter(|(_, variant)| variant.bandwidth > 0 && variant.bandwidth as f64 <= limit)
            .max_by_key(|(_, variant)| variant.bandwidth)
            .map(|(index, _)| index)
    };
    let candidate = best(affordable).or_else(|| {
        // Nothing measured fits. The cheapest rendition is the only one that
        // can keep up, and a smaller picture beats a stream that stalls.
        variants
            .iter()
            .enumerate()
            .filter(|(_, variant)| variant.bandwidth > 0)
            .min_by_key(|(_, variant)| variant.bandwidth)
            .map(|(index, _)| index)
    })?;
    if candidate == current {
        return None;
    }
    // A rendition that still fits keeps its place, so a measurement wobbling
    // around the top of its range does not flicker between renditions.
    let fits = current_bandwidth as f64 <= affordable;
    if fits && variants[candidate].bandwidth <= current_bandwidth {
        return None;
    }
    Some(candidate)
}

/// The audio renditions a master playlist offers apart from the picture.
///
/// A rendition that names no address cannot be opened on its own: its samples
/// are carried inside the video segments instead. Those are skipped here, so a
/// playlist that muxes audio into the picture still plays through the joined
/// video window.
pub(crate) fn audios(text: &str, base: &str) -> Vec<AudioRendition> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(body) = line.strip_prefix("#EXT-X-MEDIA:") else {
            continue;
        };
        if !attribute(body, "TYPE").is_some_and(|kind| kind.eq_ignore_ascii_case("AUDIO")) {
            continue;
        }
        let Some(uri) = attribute(body, "URI") else {
            continue;
        };
        out.push(AudioRendition {
            uri: resolve(base, &uri),
            group: attribute(body, "GROUP-ID"),
            default: attribute(body, "DEFAULT")
                .is_some_and(|value| value.eq_ignore_ascii_case("YES")),
        });
    }
    out
}

/// The audio rendition that belongs to `variant`: the one in its group that the
/// playlist marks default, or the first in that group, or the first default.
pub(crate) fn choose_audio<'a>(
    audios: &'a [AudioRendition],
    variant: &Variant,
) -> Option<&'a AudioRendition> {
    let fits = |audio: &&AudioRendition| {
        variant
            .audio
            .as_deref()
            .is_none_or(|group| audio.group.as_deref() == Some(group))
    };
    audios
        .iter()
        .filter(fits)
        .find(|audio| audio.default)
        .or_else(|| audios.iter().find(fits))
        .or_else(|| audios.iter().find(|audio| audio.default))
}

/// Reads a media playlist.
pub(crate) fn media(text: &str, base: &str) -> Result<Media, VideoError> {
    if !text.trim_start().starts_with("#EXTM3U") {
        return Err(VideoError::Source(
            "the HLS playlist does not start with #EXTM3U".to_owned(),
        ));
    }
    let mut segments = Vec::new();
    let mut map = None;
    let mut ended = false;
    let mut sequence = 0u64;
    let mut target = 6.0f64;
    let mut duration: Option<f64> = None;
    let mut range: Option<(u64, Option<u64>)> = None;
    let mut previous: Option<(String, u64)> = None;
    let mut discontinuity = false;
    let mut low_latency = LowLatency::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(body) = line.strip_prefix("#EXTINF:") {
            duration = Some(seconds(body)?);
        } else if let Some(body) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            range = Some(sub_range(body)?);
        } else if let Some(body) = line.strip_prefix("#EXT-X-MAP:") {
            let uri = attribute(body, "URI").ok_or_else(|| {
                VideoError::Source("the playlist's init section states no URI".to_owned())
            })?;
            let range = attribute(body, "BYTERANGE")
                .and_then(|value| sub_range(&value).ok())
                .map(|(length, offset)| Range {
                    offset: offset.unwrap_or(0),
                    length,
                });
            map = Some(Segment {
                uri: resolve(base, &uri),
                range,
                duration: 0.0,
                discontinuity: false,
            });
        } else if line == "#EXT-X-ENDLIST" {
            ended = true;
        } else if line == "#EXT-X-DISCONTINUITY" {
            // The tag applies to the next segment. A byte range cannot carry on
            // across a discontinuity, so the previous range's end is forgotten.
            discontinuity = true;
            previous = None;
        } else if let Some(body) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            // A malformed number must not renumber the stream from zero: keep
            // the last valid value (or the zero default) instead.
            if let Ok(value) = body.trim().parse::<u64>() {
                sequence = value;
            }
        } else if let Some(body) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            // A target the reader cannot use is ignored, keeping the default,
            // rather than rejecting a stream whose segments are readable.
            if let Ok(value) = body.trim().parse::<f64>() {
                if value.is_finite() && value > 0.0 && value <= MAX_SEGMENT {
                    target = value;
                }
            }
        } else if let Some(body) = line
            .strip_prefix("#EXT-X-KEY:")
            .or_else(|| line.strip_prefix("#EXT-X-SESSION-KEY:"))
        {
            check_key(body)?;
        } else if line.starts_with("#EXT-X-STREAM-INF:") {
            return Err(VideoError::Source(
                "the address names a master playlist, not a media playlist".to_owned(),
            ));
        } else if let Some(body) = line.strip_prefix("#EXT-X-PART-INF:") {
            if let Ok(value) = body.trim().parse::<f64>() {
                if value.is_finite() && value > 0.0 {
                    low_latency.part_target = Some(value);
                }
            }
        } else if let Some(body) = line.strip_prefix("#EXT-X-SERVER-CONTROL:") {
            if let Some(value) = attribute(body, "HOLD-BACK")
                .and_then(|v| v.parse::<f64>().ok())
            {
                low_latency.hold_back = Some(value);
            }
            low_latency.can_block_reload =
                attribute(body, "CAN-BLOCK-RELOAD").as_deref() == Some("YES");
            low_latency.can_skip_dates =
                attribute(body, "CAN-SKIP-UNTIL").is_some();
        } else if let Some(body) = line.strip_prefix("#EXT-X-PART:") {
            if let Some(uri) = attribute(body, "URI") {
                low_latency.parts.push(Part {
                    uri: resolve(base, &uri),
                    duration: attribute(body, "DURATION")
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0),
                    independent: attribute(body, "INDEPENDENT").as_deref() == Some("YES"),
                });
            }
        } else if let Some(body) = line.strip_prefix("#EXT-X-PRELOAD-HINT:") {
            low_latency.preload_hint = attribute(body, "URI").map(|uri| resolve(base, &uri));
        } else if line.starts_with('#') {
            // Tags this reader ignores state where a segment sits in the
            // stream or how a player should treat the start of it, neither of
            // which changes how the bytes are read.
        } else {
            if segments.len() >= MAX_SEGMENTS {
                return Err(VideoError::Source(format!(
                    "the HLS playlist names more than {MAX_SEGMENTS} segments"
                )));
            }
            let uri = resolve(base, line);
            let range = range.take().map(|(length, offset)| Range {
                offset: offset.unwrap_or_else(|| match &previous {
                    Some((last, end)) if *last == uri => *end,
                    _ => 0,
                }),
                length,
            });
            if let Some(range) = range {
                previous = Some((uri.clone(), range.offset.saturating_add(range.length)));
            } else {
                previous = None;
            }
            segments.push(Segment {
                uri,
                range,
                duration: duration.take().unwrap_or(target),
                discontinuity,
            });
            discontinuity = false;
        }
    }
    if segments.is_empty() {
        return Err(VideoError::Source(
            "the HLS playlist names no segments".to_owned(),
        ));
    }
    let mut starts = Vec::with_capacity(segments.len());
    let mut elapsed = 0.0f64;
    for segment in &segments {
        starts.push(Duration::from_secs_f64(elapsed));
        elapsed += segment.duration;
    }
    Ok(Media {
        low_latency,
        segments,
        map,
        ended,
        sequence,
        target,
        starts,
    })
}

/// Fails for a stream this crate cannot decrypt.
fn check_key(body: &str) -> Result<(), VideoError> {
    match attribute(body, "METHOD").as_deref() {
        None | Some("NONE") => Ok(()),
        Some(method) => Err(VideoError::Source(format!(
            "the stream is encrypted with {method}, which this crate does not decrypt"
        ))),
    }
}

/// The seconds an `#EXTINF` line states, before its title.
fn seconds(body: &str) -> Result<f64, VideoError> {
    let text = body.split(',').next().unwrap_or(body).trim();
    let value: f64 = text.parse().map_err(|_| {
        VideoError::Source(format!("the playlist states no segment length in {text:?}"))
    })?;
    if !value.is_finite() || value < 0.0 || value > MAX_SEGMENT {
        return Err(VideoError::Source(format!(
            "the playlist states an unusable segment length in {text:?}"
        )));
    }
    Ok(value)
}

/// The length and offset an `#EXT-X-BYTERANGE` line states.
fn sub_range(body: &str) -> Result<(u64, Option<u64>), VideoError> {
    let text = body.trim().trim_matches('"');
    let invalid = || VideoError::Source(format!("the playlist states no byte range in {text:?}"));
    let (length, offset) = match text.split_once('@') {
        Some((length, offset)) => (
            length.trim().parse::<u64>().map_err(|_| invalid())?,
            Some(offset.trim().parse::<u64>().map_err(|_| invalid())?),
        ),
        None => (text.parse::<u64>().map_err(|_| invalid())?, None),
    };
    // A zero-length range names no bytes; fetching it would produce an empty
    // fragment that fails to decode with a confusing error deep in the decoder.
    if length == 0 {
        return Err(invalid());
    }
    Ok((length, offset))
}

/// One attribute's value, with the quotes the format wraps it in removed.
fn attribute(body: &str, name: &str) -> Option<String> {
    for item in split(body) {
        if let Some((key, value)) = item.split_once('=') {
            if key.trim().eq_ignore_ascii_case(name) {
                return Some(value.trim().trim_matches('"').to_owned());
            }
        }
    }
    None
}

/// Splits an attribute list on the commas outside quotes, which is what keeps
/// a quoted value whole.
fn split(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (index, character) in body.char_indices() {
        match character {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                out.push(&body[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    out.push(&body[start..]);
    out
}

/// Whether a reference is an absolute URI rather than a relative address.
///
/// The old `contains("://")` test could mistake a relative address whose query
/// happens to hold `://` for an absolute one. A URI scheme is a leading letter
/// followed by letters, digits or `+`, `-`, `.`, then a colon. A single letter
/// is a Windows drive path, not a scheme.
fn has_scheme(reference: &str) -> bool {
    let bytes = reference.as_bytes();
    let Some(&first) = bytes.first() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    let Some(colon) = bytes.iter().position(|&byte| byte == b':') else {
        return false;
    };
    if colon <= 1 {
        return false;
    }
    bytes[1..colon]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

/// An address taken against the address that carries it.
pub(crate) fn resolve(base: &str, reference: &str) -> String {
    let reference = reference.trim();
    if has_scheme(reference) {
        return reference.to_owned();
    }
    // A reference that is only a query or fragment replaces the base's own
    // suffix but keeps its document path, as a browser would.
    if reference.starts_with(['?', '#']) {
        let (path, _) = split_suffix(base);
        return format!("{path}{reference}");
    }
    let Some(scheme) = base.split_once("://").map(|(scheme, _)| scheme) else {
        return reference.to_owned();
    };
    if reference.starts_with("//") {
        return format!("{scheme}:{reference}");
    }
    let Some(origin) = origin_end(base) else {
        return reference.to_owned();
    };
    if reference.starts_with('/') {
        return normalize(&format!("{}{reference}", &base[..origin]));
    }
    // A relative address replaces the last part of the base's path.
    let (path, _) = split_suffix(base);
    let directory = match path[origin..].rfind('/') {
        Some(index) => &path[..origin + index + 1],
        None => &path[..origin],
    };
    let mut resolved = String::with_capacity(directory.len() + reference.len() + 1);
    resolved.push_str(directory);
    if !resolved.ends_with('/') {
        resolved.push('/');
    }
    resolved.push_str(reference);
    normalize(&resolved)
}

/// The index just past an address's scheme and host.
fn origin_end(url: &str) -> Option<usize> {
    let scheme = url.find("://")?;
    let after = scheme + 3;
    Some(
        url[after..]
            .find(['/', '?', '#'])
            .map(|index| after + index)
            .unwrap_or(url.len()),
    )
}

/// Splits an address into the part before its query and fragment and the rest.
fn split_suffix(url: &str) -> (&str, &str) {
    match url.find(['?', '#']) {
        Some(index) => (&url[..index], &url[index..]),
        None => (url, ""),
    }
}

/// Removes the `.` and `..` parts of an address's path, which a server would
/// otherwise have to resolve.
fn normalize(url: &str) -> String {
    let Some(scheme) = url.find("://") else {
        return url.to_owned();
    };
    let after = scheme + 3;
    let authority = url[after..]
        .find(['/', '?', '#'])
        .map(|index| after + index)
        .unwrap_or(url.len());
    let (path, suffix) = split_suffix(&url[authority..]);
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let mut out = String::with_capacity(url.len());
    out.push_str(&url[..authority]);
    if path.starts_with('/') {
        out.push('/');
    }
    out.push_str(&parts.join("/"));
    out.push_str(suffix);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rendition the initial choice names, for the tests that read its
    /// fields.
    fn chosen(variants: &[Variant], max_height: u32) -> &Variant {
        &variants[chosen_index(variants, max_height).expect("a rendition is chosen")]
    }

    const MEDIA: &str = "#EXTM3U\n\
#EXT-X-VERSION:6\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:52216\n\
#EXT-X-MAP:URI=\"init.hls.fmp4\"\n\
#EXT-X-PROGRAM-DATE-TIME:2026-11-16T09:00:00.000Z\n\
#EXTINF:6.000,\n\
seg-0-52216.hls.fmp4\n\
#EXTINF:6.000,\n\
https://other.example/seg-1.hls.fmp4\n\
#EXTINF:3.5,title\n\
seg-2.hls.fmp4\n";

    const MASTER: &str = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=1190000,RESOLUTION=640x360,CODECS=\"avc1.4d401f,mp4a.40.2\"\n\
tracks-v3/index.fmp4.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=4530000,RESOLUTION=1920x1080\n\
tracks-v1/index.fmp4.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=2270000,RESOLUTION=852x480\n\
tracks-v2/index.fmp4.m3u8\n";

    #[test]
    fn reads_a_media_playlist() {
        let media = media(MEDIA, "https://host/a/b/index.fmp4.m3u8").unwrap();
        assert_eq!(media.sequence, 52216);
        assert!(!media.ended);
        assert_eq!(media.segments.len(), 3);
        assert_eq!(
            media.map.as_ref().unwrap().uri,
            "https://host/a/b/init.hls.fmp4"
        );
        assert_eq!(
            media.segments[0].uri,
            "https://host/a/b/seg-0-52216.hls.fmp4"
        );
        assert_eq!(
            media.segments[1].uri,
            "https://other.example/seg-1.hls.fmp4"
        );
        assert_eq!(media.segments[2].duration, 3.5);
        assert_eq!(media.target, 6.0);
        assert_eq!(media.total(), Duration::from_secs_f64(15.5));
        assert_eq!(media.start(2), Duration::from_secs(12));
        assert_eq!(media.index_of(Duration::from_secs(7)), 1);
        assert_eq!(media.index_of(Duration::from_secs(99)), 2);
        // The start of a segment belongs to that segment, and a position before
        // the first segment still lands on the first.
        assert_eq!(media.index_of(Duration::from_secs(6)), 1);
        assert_eq!(media.index_of(Duration::ZERO), 0);
        assert_eq!(media.start(9), Duration::from_secs_f64(15.5));
    }

    #[test]
    fn a_bad_target_duration_keeps_the_default_pace() {
        let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:not-a-number\n\
#EXTINF:2,\n\
a.ts\n\
#EXTINF:3,\n\
b.ts\n";
        let media = media(text, "https://host/index.m3u8").unwrap();
        // The fallback stays usable, and a missing `#EXTINF` still gets a length.
        assert_eq!(media.target, 6.0);
        assert_eq!(media.segments[0].duration, 2.0);
        assert_eq!(media.segments[1].duration, 3.0);
    }

    #[test]
    fn a_bad_segment_length_is_refused() {
        for length in ["-1", "NaN", "inf", "1e12", "later"] {
            let text = format!("#EXTM3U\n#EXTINF:{length},\na.ts\n");
            assert!(
                media(&text, "https://host/index.m3u8").is_err(),
                "{length:?} should be refused"
            );
        }
    }

    #[test]
    fn a_playlist_longer_than_the_ceiling_is_refused() {
        let mut text = String::from("#EXTM3U\n");
        for index in 0..=MAX_SEGMENTS {
            text.push_str("#EXTINF:1,\n");
            text.push_str(&format!("s{index}.ts\n"));
        }
        assert!(media(&text, "https://host/index.m3u8").is_err());
    }

    #[test]
    fn reads_a_master_playlist() {
        assert!(is_master(MASTER));
        let variants = variants(MASTER, "https://host/index.fmp4.m3u8").unwrap();
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].height, Some(360));
        assert_eq!(variants[0].uri, "https://host/tracks-v3/index.fmp4.m3u8");
        assert_eq!(chosen(&variants, 1080).height, Some(1080));
        assert_eq!(chosen(&variants, 720).height, Some(480));
        assert_eq!(chosen(&variants, 200).height, Some(360));
    }

    #[test]
    fn a_master_playlist_is_not_a_media_playlist() {
        assert!(media(MASTER, "https://host/index.m3u8").is_err());
    }

    #[test]
    fn a_content_type_matches_the_resource_it_names() {
        assert!(accepts_content_type(
            ResourceKind::Playlist,
            Some("application/vnd.apple.mpegURL; charset=utf-8")
        ));
        assert!(accepts_content_type(
            ResourceKind::Playlist,
            Some("text/plain")
        ));
        assert!(!accepts_content_type(
            ResourceKind::Playlist,
            Some("text/html")
        ));
        assert!(accepts_content_type(
            ResourceKind::Media,
            Some("video/mp2t")
        ));
        assert!(accepts_content_type(
            ResourceKind::Media,
            Some("audio/mp4; codecs=\"mp4a.40.2\"")
        ));
        assert!(!accepts_content_type(
            ResourceKind::Media,
            Some("application/json")
        ));
        // A missing header states nothing, so the bytes are left to the parser
        // and the decoder rather than rejected on an absent hint.
        assert!(accepts_content_type(ResourceKind::Playlist, None));
        assert!(accepts_content_type(ResourceKind::Media, Some("")));
    }

    #[test]
    fn a_measured_link_chooses_the_rendition_it_can_carry() {
        let text = concat!(
            "#EXTM3U\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\n",
            "low.m3u8\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720\n",
            "mid.m3u8\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\n",
            "high.m3u8\n",
        );
        let variants = variants(text, "https://host/master.m3u8").unwrap();

        // Five megabits measured cannot carry the five megabit rendition: the
        // headroom for the container and its bursts is what the choice keeps.
        assert_eq!(choose_for_bandwidth(&variants, 5_000_000.0, 2), Some(1));
        // With room to spare the stream moves up to the tallest one.
        assert_eq!(choose_for_bandwidth(&variants, 10_000_000.0, 1), Some(2));
        // A rendition that still fits stays where it is.
        assert_eq!(choose_for_bandwidth(&variants, 4_000_000.0, 1), None);
        // A link that grew beyond the smallest rendition moves up.
        assert_eq!(choose_for_bandwidth(&variants, 4_000_000.0, 0), Some(1));
        // A link that cannot carry the middle one drops to the smallest, and
        // the smallest is already as low as the playlist goes.
        assert_eq!(choose_for_bandwidth(&variants, 1_000_000.0, 1), Some(0));
        assert_eq!(choose_for_bandwidth(&variants, 100_000.0, 0), None);

        // Nothing measured, or nothing to choose between, states no answer.
        assert_eq!(choose_for_bandwidth(&variants, 0.0, 0), None);
        assert_eq!(choose_for_bandwidth(&variants, f64::NAN, 0), None);
        assert_eq!(choose_for_bandwidth(&variants[..1], 9_000_000.0, 0), None);
    }

    #[test]
    fn reads_a_4k_master_playlist() {
        const MASTER_4K: &str = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=813664,RESOLUTION=640x360\n\
low/index.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=4530000,RESOLUTION=3840x2160,CODECS=\"avc1.640033,mp4a.40.2\"\n\
uhd/index.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=2270000,RESOLUTION=1920x1080\n\
hd/index.m3u8\n";
        let variants = variants(MASTER_4K, "https://host/index.m3u8").unwrap();
        assert_eq!(variants.len(), 3);
        // With no fixed ceiling the tallest rendition wins, so 4K is playable.
        assert_eq!(chosen(&variants, u32::MAX).height, Some(2160));
        assert_eq!(
            chosen(&variants, u32::MAX).uri,
            "https://host/uhd/index.m3u8"
        );
    }

    const MASTER_WITH_AUDIO: &str = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"stereo\",DEFAULT=YES,URI=\"tracks-a1/index.m3u8\"\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"commentary\",URI=\"tracks-a2/index.m3u8\"\n\
#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"en\",DEFAULT=YES,URI=\"subs/en.m3u8\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=720000,AUDIO=\"aac\",RESOLUTION=640x360\n\
tracks-v3/index.m3u8\n";

    #[test]
    fn reads_audio_renditions() {
        let audios = audios(MASTER_WITH_AUDIO, "https://host/index.m3u8");
        // Only TYPE=AUDIO entries that name a playlist are offered.
        assert_eq!(audios.len(), 2);
        assert_eq!(audios[0].uri, "https://host/tracks-a1/index.m3u8");
        assert_eq!(audios[0].group.as_deref(), Some("aac"));
        assert!(audios[0].default);
        assert!(!audios[1].default);

        let variants = variants(MASTER_WITH_AUDIO, "https://host/index.m3u8").unwrap();
        assert_eq!(variants[0].audio.as_deref(), Some("aac"));
        // The default of the group the video points at wins.
        assert_eq!(
            choose_audio(&audios, &variants[0]).unwrap().uri,
            "https://host/tracks-a1/index.m3u8"
        );
    }

    #[test]
    fn falls_back_when_no_audio_group_matches() {
        let text = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"other\",NAME=\"a\",DEFAULT=YES,URI=\"a.m3u8\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=100,AUDIO=\"missing\"\n\
v.m3u8\n";
        let audios = audios(text, "https://host/index.m3u8");
        let variants = variants(text, "https://host/index.m3u8").unwrap();
        // No audio in the video's group, so the only rendition is used.
        assert_eq!(
            choose_audio(&audios, &variants[0]).unwrap().uri,
            "https://host/a.m3u8"
        );
    }

    #[test]
    fn reads_byte_ranges() {
        let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:4\n\
#EXTINF:4,\n\
#EXT-X-BYTERANGE:1000@0\n\
all.mp4\n\
#EXTINF:4,\n\
#EXT-X-BYTERANGE:500\n\
all.mp4\n\
#EXTINF:4,\n\
#EXT-X-BYTERANGE:250@4000\n\
all.mp4\n";
        let media = media(text, "https://host/playlist.m3u8").unwrap();
        assert_eq!(
            media.segments[0].range,
            Some(Range {
                offset: 0,
                length: 1000
            })
        );
        // A range with no offset follows the previous range of the same file.
        assert_eq!(
            media.segments[1].range,
            Some(Range {
                offset: 1000,
                length: 500
            })
        );
        assert_eq!(
            media.segments[2].range,
            Some(Range {
                offset: 4000,
                length: 250
            })
        );
    }

    #[test]
    fn refuses_an_encrypted_stream() {
        let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:4\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n\
#EXTINF:4,\n\
seg.mp4\n";
        assert!(media(text, "https://host/playlist.m3u8").is_err());
    }

    #[test]
    fn a_none_key_is_not_encryption() {
        let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:4\n\
#EXT-X-KEY:METHOD=NONE\n\
#EXTINF:4,\n\
seg.mp4\n";
        assert!(media(text, "https://host/playlist.m3u8").is_ok());
    }

    #[test]
    fn resolves_addresses_the_way_a_browser_does() {
        let base = "https://host/a/b/index.m3u8?token=1";
        assert_eq!(resolve(base, "seg.mp4"), "https://host/a/b/seg.mp4");
        assert_eq!(resolve(base, "./seg.mp4"), "https://host/a/b/seg.mp4");
        assert_eq!(resolve(base, "../seg.mp4"), "https://host/a/seg.mp4");
        assert_eq!(resolve(base, "/seg.mp4"), "https://host/seg.mp4");
        assert_eq!(resolve(base, "//cdn/seg.mp4"), "https://cdn/seg.mp4");
        assert_eq!(resolve(base, "http://cdn/seg.mp4"), "http://cdn/seg.mp4");
        assert_eq!(resolve("https://host", "seg.mp4"), "https://host/seg.mp4");
        assert_eq!(
            resolve(base, "seg.mp4?part=2"),
            "https://host/a/b/seg.mp4?part=2"
        );
    }

    #[test]
    fn a_query_or_fragment_only_reference_keeps_the_document_path() {
        let base = "https://host/a/b/index.m3u8?old=1#frag";
        assert_eq!(resolve(base, "?new=2"), "https://host/a/b/index.m3u8?new=2");
        assert_eq!(resolve(base, "#part"), "https://host/a/b/index.m3u8#part");
    }

    #[test]
    fn a_colon_in_a_relative_address_is_not_a_scheme() {
        // A query may legitimately contain "://"; it must not be mistaken for
        // an absolute address. A single-letter scheme is a Windows drive path.
        assert!(!has_scheme("seg.mp4?next=https://host/x"));
        assert!(!has_scheme("C:\\videos\\seg.ts"));
        assert!(has_scheme("https://host/seg.ts"));
        assert_eq!(
            resolve("https://host/a/index.m3u8", "seg.mp4?next=https://x/y"),
            "https://host/a/seg.mp4?next=https://x/y"
        );
    }

    #[test]
    fn a_discontinuity_marks_the_next_segment_and_breaks_range_continuation() {
        let text = "#EXTM3U\n\
        #EXT-X-TARGETDURATION:4\n\
        #EXTINF:4,\n\
        #EXT-X-BYTERANGE:1000@0\n\
        all.mp4\n\
        #EXT-X-DISCONTINUITY\n\
        #EXTINF:4,\n\
        #EXT-X-BYTERANGE:500\n\
        all.mp4\n";
        let media = media(text, "https://host/playlist.m3u8").unwrap();
        assert!(!media.segments[0].discontinuity);
        assert!(media.segments[1].discontinuity);
        // With the continuation broken, an offset-less range starts at zero
        // rather than at the previous range's end.
        assert_eq!(media.segments[1].range.unwrap().offset, 0);
    }

    #[test]
    fn a_zero_length_byte_range_is_refused() {
        for range in ["0", "0@10", "0@18446744073709551615"] {
            let text = format!("#EXTM3U\n#EXTINF:4,\n#EXT-X-BYTERANGE:{range}\nseg.mp4\n");
            assert!(
                media(&text, "https://host/playlist.m3u8").is_err(),
                "{range:?} should be refused"
            );
        }
    }

    #[test]
    fn a_malformed_media_sequence_keeps_the_last_valid_number() {
        let text = "#EXTM3U\n\
        #EXT-X-MEDIA-SEQUENCE:52216\n\
        #EXT-X-MEDIA-SEQUENCE:not-a-number\n\
        #EXTINF:4,\n\
        seg.mp4\n";
        let media = media(text, "https://host/playlist.m3u8").unwrap();
        assert_eq!(media.sequence, 52216);
    }

    #[test]
    fn an_overflowing_range_continuation_saturates() {
        let text = "#EXTM3U\n\
        #EXTINF:4,\n\
        #EXT-X-BYTERANGE:18446744073709551615@0\n\
        all.mp4\n\
        #EXTINF:4,\n\
        #EXT-X-BYTERANGE:10\n\
        all.mp4\n";
        // The playlist still parses; the continuation's end saturates instead
        // of overflowing and panicking in debug builds.
        let media = media(text, "https://host/playlist.m3u8").unwrap();
        assert_eq!(media.segments.len(), 2);
    }
}

    #[test]
    fn parses_low_latency_tags() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-PART-INF:0.5\n#EXT-X-SERVER-CONTROL:HOLD-BACK=3.0,CAN-BLOCK-RELOAD=YES,CAN-SKIP-UNTIL=12.0\n#EXT-X-PART:DURATION=0.5,URI=part1.mp4,INDEPENDENT=YES\n#EXT-X-PART:DURATION=0.4,URI=part2.mp4\n#EXT-X-PRELOAD-HINT:URI=part3.mp4\n#EXTINF:4.0,\nseg1.ts\n#EXT-X-ENDLIST\n";
        let base = "https://cdn/live/stream.m3u8";
        let media = media(text, base).expect("a low-latency playlist");
        assert_eq!(media.low_latency.part_target, Some(0.5));
        assert_eq!(media.low_latency.hold_back, Some(3.0));
        assert!(media.low_latency.can_block_reload);
        assert!(media.low_latency.can_skip_dates);
        assert_eq!(media.low_latency.parts.len(), 2);
        assert!(media.low_latency.parts[0].independent);
        assert!(!media.low_latency.parts[1].independent);
        assert_eq!(
            media.low_latency.parts[0].uri,
            "https://cdn/live/part1.mp4"
        );
        assert_eq!(
            media.low_latency.preload_hint.as_deref(),
            Some("https://cdn/live/part3.mp4")
        );
        // The classic fields still parse alongside the new ones.
        assert_eq!(media.segments.len(), 1);
    }

    #[test]
    fn a_classic_playlist_has_no_low_latency_facts() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\nseg.ts\n#EXT-X-ENDLIST\n";
        let media = media(text, "https://cdn/a.m3u8").expect("a classic playlist");
        assert_eq!(media.low_latency, LowLatency::default());
    }
