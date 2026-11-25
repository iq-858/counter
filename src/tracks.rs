//! Track selection: which audio and subtitle stream a player reads.
//!
//! A container can carry several audio tracks (language dubs, a commentary, an
//! audio-description track) and several subtitle tracks. A player that can only
//! take the first one is unusable for anything but the simplest file, so the
//! engine exposes the list and a selection instead.
//!
//! Pure Rust and platform-free: a backend fills a [`TrackList`] from its own
//! demuxer and honours the index it is given. Nothing here touches a decoder,
//! so the selection rules are testable on their own.

use std::fmt;

/// What a track carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TrackKind {
    /// A sound track, which is the default a list starts with.
    #[default]
    Audio,
    /// A timed-text track.
    Subtitle,
}

impl TrackKind {
    /// The kind as a person reads it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Subtitle => "subtitle",
        }
    }
}

impl fmt::Display for TrackKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One selectable stream, as the container describes it.
///
/// Every field is optional in practice: a container may state no language, no
/// title and no codec. The index is the backend's own and is what
/// [`VideoPlayer::set_audio_track`](crate::VideoPlayer::set_audio_track) takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInfo {
    /// The backend's index for this stream.
    pub index: usize,
    /// What the stream carries.
    pub kind: TrackKind,
    /// The BCP-47 language tag the container states, such as `"en"` or
    /// `"ar"`, when it states one.
    pub language: Option<String>,
    /// The human-readable title the container states, such as
    /// `"Director's commentary"`.
    pub label: Option<String>,
    /// The codec the container names, such as `"aac"` or `"subrip"`.
    pub codec: Option<String>,
    /// Channel count for an audio track.
    pub channels: Option<u16>,
    /// Whether the container marks this track as the default one.
    pub default: bool,
    /// Whether the track is forced (signs and foreign dialogue only).
    pub forced: bool,
}

impl TrackInfo {
    /// A track with only its index and kind set.
    pub fn new(index: usize, kind: TrackKind) -> Self {
        Self {
            index,
            kind,
            language: None,
            label: None,
            codec: None,
            channels: None,
            default: false,
            forced: false,
        }
    }

    /// Sets the language tag.
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Sets the title.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the codec name.
    pub fn with_codec(mut self, codec: impl Into<String>) -> Self {
        self.codec = Some(codec.into());
        self
    }

    /// Sets the channel count.
    pub fn with_channels(mut self, channels: u16) -> Self {
        self.channels = Some(channels);
        self
    }

    /// Marks the track as the container's default.
    pub fn as_default(mut self) -> Self {
        self.default = true;
        self
    }

    /// Marks the track as forced.
    pub fn as_forced(mut self) -> Self {
        self.forced = true;
        self
    }

    /// Whether the track's language matches `tag`, comparing the primary
    /// subtag so `"en"` matches `"en-GB"`.
    pub fn speaks(&self, tag: &str) -> bool {
        let Some(language) = self.language.as_deref() else {
            return false;
        };
        let primary = |value: &str| {
            value
                .split(['-', '_'])
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
        };
        let wanted = primary(tag);
        !wanted.is_empty() && primary(language) == wanted
    }

    /// A short description for a menu or a log line.
    pub fn describe(&self) -> String {
        let mut parts = vec![format!("{} #{}", self.kind, self.index)];
        if let Some(language) = &self.language {
            parts.push(language.clone());
        }
        if let Some(label) = &self.label {
            parts.push(label.clone());
        }
        if let Some(codec) = &self.codec {
            parts.push(codec.clone());
        }
        if let Some(channels) = self.channels {
            parts.push(format!("{channels}ch"));
        }
        if self.forced {
            parts.push("forced".into());
        }
        if self.default {
            parts.push("default".into());
        }
        parts.join(", ")
    }
}

/// The tracks of one kind that a source carries, and which one is selected.
///
/// The list is ordered as the container states it. Selection is by index, and
/// `None` means the kind is off (no sound, or no subtitles shown).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackList {
    kind: TrackKind,
    tracks: Vec<TrackInfo>,
    selected: Option<usize>,
}

impl TrackList {
    /// An empty list for `kind`.
    pub fn new(kind: TrackKind) -> Self {
        Self {
            kind,
            tracks: Vec::new(),
            selected: None,
        }
    }

    /// What the list holds.
    pub fn kind(&self) -> TrackKind {
        self.kind
    }

    /// Adds a track. A track of another kind is ignored.
    pub fn push(&mut self, track: TrackInfo) {
        if track.kind == self.kind {
            self.tracks.push(track);
        }
    }

    /// Every track, in container order.
    pub fn tracks(&self) -> &[TrackInfo] {
        &self.tracks
    }

