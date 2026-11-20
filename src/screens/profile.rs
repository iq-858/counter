use cranpose::prelude::*;
use cranpose::widgets::Icon;

use crate::data::posts;
use crate::icons;
use crate::theme::*;

pub fn profile_screen() -> NodeId {
    let grid = std::rc::Rc::new(posts());
    Box(
        Modifier::empty().fill_max_size().background(BG),
        BoxSpec::new(),
        move || {
            let grid = grid.clone();
            Column(
                Modifier::empty().fill_max_size().padding(14.0),
                ColumnSpec::new()
                    .horizontal_alignment(HorizontalAlignment::CenterHorizontally)
                    .vertical_arrangement(LinearArrangement::SpacedBy(12.0)),
                move || {
                    let grid = grid.clone();
                    Box(
                        Modifier::empty().fill_max_width().height(40.0),
                        BoxSpec::new().content_alignment(Alignment::CENTER),
                        || {
                            Text(
                                "@novarivers",
                                Modifier::empty(),
                                ts(16.0, TEXT, FontWeight::BOLD),
                            );
                        },
                    );

                    crate::components::avatar("N", 0, 84.0);

                    Row(
                        Modifier::empty(),
                        RowSpec::new()
                            .horizontal_arrangement(LinearArrangement::SpacedBy(28.0))
                            .vertical_alignment(VerticalAlignment::CenterVertically),
                        || {
                            stat("482", "Following");
                            stat("1.2M", "Followers");
                            stat("9.4M", "Likes");
                        },
                    );

                    Row(
                        Modifier::empty(),
                        RowSpec::new().horizontal_arrangement(LinearArrangement::SpacedBy(8.0)),
                        || {
                            profile_button("Edit profile", icons::EDIT, true);
                            profile_button("Share", icons::SHARE, false);
                            profile_button("Settings", icons::SETTINGS, false);
                        },
                    );

                    Text(
                        "Cinematic edits & late-night city loops. Seoul → anywhere.",
                        Modifier::empty(),
                        ts(13.0, TEXT_DIM, FontWeight::NORMAL),
                    );

                    Row(
                        Modifier::empty(),
                        RowSpec::new().horizontal_arrangement(LinearArrangement::SpacedBy(24.0)),
                        || {
                            Icon(icons::GRID, None, 22.0, TEXT);
                            Icon(icons::HEART, None, 22.0, TEXT_FAINT);
                            Icon(icons::BOOKMARK, None, 22.0, TEXT_FAINT);
                        },
                    );

                    grid_3x2(grid);
                },
            );
        },
    )
}

fn stat(value: &'static str, label: &'static str) -> NodeId {
    Column(
        Modifier::empty(),
        ColumnSpec::new()
            .horizontal_alignment(HorizontalAlignment::CenterHorizontally)
            .vertical_arrangement(LinearArrangement::SpacedBy(2.0)),
        move || {
            Text(value, Modifier::empty(), ts(17.0, TEXT, FontWeight::BOLD));
            Text(
                label,
                Modifier::empty(),
                ts(12.0, TEXT_FAINT, FontWeight::NORMAL),
            );
        },
    )
}

fn profile_button(label: &'static str, path: &'static str, filled: bool) -> NodeId {
    let bg = if filled { PRIMARY } else { SURFACE_2 };
    let fg = if filled { TEXT } else { TEXT };
    Box(
        Modifier::empty()
            .height(34.0)
            .background(bg)
            .rounded_corners(8.0),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Row(
                Modifier::empty().padding(12.0),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(6.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                move || {
                    Icon(path, None, 14.0, fg);
                    Text(label, Modifier::empty(), ts(12.0, fg, FontWeight::MEDIUM));
                },
            );
        },
    )
}

fn grid_3x2(grid: std::rc::Rc<Vec<crate::data::Post>>) -> NodeId {
    Box(
        Modifier::empty().fill_max_width().height(300.0),
        BoxSpec::new(),
        move || {
            let grid = grid.clone();
            Column(
                Modifier::empty(),
                ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(3.0)),
                move || {
                    let grid = grid.clone();
                    for row in 0..2usize {
                        let grid_row = grid.clone();
                        Row(
                            Modifier::empty(),
                            RowSpec::new().horizontal_arrangement(LinearArrangement::SpacedBy(3.0)),
                            move || {
                                for col in 0..3usize {
                                    let idx = row * 3 + col;
                                    let variant = grid_row[idx].variant;
                                    let color = match variant % 5 {
                                        0 => Color::from_rgb_u8(254, 44, 85),
                                        1 => Color::from_rgb_u8(37, 244, 238),
                                        2 => Color::from_rgb_u8(255, 150, 40),
                                        3 => Color::from_rgb_u8(140, 90, 255),
                                        _ => Color::from_rgb_u8(80, 230, 140),
                                    };
                                    Box(
                                        Modifier::empty()
                                            .width(126.0)
                                            .height(146.0)
                                            .background(color.with_alpha(0.3))
                                            .rounded_corners(4.0),
                                        BoxSpec::new(),
                                        || {},
                                    );
                                }
                            },
                        );
                    }
                },
            );
        },
    )
}

pub fn profile_window_content() -> NodeId {
    profile_screen()
}
