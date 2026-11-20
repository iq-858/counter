//! Small composable building blocks shared by the sections.

use cranpose::prelude::*;

use super::theme;

/// A compact button used by the player controls and the test console.
#[composable]
pub fn small_button(label: impl Into<String> + 'static, on_click: impl FnMut() + 'static) {
    let label = label.into();
    Button(
        Modifier::empty().padding(6.0),
        ButtonSpec::new(),
        on_click,
        move || {
            Text(label.clone(), Modifier::empty(), theme::body_style());
        },
    );
}