    /// How many tracks the list holds.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Whether the source carries no track of this kind.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// The selected index, or `None` when the kind is off.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The selected track, when one is selected.
    pub fn selected_track(&self) -> Option<&TrackInfo> {
        let index = self.selected?;
        self.tracks.iter().find(|track| track.index == index)
    }

    /// Whether `index` is one of the tracks.
    pub fn contains(&self, index: usize) -> bool {
        self.tracks.iter().any(|track| track.index == index)
    }

    /// Selects `index`, or turns the kind off with `None`.
    ///
    /// Returns `false` when the index is not in the list, leaving the selection
    /// as it was.
    pub fn select(&mut self, index: Option<usize>) -> bool {
        match index {
            None => {
                self.selected = None;
                true
            }
            Some(index) if self.contains(index) => {
                self.selected = Some(index);
                true
            }
            Some(_) => false,
        }
    }

    /// Selects the first track that speaks `tag`, preferring a forced track
    /// only when `forced_only` is set.
    ///
    /// Returns the index it selected, or `None` when nothing matched.
    pub fn select_language(&mut self, tag: &str, forced_only: bool) -> Option<usize> {
        let found = self
            .tracks
            .iter()
            .find(|track| track.speaks(tag) && (!forced_only || track.forced))
            .map(|track| track.index)?;
        self.selected = Some(found);
        Some(found)
    }

    /// The index the player should start on: the container's default track,
    /// else the first one, else `None` for an empty list.
    ///
    /// A subtitle list starts off, because showing subtitles unasked is not
    /// what a viewer expects; use [`default_on`](Self::default_on) for the
    /// other rule.
    pub fn default_index(&self) -> Option<usize> {
        self.tracks
            .iter()
            .find(|track| track.default)
            .or_else(|| self.tracks.first())
            .map(|track| track.index)
    }

    /// Like [`default_index`](Self::default_index), but an empty list stays
    /// off and a non-empty one always selects something.
    pub fn default_on(&self) -> Option<usize> {
        self.default_index()
    }

    /// Applies [`default_index`](Self::default_index) as the selection.
    pub fn select_default(&mut self) -> Option<usize> {
        let index = self.default_index();
        self.selected = index;
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(index: usize, language: &str) -> TrackInfo {
        TrackInfo::new(index, TrackKind::Audio).with_language(language)
    }

    #[test]
    fn a_track_of_another_kind_is_ignored() {
        let mut list = TrackList::new(TrackKind::Audio);
        list.push(audio(0, "en"));
        list.push(TrackInfo::new(1, TrackKind::Subtitle));
        assert_eq!(list.len(), 1);
        assert_eq!(list.tracks()[0].index, 0);
    }

    #[test]
    fn selecting_an_absent_index_leaves_the_selection() {
        let mut list = TrackList::new(TrackKind::Audio);
        list.push(audio(0, "en"));
        assert!(list.select(Some(0)));
        assert!(!list.select(Some(7)));
        assert_eq!(list.selected(), Some(0));
        assert!(list.select(None));
        assert_eq!(list.selected(), None);
    }

    #[test]
    fn the_default_track_wins_over_the_first() {
        let mut list = TrackList::new(TrackKind::Audio);
        list.push(audio(0, "en"));
        list.push(audio(1, "ar").as_default());
        assert_eq!(list.default_index(), Some(1));
    }

    #[test]
    fn an_empty_list_has_no_default() {
        let list = TrackList::new(TrackKind::Subtitle);
        assert_eq!(list.default_index(), None);
        assert!(list.is_empty());
    }

    #[test]
    fn language_matching_uses_the_primary_subtag() {
        let track = TrackInfo::new(0, TrackKind::Audio).with_language("en-GB");
        assert!(track.speaks("en"));
        assert!(track.speaks("EN"));
        assert!(!track.speaks("ar"));
        assert!(!TrackInfo::new(1, TrackKind::Audio).speaks("en"));
    }

    #[test]
    fn language_selection_can_require_forced() {
        let mut list = TrackList::new(TrackKind::Subtitle);
        list.push(TrackInfo::new(0, TrackKind::Subtitle).with_language("fr"));
        list.push(TrackInfo::new(1, TrackKind::Subtitle).with_language("fr").as_forced());
        assert_eq!(list.select_language("fr", false), Some(0));
        assert_eq!(list.select_language("fr", true), Some(1));
        assert_eq!(list.select_language("de", false), None);
    }

    #[test]
    fn describe_names_the_track() {
        let track = TrackInfo::new(2, TrackKind::Audio)
            .with_language("ar")
            .with_label("Commentary")
            .with_codec("aac")
            .with_channels(2)
            .as_default();
        let text = track.describe();
        assert!(text.contains("audio #2"));
        assert!(text.contains("ar"));
        assert!(text.contains("Commentary"));
        assert!(text.contains("2ch"));
        assert!(text.contains("default"));
    }
}
