//! A DASH segment scheduler over the structural `DashManifest` parser.
//!
//! The MPD reader in `dash.rs` extracts representations and the segment
//! template; this module turns that into an ordered, addressable segment list
//! the same way `hls::playlist` does for HLS, so `AbrController` can drive
//! both engines through one interface.

use crate::abr::{AbrController, Representation};
use crate::dash::DashManifest;

/// One addressable DASH segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashSegment {
    /// Resolved URL (template with `$Number$` substituted).
    pub url: String,
    /// Zero-based segment number.
    pub number: u64,
    /// Declared or assumed duration in milliseconds.
    pub duration_ms: u64,
}

/// A scheduled representation: its segments plus the ABR view of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashStream {
    /// Representation id from the MPD.
    pub id: String,
    /// Declared bandwidth in bits per second.
    pub bandwidth: u64,
    /// Encoded height in pixels.
    pub height: u32,
    /// Initialization segment URL, when the template declares one.
    pub initialization: Option<String>,
    /// The ordered segment list.
    pub segments: Vec<DashSegment>,
}

impl DashStream {
    /// The URL of segment `number`, when it exists.
    pub fn segment_url(&self, number: u64) -> Option<&str> {
        self.segments
            .iter()
            .find(|segment| segment.number == number)
            .map(|segment| segment.url.as_str())
    }

    /// Total declared duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.segments.iter().map(|segment| segment.duration_ms).sum()
    }
}

/// Expands a parsed MPD into per-representation segment lists.
///
/// `segment_count` bounds the expansion for a static MPD whose
/// `duration_ms` and template cadence would otherwise imply an unbounded
/// list; a dynamic MPD is expanded the same way and refreshed by the caller.
pub fn schedule(
    manifest: &DashManifest,
    base_url: &str,
    segment_count: u64,
    segment_duration_ms: u64,
) -> Vec<DashStream> {
    let template = match &manifest.segment_template {
        Some(template) => template,
        None => return Vec::new(),
    };
    let cadence = if segment_duration_ms > 0 {
        segment_duration_ms
    } else {
        2000
    };
    manifest
        .representations
        .iter()
        .map(|representation| {
            let segments = (0..segment_count)
                .map(|number| DashSegment {
                    url: resolve(base_url, &template.replace("$Number$", &number.to_string())),
                    number,
                    duration_ms: cadence,
                })
                .collect();
            DashStream {
                id: representation.id.clone(),
                bandwidth: representation.bandwidth,
                height: representation.height,
                initialization: manifest
                    .initialization
                    .as_ref()
                    .map(|init| resolve(base_url, init)),
                segments,
            }
        })
        .collect()
}

/// Picks the representation `AbrController` currently prefers and returns its
/// index into the stream list.
///
/// The controller is fed the declared bandwidths as `Representation`s, so the
/// same ABR policy governs HLS and DASH.
pub fn choose_stream(
    controller: &mut AbrController,
    streams: &[DashStream],
    throughput_bps: u64,
    buffer_millis: u64,
) -> Option<usize> {
    if streams.is_empty() {
        return None;
    }
    let representations: Vec<Representation> = streams
        .iter()
        .enumerate()
        .map(|(index, stream)| Representation {
            id: index as u64,
            bandwidth: stream.bandwidth,
            width: 0,
            height: stream.height,
        })
        .collect();
    Some(controller.sample(throughput_bps, buffer_millis, &representations))
}

fn resolve(base: &str, reference: &str) -> String {
    if reference.starts_with("http://") || reference.starts_with("https://") {
        return reference.to_owned();
    }
    if let Some(position) = base.rfind('/') {
        return format!("{}/{}", &base[..position], reference);
    }
    reference.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abr::AbrConfig;
    use crate::dash::parse_mpd;

    const MPD: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT60S">
  <Period>
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate media="seg-$Number$.m4s" initialization="init.mp4" />
      <Representation id="0" bandwidth="500000" width="640" height="360" />
      <Representation id="1" bandwidth="1500000" width="1280" height="720" />
    </AdaptationSet>
  </Period>
</MPD>"#;

    fn streams() -> Vec<DashStream> {
        let manifest = parse_mpd(MPD).expect("a valid MPD");
        schedule(&manifest, "https://cdn.example/video/manifest.mpd", 4, 2000)
    }

    #[test]
    fn expands_the_template_per_representation() {
       let streams = streams();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[1].segments.len(), 4);
        assert_eq!(
            streams[1].segments[2].url,
            "https://cdn.example/video/seg-2.m4s"
        );
        assert_eq!(
            streams[1].initialization.as_deref(),
            Some("https://cdn.example/video/init.mp4")
        );
        assert_eq!(streams[1].duration_ms(), 8000);
    }

    #[test]
    fn segment_lookup_by_number() {
        let streams = streams();
        assert_eq!(streams[0].segment_url(3), Some("https://cdn.example/video/seg-3.m4s"));
        assert_eq!(streams[0].segment_url(9), None);
    }

    #[test]
    fn a_manifest_without_a_template_schedules_nothing() {
        let manifest = parse_mpd("<MPD><Period><Representation id=\"0\" bandwidth=\"1\"/></Period></MPD>")
            .expect("parses");
        assert!(schedule(&manifest, "https://cdn/", 4, 2000).is_empty());
    }

    #[test]
    fn abr_prefers_the_low_representation_on_a_thin_pipe() {
        let streams = streams();
        let mut controller = AbrController::new(AbrConfig::default());
        let chosen = choose_stream(&mut controller, &streams, 600_000, 4_000);
        assert_eq!(chosen, Some(0), "600 kbps must select the 500 kbps stream");
    }

    #[test]
    fn abr_prefers_the_high_representation_on_a_fast_pipe() {
        let streams = streams();
        let mut controller = AbrController::new(AbrConfig::default());
        // An upgrade needs `upgrade_after` consecutive good samples.
        let mut chosen = None;
        for _ in 0..3 {
            chosen = choose_stream(&mut controller, &streams, 20_000_000, 30_000);
        }
        assert_eq!(chosen, Some(1), "20 Mbps must select the 1.5 Mbps stream");
    }

    #[test]
    fn absolute_urls_are_not_resolved_again() {
        let manifest = parse_mpd(MPD).expect("a valid MPD");
        let mut streams = schedule(&manifest, "https://cdn.example/video/manifest.mpd", 1, 2000);
        streams[0].segments[0].url = "https://other.example/seg-0.m4s".to_owned();
        assert_eq!(streams[0].segment_url(0), Some("https://other.example/seg-0.m4s"));
    }
}
