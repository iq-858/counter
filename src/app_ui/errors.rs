//! On-screen error view.
//!
//! Background failures (decoder open, route change, HLS fetch, panic) used to
//! be visible only in `diagnostics.log` — a file a phone user cannot open. This
//! section mirrors the app's recent WARN/ERROR/PANIC lines into the UI so every
//! failure that happens away from the screen is visible where the user is.

use cranpose::prelude::*;

use super::theme;
use crate::app_diag;

/// The error view, refreshed on a short cadence so background failures appear
/// without a restart. Each read is just a locked snapshot of a 40-line ring.
#[composable]
pub fn ErrorsSection() {
    let ticks = rememberMutableStateOf(|| 0_u64);
    LaunchedEffect((), async move |_| {
        loop {
            cranpose::prelude::delay(std::time::Duration::from_millis(500)).await;
            ticks.update(|value| *value += 1);
        }
    });
    let _pump = ticks.get();

    let lines = app_diag::recent_lines();
    Column(
        Modifier::empty().fill_max_size().padding(12.0),
        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
        move || {
            Text("Background errors", Modifier::empty(), theme::heading_style());
            Text(
                format!("{} recent warning / error line(s)", lines.len()),
                Modifier::empty(),
                theme::muted_style(),
            );
            if lines.is_empty() {
                Text(
                    "No background errors yet. Open a clip or a live channel to exercise the decoder.",
                    Modifier::empty(),
                    theme::muted_style(),
                );
            } else {
                for line in lines.iter() {
                    Text(
                        line.clone(),
                        Modifier::empty().fill_max_width(),
                        theme::muted_style(),
                    );
                }
            }
        },
    );
}
