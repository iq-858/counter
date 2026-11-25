//! A minimal, dependency-free MPEG-DASH MPD reader.
//!
//! This parses the structural facts a player needs to begin selecting a
//! representation: representation ids/bandwidths/resolutions, a segment
//! template, and whether the MPD is static or dynamic. It does not download
//! segments, decode them, or implement a full DASH scheduler.

/// A representation declared by an MPD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashRepresentation {
    /// Manifest identifier.
    pub id: String,
    /// Declared bandwidth in bits per second.
    pub bandwidth: u64,
    /// Encoded width in pixels.
    pub width: u32,
    /// Encoded height in pixels.
    pub height: u32,
    /// MIME type declared by the adaptation set.
    pub mime_type: String,
}

/// The structural contents of an MPD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashManifest {
    /// Whether the MPD declares a dynamic (live) presentation.
    pub dynamic: bool,
    /// Declared duration in milliseconds when parseable, otherwise zero.
    pub duration_ms: u64,
    /// Segment template path from the first `<SegmentTemplate>`.
    pub segment_template: Option<String>,
    /// Initialization segment path when declared.
    pub initialization: Option<String>,
    /// Representations found in the document.
    pub representations: Vec<DashRepresentation>,
}

fn attr(tag: &str, name: &str) -> Option<String> {
    // A plain `find("width=\"")` also matches inside `bandwidth="..."`, so
    // require a real attribute boundary before the name.
    let needle = format!("{name}=\"");
    let mut from = 0usize;
    while let Some(offset) = tag[from..].find(&needle) {
        let start = from + offset;
        let boundary = start == 0
            || tag.as_bytes()[start - 1].is_ascii_whitespace()
            || tag.as_bytes()[start - 1] == b'<';
        if boundary {
            let value_start = start + needle.len();
            let rest = &tag[value_start..];
            let end = rest.find('"')?;
            return Some(rest[..end].to_owned());
        }
        from = start + needle.len();
    }
    None
}

fn number(tag: &str, name: &str) -> Option<u64> {
    attr(tag, name)?.parse().ok()
}

/// Parses the representations and segment template from an MPD document.
///
/// Returns an error when the document has no `<MPD>` root or no
/// `<Representation>` elements. Unknown attributes are ignored.
pub fn parse_mpd(input: &str) -> Result<DashManifest, String> {
    if !input.contains("<MPD") {
        return Err("not an MPD document".to_owned());
    }
    let dynamic = input.contains("type=\"dynamic\"");
    let duration_ms = number(&input[..input.find('>').unwrap_or(input.len())], "mediaPresentationDuration")
        .or_else(|| number(input, "mediaPresentationDuration"))
        .unwrap_or(0);

    let mut template = None;
    let mut initialization = None;
    if let Some(start) = input.find("<SegmentTemplate") {
        let end = input[start..].find('>').map(|offset| start + offset).unwrap_or(input.len());
        let tag = &input[start..end];
        template = attr(tag, "media");
        initialization = attr(tag, "initialization");
    }

    let mut representations = Vec::new();
    let mut search = 0usize;
    while let Some(offset) = input[search..].find("<Representation") {
        let start = search + offset;
        let end = input[start..]
            .find('>')
            .map(|offset| start + offset)
            .unwrap_or(input.len());
        let tag = &input[start..end];
        representations.push(DashRepresentation {
            id: attr(tag, "id").unwrap_or_default(),
            bandwidth: number(tag, "bandwidth").unwrap_or(0),
            width: number(tag, "width").unwrap_or(0) as u32,
            height: number(tag, "height").unwrap_or(0) as u32,
            mime_type: attr(tag, "mimeType").unwrap_or_default(),
        });
        search = end;
    }
    if representations.is_empty() {
        return Err("MPD has no representations".to_owned());
    }
    Ok(DashManifest {
        dynamic,
        duration_ms,
        segment_template: template,
        initialization,
        representations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn parses_representations_and_template() {
        let manifest = parse_mpd(MPD).expect("a valid MPD");
        assert_eq!(manifest.representations.len(), 2);
        assert_eq!(manifest.representations[1].bandwidth, 1_500_000);
        assert_eq!(manifest.representations[1].width, 1280);
        assert_eq!(manifest.segment_template.as_deref(), Some("seg-$Number$.m4s"));
        assert_eq!(manifest.initialization.as_deref(), Some("init.mp4"));
        assert!(!manifest.dynamic);
    }

    #[test]
    fn a_dynamic_mpd_is_marked() {
        let dynamic = MPD.replace("type=\"static\"", "type=\"dynamic\"");
        assert!(parse_mpd(&dynamic).unwrap().dynamic);
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(parse_mpd("<html/>").is_err());
        assert!(parse_mpd("<MPD><Period/></MPD>").is_err());
    }
}
