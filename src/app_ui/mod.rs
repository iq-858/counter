//! Native, single-window demo UI built on source-verified Cranpose APIs.
//!
//! The sections are separate modules so the pager, the catalogue view and the
//! decode test console stay independent of each other.

mod errors;
mod library;
mod live;
mod reels;
mod theme;
mod tests_console;
mod widgets;

use std::sync::OnceLock;

use cranpose::prelude::*;

use crate::app_model;

/// Optional first command-line argument, kept for the whole process.
pub(crate) static INITIAL_CLIP: OnceLock<String> = OnceLock::new();

/// The section currently shown in the single window.
#[derive(Clone, Copy, PartialEq)]
enum Section {
    Reels,
    Live,
    Library,
    Tests,
    Errors,
}

/// Maps the optional CLI clip to a starting pager page.
///
/// Accepts either the catalogue source path or the short id. An unknown value
/// falls back to the original harness default (`bunny.mp4`, page index 2).
fn initial_page() -> usize {
    let selected = INITIAL_CLIP.get().map(String::as_str).unwrap_or("");
    app_model::CLIPS
        .iter()
        .position(|clip| clip.source == selected || clip.id == selected)
        .unwrap_or(2)
}

/// The selected clip as a label for the library section.
fn initial_clip_label() -> String {
    INITIAL_CLIP
        .get()
        .cloned()
        .unwrap_or_else(|| app_model::CLIPS[2].source.to_owned())
}

/// Root composable of the application.
#[composable]
pub fn AppRoot() {
    let section = rememberMutableStateOf(|| Section::Reels);
    Column(
        Modifier::empty()
            .fill_max_size()
            .background(theme::CANVAS)
            .padding(16.0),
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(10.0)),
        {
            let section = section.clone();
            move || {
                Text(
                    "cranpose-video · Reels Lab",
                    Modifier::empty(),
                    theme::title_style(),
                );
                Text(
                    "One window · one active decoder · local clips + 42 opt-in live channels · local decode sweep",
                    Modifier::empty(),
                    theme::muted_style(),
                );
                Row(
                    Modifier::empty(),
                    RowSpec::new().horizontal_arrangement(LinearArrangement::SpacedBy(8.0)),
                    {
                        let reels = section.clone();
                        let live = section.clone();
                        let library = section.clone();
                        let tests = section.clone();
                        let errors = section.clone();
                        move || {
                            section_tab("Reels", Section::Reels, reels.clone());
                            section_tab("Live", Section::Live, live.clone());
                            section_tab("Library", Section::Library, library.clone());
                            section_tab("Decode tests", Section::Tests, tests.clone());
                            let err_count = crate::app_diag::recent_lines().len();
                            section_tab(
                                if err_count > 0 {
                                    "Errors !"
                                } else {
                                    "Errors"
                                },
                                Section::Errors,
                                errors.clone(),
                            );
                        }
                    },
                );
                match section.get() {
                    Section::Reels => reels::ReelsSection(initial_page()),
                    Section::Live => live::LiveSection(),
                    Section::Library => library::LibrarySection(initial_clip_label()),
                    Section::Tests => tests_console::TestsSection(),
                    Section::Errors => errors::ErrorsSection(),
                }
            }
        },
    );
}

/// One tab of the section switcher.
#[composable]
fn section_tab(label: impl Into<String>, target: Section, current: MutableState<Section>) {
    let label = label.into();
    let selected = current.get() == target;
    let on_click = current.clone();
    Button(
        Modifier::empty().padding(6.0),
        ButtonSpec::new(),
        {
            let label = label.clone();
            move || {
                // Clear the on-screen error ring on section change so a tab
                // shows only its own failures, never stale ones from another
                // section (e.g. an old Reels clip error in the Live tab). The
                // file log keeps the complete history.
                crate::app_diag::clear_screen();
                crate::app_diag::info("nav", format!("switch to {label}"));
                on_click.set(target);
            }
        },
        move || {
            Text(
                label.clone(),
                Modifier::empty(),
                if selected {
                    theme::accent_style()
                } else {
                    theme::body_style()
                },
            );
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_page_is_the_original_default_clip() {
        // The static is unset in unit tests, so the fallback is used.
        assert_eq!(initial_page(), 2);
        assert_eq!(
            initial_clip_label(),
            app_model::CLIPS[2].source.to_owned()
        );
    }
}
