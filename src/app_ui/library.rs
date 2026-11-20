//! Library section: the full clip catalogue and an honest diagnostics summary.

use cranpose::prelude::*;

use super::theme;
use crate::app_model::{self, Clip, ClipKind};

/// One-card-per-entry catalogue view. This shows the whole [`app_model::catalogue`]
/// — offline clips, the opt-in test stream and the 42 live channels — so the
/// library never under-reports what the app can open.
#[composable]
pub fn LibrarySection(initial: String) {
    let catalogue = app_model::catalogue();
    let total = catalogue.len();
    let (offline, opt_in, live) = catalogue_split();

    Column(
        Modifier::empty().fill_max_size().padding(12.0),
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
        {
            move || {
                Text("LIBRARY · clip catalogue", Modifier::empty(), theme::heading_style());
                Text(
                    format!(
                        "{total} entries · {offline} offline · {opt_in} opt-in test stream · {live} live channels"
                    ),
                    Modifier::empty(),
                    theme::muted_style(),
                );
                Text(
                    format!("initial CLI selection: {initial}"),
                    Modifier::empty(),
                    theme::muted_style(),
                );
                for (index, clip) in catalogue.iter().enumerate() {
                    ClipCard(*clip, index);
                }
            }
        },
    );
}

/// A catalogue entry, kept deliberately simple and text-only.
#[composable]
fn ClipCard(clip: Clip, index: usize) {
    let group = if is_live(&clip) {
        "LIVE"
    } else if clip.kind == ClipKind::LocalMp4 {
        "OFFLINE"
    } else {
        "OPT-IN"
    };
    Column(
        Modifier::empty()
            .fill_max_width()
            .padding(10.0)
            .background(theme::SURFACE),
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(3.0)),
        move || {
            Text(
                format!("{}. [{group}] {} · [{}]", index + 1, clip.title, clip.id),
                Modifier::empty(),
                theme::heading_style(),
            );
            Text(
                format!("author: {}", clip.author),
                Modifier::empty(),
                theme::muted_style(),
            );
            Text(
                format!("format: {} · source kind: {}", clip.format, kind_label(clip.kind)),
                Modifier::empty(),
                theme::muted_style(),
            );
            Text(
                format!("source: {}", clip.source),
                Modifier::empty(),
                theme::muted_style(),
            );
            Text(clip.note, Modifier::empty(), theme::muted_style());
        },
    );
}

/// Counts the three catalogue groups: offline decode clips, the opt-in test
/// stream, and the live channels. Pure for unit testing.
fn catalogue_split() -> (usize, usize, usize) {
    let offline = app_model::CLIPS
        .iter()
        .filter(|clip| clip.kind == ClipKind::LocalMp4)
        .count();
    let opt_in = app_model::CLIPS.len() - offline;
    let live = app_model::LIVE_CHANNELS.len();
    (offline, opt_in, live)
}

/// Whether an entry belongs to the live channel list. Pure for unit testing.
fn is_live(clip: &Clip) -> bool {
    app_model::LIVE_CHANNELS.iter().any(|live| live.id == clip.id)
}

/// Stable label for a source kind, pure for unit testing.
fn kind_label(kind: ClipKind) -> &'static str {
    match kind {
        ClipKind::LocalMp4 => "local MP4 (offline)",
        ClipKind::Hls => "HLS (network, opt-in)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_labels_are_stable_and_distinct() {
        assert_eq!(kind_label(ClipKind::LocalMp4), "local MP4 (offline)");
        assert_eq!(kind_label(ClipKind::Hls), "HLS (network, opt-in)");
        assert_ne!(
            kind_label(ClipKind::LocalMp4),
            kind_label(ClipKind::Hls)
        );
    }

    #[test]
    fn catalogue_split_accounts_for_all_48_entries() {
        let (offline, opt_in, live) = catalogue_split();
        assert_eq!(offline, 5);
        assert_eq!(opt_in, 1);
        assert_eq!(live, 42);
        assert_eq!(offline + opt_in + live, app_model::catalogue().len());
    }

    #[test]
    fn live_membership_is_distinct_from_local_and_opt_in_clips() {
        assert!(is_live(&app_model::LIVE_CHANNELS[0]));
        assert!(!is_live(&app_model::CLIPS[0]));
        assert!(!is_live(&app_model::CLIPS[5]));
        assert_eq!(
            app_model::catalogue()
                .iter()
                .filter(|clip| is_live(*clip))
                .count(),
            app_model::LIVE_CHANNELS.len()
        );
    }
}
