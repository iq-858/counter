//! Live section: a vertical pager over the 42 opt-in Free-TV IPTV channels.
//!
//! Kept separate from the offline Reels pager so the deterministic decode sweep
//! and the network sources never mix. This section reuses the Reels page
//! composables, which means the same strict gate applies: only the settled page
//! composes a `VideoPlayer`, every other page is a static poster. Because the
//! root composes either Reels or Live (never both), at most one decoder is open
//! across the whole app at any time.

use cranpose::prelude::*;

use super::{reels, theme};
use crate::app_model::{self, Clip};

/// The visible caveat: these are third-party, opt-in, unverified streams.
const DISCLAIMER: &str = "Third-party, opt-in streams from two Free-TV IPTV playlists.";
const DISCLAIMER_DETAIL: &str =
    "Transcribed, not playback-verified; availability may be geo/filter-sensitive.";

/// The Live pager itself.
#[composable]
pub fn LiveSection() {
    let state = rememberPagerState(0);
    let settled = state.settled_page();
    let page_count = app_model::LIVE_CHANNELS.len();
    let (news, uae) = category_counts(app_model::LIVE_CHANNELS);
    crate::app_diag::info(
        "live.gate",
        format!("settled page {settled} of {page_count}"),
    );

    Column(
        Modifier::empty().fill_max_size(),
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
        {
            move || {
                Text(
                    live_page_label(settled, page_count),
                    Modifier::empty(),
                    theme::heading_style(),
                );
                Text(
                    format!("{page_count} channels · Arabic news {news} · UAE {uae}"),
                    Modifier::empty(),
                    theme::muted_style(),
                );
                Text(DISCLAIMER, Modifier::empty(), theme::accent_style());
                Text(DISCLAIMER_DETAIL, Modifier::empty(), theme::muted_style());
                VerticalPager(
                    Modifier::empty()
                        .fill_max_width()
                        .height(reels::pager_height())
                        .background(theme::SURFACE),
                    state,
                    PagerSpec::default(),
                    page_count,
                    move |page| {
                        let clip = app_model::LIVE_CHANNELS[page];
                        if app_model::active_page(settled, page) {
                            reels::ActiveReel(clip);
                        } else {
                            reels::InactiveReel(clip, page);
                        }
                    },
                );
            }
        },
    );
}

/// A human-readable page indicator, kept pure for unit testing.
fn live_page_label(settled: usize, page_count: usize) -> String {
    format!("LIVE · page {} of {}", settled.saturating_add(1), page_count)
}

/// Pulls the category out of the author label, e.g.
/// `"Free-TV IPTV · Arabic news"` becomes `"Arabic news"`. Pure for testing.
fn category_of(clip: &Clip) -> &'static str {
    clip.author
        .rsplit_once("· ")
        .map_or(clip.author, |(_, category)| category)
}

/// Counts the two playlist categories in a channel slice. Pure for testing.
fn category_counts(channels: &[Clip]) -> (usize, usize) {
    let news = channels
        .iter()
        .filter(|clip| category_of(clip) == "Arabic news")
        .count();
    let uae = channels
        .iter()
        .filter(|clip| category_of(clip) == "UAE")
        .count();
    (news, uae)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_page_label_is_one_based_and_handles_empty_catalogues() {
        assert_eq!(live_page_label(0, 42), "LIVE · page 1 of 42");
        assert_eq!(live_page_label(7, 42), "LIVE · page 8 of 42");
        assert_eq!(live_page_label(0, 0), "LIVE · page 1 of 0");
    }

    #[test]
    fn category_is_taken_from_the_author_label() {
        assert_eq!(
            category_of(&app_model::LIVE_CHANNELS[0]),
            "Arabic news"
        );
        assert_eq!(
            category_of(&app_model::LIVE_CHANNELS[8]),
            "UAE"
        );
        // A label without the separator falls back to the whole author string.
        let plain = Clip {
            author: "Plain author",
            ..app_model::LIVE_CHANNELS[0]
        };
        assert_eq!(category_of(&plain), "Plain author");
    }

    #[test]
    fn every_live_channel_is_in_one_of_the_two_playlists() {
        let (news, uae) = category_counts(app_model::LIVE_CHANNELS);
        assert_eq!(news, 8, "eight Arabic news channels are expected");
        assert_eq!(uae, 34, "thirty-four UAE channels are expected");
        assert_eq!(news + uae, app_model::LIVE_CHANNELS.len());
    }
}
