use cranpose::prelude::*;

pub struct Palette {
    pub bg_top: Color,
    pub bg_bottom: Color,
    pub a: Color,
    pub b: Color,
    pub c: Color,
}

pub fn palette(variant: u8) -> Palette {
    match variant % 5 {
        0 => Palette {
            bg_top: Color::from_rgb_u8(8, 10, 30),
            bg_bottom: Color::from_rgb_u8(40, 8, 50),
            a: Color::from_rgb_u8(254, 44, 85),
            b: Color::from_rgb_u8(37, 244, 238),
            c: Color::from_rgb_u8(120, 60, 220),
        },
        1 => Palette {
            bg_top: Color::from_rgb_u8(4, 18, 28),
            bg_bottom: Color::from_rgb_u8(6, 40, 52),
            a: Color::from_rgb_u8(37, 244, 238),
            b: Color::from_rgb_u8(30, 120, 255),
            c: Color::from_rgb_u8(140, 255, 200),
        },
        2 => Palette {
            bg_top: Color::from_rgb_u8(28, 14, 6),
            bg_bottom: Color::from_rgb_u8(60, 22, 8),
            a: Color::from_rgb_u8(255, 150, 40),
            b: Color::from_rgb_u8(254, 44, 85),
            c: Color::from_rgb_u8(255, 214, 90),
        },
        3 => Palette {
            bg_top: Color::from_rgb_u8(10, 6, 26),
            bg_bottom: Color::from_rgb_u8(22, 6, 42),
            a: Color::from_rgb_u8(140, 90, 255),
            b: Color::from_rgb_u8(37, 244, 238),
            c: Color::from_rgb_u8(254, 44, 85),
        },
        _ => Palette {
            bg_top: Color::from_rgb_u8(6, 22, 16),
            bg_bottom: Color::from_rgb_u8(10, 44, 30),
            a: Color::from_rgb_u8(80, 230, 140),
            b: Color::from_rgb_u8(37, 200, 244),
            c: Color::from_rgb_u8(255, 220, 120),
        },
    }
}
