use cranpose::prelude::*;
use cranpose::text::TextUnit;

pub use cranpose::text::FontWeight;
pub type NodeId = usize;

pub const BG: Color = Color::from_rgb_u8(9, 11, 18);
pub const SURFACE: Color = Color::from_rgb_u8(19, 22, 33);
pub const SURFACE_2: Color = Color::from_rgb_u8(29, 33, 47);
pub const DIVIDER: Color = Color::from_rgb_u8(53, 59, 76);
pub const PRIMARY: Color = Color::from_rgb_u8(255, 73, 112);
pub const ACCENT: Color = Color::from_rgb_u8(108, 178, 255);
pub const TEXT: Color = Color::from_rgb_u8(255, 255, 255);
pub const TEXT_DIM: Color = Color::from_rgb_u8(176, 184, 201);
pub const TEXT_FAINT: Color = Color::from_rgb_u8(126, 136, 157);

/// Corner radii shared across cards and sheets.
pub const RADIUS_CARD: f32 = 18.0;
pub const RADIUS_TILE: f32 = 20.0;
/// Horizontal gutter used by every screen.
pub const GUTTER: f32 = 18.0;

pub fn ts(size: f32, color: Color, weight: FontWeight) -> TextStyle {
    TextStyle::from_span_style(SpanStyle {
        color: Some(color),
        font_size: TextUnit::Sp(size),
        font_weight: Some(weight),
        ..Default::default()
    })
}

pub fn label(size: f32, color: Color) -> TextStyle {
    ts(size, color, FontWeight::NORMAL)
}

pub fn bold(size: f32, color: Color) -> TextStyle {
    ts(size, color, FontWeight::BOLD)
}
