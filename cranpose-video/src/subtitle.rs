//! Unified subtitle parsing and synchronisation (SRT + WebVTT).
//!
//! Pure Rust, no renderer dependency: the player asks for cues active at a
//! presentation time and the app renders them with its own UI. This keeps the
//! core decoder usable without Cranpose UI.

use std::time::Duration;

/// One timed text cue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleCue {
    /// Sequential cue number (assigned during parsing).
    pub index: u32,
    /// Time when the cue appears.
    pub start: Duration,
    /// Time when the cue disappears.
    pub end: Duration,
    /// Text lines of the cue, without the timing header.
    pub lines: Vec<String>,
    /// WebVTT cue settings that map to positioning (raw, for the app to apply).
    pub settings: String,
}

impl SubtitleCue {
    /// Whether this cue is active at `time` (start inclusive, end exclusive).
    pub fn is_active_at(&self, time: Duration) -> bool {
        time >= self.start && time < self.end
    }
}

fn parse_timestamp(raw: &str) -> Option<Duration> {
    let raw = raw.trim().replace(',', ".");
    let mut parts = raw.split(':');
    let hours: u64 = parts.next()?.parse().ok()?;
    let minutes: u64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    let total = hours as f64 * 3600.0 + minutes as f64 * 60.0 + seconds;
    if !total.is_finite() || total < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(total))
}

/// Parses SRT or WebVTT text into cues. Malformed blocks are skipped rather
/// than failing the whole track. WebVTT headers and NOTE blocks are ignored.
pub fn parse(input: &str) -> Vec<SubtitleCue> {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let blocks: Vec<&str> = normalized.split("\n\n").collect();
    let mut cues = Vec::new();
    let mut next_index = 1u32;

    for block in blocks {
        let block = block.trim();
        if block.is_empty()
            || block.starts_with("WEBVTT")
            || block.starts_with("NOTE")
            || block.starts_with("Kind:")
            || block.starts_with("Language:")
        {
            continue;
        }
        let lines: Vec<&str> = block.lines().collect();
        // Timing can be on line 1 (no numeric index) or line 2 (SRT index).
        let timing_pos = lines.iter().position(|line| line.contains("-->"));
        let Some(pos) = timing_pos else { continue };
        let timing = lines[pos];
        let mut tp = timing.split("-->");
        let start_raw = tp.next().unwrap_or("");
        let end_with_settings = tp.next().unwrap_or("");
        let mut ep = end_with_settings.split_whitespace();
        let end_raw = ep.next().unwrap_or("");
        let settings = ep.collect::<Vec<_>>().join(" ");
        let (Some(start), Some(end)) = (parse_timestamp(start_raw), parse_timestamp(end_raw))
        else {
            continue;
        };
        if end <= start {
            continue;
        }
        let mut text_lines: Vec<String> = lines
            .iter()
            .skip(pos + 1)
            .map(|line| (*line).to_owned())
            .collect();
        // A line before the timing is an SRT numeric index (skip it) or a
        // WebVTT cue identifier (keep it). WebVTT identifiers are non-numeric.
        if pos > 0 && !lines[0].contains("-->") && lines[0].trim().parse::<u32>().is_err() {
            text_lines.insert(0, lines[0].to_owned());
        }
        if text_lines.iter().all(|line| line.trim().is_empty()) {
            continue;
        }
        cues.push(SubtitleCue {
            index: next_index,
            start,
            end,
            lines: text_lines,
            settings,
        });
        next_index = next_index.saturating_add(1);
    }
    cues
}

/// Returns only the cues active at `time`.
pub fn active_at<'a>(cues: &'a [SubtitleCue], time: Duration) -> Vec<&'a SubtitleCue> {
    cues.iter().filter(|cue| cue.is_active_at(time)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_srt() {
        let srt = "1\n00:00:01,000 --> 00:00:03,500\nHello\nworld\n\n2\n00:00:04,000 --> 00:00:05,000\nBye\n";
        let cues = parse(srt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start, Duration::from_secs(1));
        assert_eq!(cues[0].end, Duration::from_millis(3500));
        assert_eq!(cues[0].lines, vec!["Hello", "world"]);
    }

    #[test]
    fn parses_webvtt_with_settings() {
        let vtt = "WEBVTT\n\n00:00:02.000 --> 00:00:04.000 align:start\nHi there\n";
        let cues = parse(vtt);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].settings, "align:start");
    }

    #[test]
    fn skips_malformed_and_notes() {
        let text = "NOTE this is a comment\n\nbad timing\n00:xx --> nope\ntext\n\n1\n00:00:01,000 --> 00:00:02,000\nGood\n";
        let cues = parse(text);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].lines, vec!["Good"]);
    }

    #[test]
    fn active_cue_lookup() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nA\n";
        let cues = parse(srt);
        assert_eq!(active_at(&cues, Duration::from_millis(1500)).len(), 1);
        assert!(active_at(&cues, Duration::from_secs(3)).is_empty());
    }

    #[test]
    fn cue_active_boundaries() {
        let cue = SubtitleCue {
            index: 1,
            start: Duration::from_secs(1),
            end: Duration::from_secs(2),
            lines: vec![],
            settings: String::new(),
        };
        assert!(cue.is_active_at(Duration::from_secs(1)));
        assert!(!cue.is_active_at(Duration::from_secs(2)));
    }
}
