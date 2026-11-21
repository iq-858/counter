//! On-screen diagnostic view — a full timeline of what the app and library
//! logged in the background.
//!
//! Everything written to `app_diag` (decoder open/play/frame, route changes,
//! session failures, HLS fetches, panics, and all library diagnostics through
//! `set_diagnostic_sink`) is mirrored into an in-memory ring. A phone user cannot
//! open `diagnostics.log` in the app sandbox, so this view is the only place a
//! background failure is visible.

use cranpose::prelude::*;

use super::theme;
use crate::app_diag;

/// The diagnostic view, refreshed on a short cadence so background events
/// appear without a restart. Each read is just a locked snapshot of a bounded
/// ring, so a long session cannot grow without limit.
#[composable]
pub fn ErrorsSection() {
    let ticks = rememberMutableStateOf(|| 0_u64);
    LaunchedEffect((), async move |_| {
        loop {
            delay(std::time::Duration::from_millis(400)).await;
            ticks.update(|value| *value += 1);
        }
    });
    let _pump = ticks.get();

    let lines = app_diag::recent_lines();
    let error_count = lines.iter().filter(|l| l.contains("ERROR") || l.contains("PANIC")).count();
    let clear = {
        move || app_diag::clear_screen()
    };
    Column(
        Modifier::empty().fill_max_size().padding(12.0),
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
        move || {
            Text("Diagnostics & errors", Modifier::empty(), theme::heading_style());
            Text(
                format!(
                    "{} line(s) · {} error(s) — refreshes automatically",
                    lines.len(),
                    error_count
                ),
                Modifier::empty(),
                theme::muted_style(),
            );
            super::widgets::small_button("Clear", clear.clone());
            if lines.is_empty() {
                Text(
                    "No events yet. Open a clip or a live channel to exercise the decoder.",
                    Modifier::empty(),
                    theme::muted_style(),
                );
            } else {
                for line in lines.iter() {
                    Text(
                        line.clone(),
                        Modifier::empty().fill_max_width(),
                        if line.contains("ERROR") || line.contains("PANIC") {
                            theme::accent_style()
                        } else {
                            theme::muted_style()
                        },
                    );
                }
            }
        },
    );
}
