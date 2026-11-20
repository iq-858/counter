//! Dark-themed colours and text styles shared by every section.
//!
//! Only pure values live here, so these helpers can be unit tested without a
//! window or a decoder.

use cranpose::prelude::*;
use cranpose::text::TextUnit;

/// Canvas behind the whole window.
pub const CANVAS: Color = Color(0.055, 0.063, 0.082, 1.0);
/// Surface of cards and the pager.
pub const SURFACE: Color = Color(0.086, 0.098, 0.125, 1.0);
/// Slightly lighter surface used behind video pictures.
pub const SURFACE_RAISED: Color = Color(0.125, 0.141, 0.176, 1.0);
/// Primary text.
pub const TEXT: Color = Color(0.92, 0.94, 0.98, 1.0);
/// Secondary text.
pub const TEXT_MUTED: Color = Color(0.62, 0.67, 0.76, 1.0);
/// Accent for the active tab and live status.
pub const ACCENT: Color = Color(0.36, 0.62, 0.98, 1.0);
/// Passing decode result.
pub const PASS: Color = Color(0.32, 0.78, 0.52, 1.0);
/// Failing decode result.
pub const FAIL: Color = Color(0.92, 0.38, 0.38, 1.0);

/// Builds a text style from a size in scaled pixels and a colour.
fn styled(size: f32, color: Color) -> TextStyle {
    TextStyle::from_span_style(SpanStyle {
        color: Some(color),
        font_size: TextUnit::Sp(size),
        ..SpanStyle::default()
    })
}

/// Window heading.
pub fn title_style() -> TextStyle {
    styled(22.0, TEXT)
}

/// Section and card headings.
pub fn heading_style() -> TextStyle {
    styled(16.0, TEXT)
}

/// Ordinary body copy and control labels.
pub fn body_style() -> TextStyle {
    styled(13.0, TEXT)
}

/// Secondary copy such as authors and notes.
pub fn muted_style() -> TextStyle {
    styled(12.0, TEXT_MUTED)
}

/// Live status and selection.
pub fn accent_style() -> TextStyle {
    styled(13.0, ACCENT)
}

/// A passing test row.
pub fn pass_style() -> TextStyle {
    styled(13.0, PASS)
}

/// A failing test row.
pub fn fail_style() -> TextStyle {
    styled(13.0, FAIL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_carry_their_requested_size_and_colour() {
        let style = styled(17.0, ACCENT);
        assert_eq!(style.span_style.color, Some(ACCENT));
        assert_eq!(style.span_style.font_size, TextUnit::Sp(17.0));
    }

    #[test]
    fn pass_and_fail_have_distinct_colours() {
        assert_ne!(pass_style().span_style.color, fail_style().span_style.color);
    }
}
