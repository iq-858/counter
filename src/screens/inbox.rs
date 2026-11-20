use cranpose::prelude::*;
use cranpose::widgets::Icon;
use cranpose_animation::{animateFloatAsState, spring};

use crate::components::{entrance_layer, first_letter};
use crate::data::Notification;
use crate::icons;
use crate::theme::*;

pub fn inbox_screen(
    items: Vec<Notification>,
    following: Vec<bool>,
    on_follow: impl Fn(usize) + 'static,
) -> NodeId {
    let entered = rememberMutableStateOf(|| false);
    let filter = rememberMutableStateOf(|| 0usize);
    let target = if entered.get() { 1.0_f32 } else { 0.0_f32 };
    let progress = animateFloatAsState(target, spring(0.88, 220.0), "inbox-enter").get();
    LaunchedEffect(0u8, move |_scope| entered.set(true));

    let items = std::rc::Rc::new(items);
    let following = std::rc::Rc::new(following);
    let on_follow = std::rc::Rc::new(on_follow);
    let filters: [&'static str; 4] = ["All", "Likes", "Comments", "Followers"];
    Box(
        Modifier::empty()
            .fill_max_size()
            .graphics_layer_value(entrance_layer(progress)),
        BoxSpec::new(),
        move || {
            let items = items.clone();
            let following = following.clone();
            let on_follow = on_follow.clone();
            let selected_filter = filter.get();
            Column(
                Modifier::empty()
                    .fill_max_size()
                    .vertical_scroll(remember(|| ScrollState::new(0.0)).with(|s| *s), false)
                    .padding(GUTTER),
                ColumnSpec::new()
                    .horizontal_alignment(HorizontalAlignment::Start)
                    .vertical_arrangement(LinearArrangement::SpacedBy(14.0)),
                move || {
                    let following = following.clone();
                    let on_follow = on_follow.clone();
                    Row(
                        Modifier::empty()
                            .fill_max_width()
                            .padding_each(0.0, 8.0, 0.0, 0.0),
                        RowSpec::new().vertical_alignment(VerticalAlignment::CenterVertically),
                        || {
                            Column(
                                Modifier::empty(),
                                ColumnSpec::new()
                                    .vertical_arrangement(LinearArrangement::SpacedBy(3.0)),
                                || {
                                    Text(
                                        "Your activity",
                                        Modifier::empty(),
                                        ts(25.0, TEXT, FontWeight::BOLD),
                                    );
                                    Text(
                                        "Stay close to your community",
                                        Modifier::empty(),
                                        ts(13.0, TEXT_DIM, FontWeight::NORMAL),
                                    );
                                },
                            );
                        },
                    );
                    Row(
                        Modifier::empty()
                            .fill_max_width()
                            .height(42.0)
                            .background(SURFACE.with_alpha(0.9))
                            .rounded_corners(21.0)
                            .padding(4.0),
                        RowSpec::new()
                            .horizontal_arrangement(LinearArrangement::SpacedBy(4.0))
                            .vertical_alignment(VerticalAlignment::CenterVertically),
                        move || {
                            for (i, label) in filters.iter().copied().enumerate() {
                                let filter = filter.clone();
                                tab_pill(label, selected_filter == i, move || filter.set(i));
                            }
                        },
                    );
                    for (i, n) in items.iter().enumerate() {
                        let matches_filter = match selected_filter {
                            1 => n.kind == 1,
                            2 => n.kind == 2,
                            3 => n.kind == 0,
                            _ => true,
                        };
                        if !matches_filter {
                            continue;
                        }
                        let follow = on_follow.clone();
                        let is_following = following.get(i).copied().unwrap_or(false);
                        notif_row(i, n, is_following, move || follow(i));
                    }
                },
            );
        },
    )
}

fn tab_pill(label: &'static str, active: bool, on_select: impl Fn() + 'static) -> NodeId {
    let on_select = std::rc::Rc::new(on_select);
    let bg = if active {
        SURFACE_2
    } else {
        SURFACE.with_alpha(0.0)
    };
    let fg = if active { TEXT } else { TEXT_DIM };
    Box(
        Modifier::empty()
            .weight(1.0)
            .fill_max_size()
            .background(bg)
            .rounded_corners(18.0)
            .clickable({
                let on_select = on_select.clone();
                move |_| on_select()
            }),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Text(
                label,
                Modifier::empty(),
                ts(
                    12.0,
                    fg,
                    if active {
                        FontWeight::BOLD
                    } else {
                        FontWeight::MEDIUM
                    },
                ),
            );
        },
    )
}

fn notif_row(
    idx: usize,
    n: &Notification,
    is_following: bool,
    on_follow: impl Fn() + 'static,
) -> NodeId {
    let (path, tint) = match n.kind {
        0 => (icons::PERSON_OUTLINE, ACCENT),
        1 => (icons::HEART, PRIMARY),
        2 => (icons::COMMENT, ACCENT),
        _ => (icons::TRENDING, Color::from_rgb_u8(255, 200, 60)),
    };
    let show_follow = n.kind == 0;
    let user = n.user;
    let text = n.text;
    let ago = n.ago;
    let variant = (idx % 5) as u8;
    let letter = first_letter(user);
    let on_follow = std::rc::Rc::new(on_follow);
    let col_w = if show_follow { 194.0 } else { 270.0 };
    Box(
        Modifier::empty()
            .fill_max_width()
            .height(72.0)
            .background(SURFACE.with_alpha(0.88))
            .border(
                1.0,
                TEXT.with_alpha(0.04),
                RoundedCornerShape::uniform(RADIUS_CARD),
            )
            .rounded_corners(RADIUS_CARD),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            let on_follow = on_follow.clone();
            Row(
                Modifier::empty().padding(10.0),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(12.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                move || {
                    let follow = on_follow.clone();
                    Box(
                        Modifier::empty().width(40.0).height(40.0),
                        BoxSpec::new().content_alignment(Alignment::CENTER),
                        move || {
                            crate::components::avatar(letter, variant, 40.0);
                            Box(
                                Modifier::empty()
                                    .width(18.0)
                                    .height(18.0)
                                    .background(BG)
                                    .rounded_corners(9.0),
                                BoxSpec::new().content_alignment(Alignment::CENTER),
                                move || {
                                    Icon(path, None, 11.0, tint);
                                },
                            );
                        },
                    );
                    Column(
                        Modifier::empty().width(col_w),
                        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(3.0)),
                        move || {
                            Row(
                                Modifier::empty(),
                                RowSpec::new()
                                    .horizontal_arrangement(LinearArrangement::SpacedBy(6.0))
                                    .vertical_alignment(VerticalAlignment::CenterVertically),
                                move || {
                                    Text(user, Modifier::empty(), ts(14.0, TEXT, FontWeight::BOLD));
                                    Text(
                                        ago,
                                        Modifier::empty(),
                                        ts(11.0, TEXT_FAINT, FontWeight::NORMAL),
                                    );
                                },
                            );
                            Text(
                                text,
                                Modifier::empty(),
                                ts(13.0, TEXT_DIM, FontWeight::NORMAL),
                            );
                        },
                    );
                    if show_follow {
                        let follow = follow.clone();
                        Box(
                            Modifier::empty().width(76.0),
                            BoxSpec::new().content_alignment(Alignment::CENTER),
                            move || {
                                let follow = follow.clone();
                                crate::components::follow_pill(is_following, move || follow());
                            },
                        );
                    }
                },
            );
        },
    )
}
