use cranpose::prelude::*;
use cranpose::widgets::Icon;
use cranpose_animation::{animateFloatAsState, spring};

use crate::components::entrance_layer;
use crate::data::{Creator, DISCOVER_TAGS, Sound, TRENDING, fmt_count};
use crate::icons;
use crate::scene::palette;
use crate::theme::*;

pub fn discover_screen(
    creators: Vec<Creator>,
    sounds: Vec<Sound>,
    on_follow: impl Fn(usize) + 'static,
) -> NodeId {
    let entered = rememberMutableStateOf(|| false);
    let target = if entered.get() { 1.0_f32 } else { 0.0_f32 };
    let progress = animateFloatAsState(target, spring(0.88, 220.0), "discover-enter").get();
    LaunchedEffect(0u8, move |_scope| entered.set(true));
    let scroll = remember(|| ScrollState::new(0.0)).with(|s| *s);

    let creators = std::rc::Rc::new(creators);
    let sounds = std::rc::Rc::new(sounds);
    let on_follow = std::rc::Rc::new(on_follow);
    Box(
        Modifier::empty()
            .fill_max_size()
            .graphics_layer_value(entrance_layer(progress)),
        BoxSpec::new(),
        move || {
            let creators = creators.clone();
            let sounds = sounds.clone();
            let on_follow = on_follow.clone();
            Column(
                Modifier::empty()
                    .fill_max_size()
                    .vertical_scroll(scroll, false)
                    .padding(GUTTER),
                ColumnSpec::new()
                    .horizontal_alignment(HorizontalAlignment::Start)
                    .vertical_arrangement(LinearArrangement::SpacedBy(14.0)),
                move || {
                    let creators = creators.clone();
                    let sounds = sounds.clone();
                    let on_follow = on_follow.clone();
                    Column(
                        Modifier::empty()
                            .fill_max_width()
                            .padding_each(4.0, 8.0, 4.0, 2.0),
                        ColumnSpec::new()
                            .horizontal_alignment(HorizontalAlignment::Start)
                            .vertical_arrangement(LinearArrangement::SpacedBy(3.0)),
                        || {
                            Text(
                                "Discover",
                                Modifier::empty(),
                                ts(26.0, TEXT, FontWeight::BOLD),
                            );
                            Text(
                                "Find your next favorite",
                                Modifier::empty(),
                                ts(13.0, TEXT_DIM, FontWeight::NORMAL),
                            );
                        },
                    );
                    search_bar();

                    Row(
                        Modifier::empty(),
                        RowSpec::new().horizontal_arrangement(LinearArrangement::SpacedBy(8.0)),
                        || {
                            trend_chip("Trending", icons::TRENDING);
                            trend_chip("Live", icons::LIVE);
                            trend_chip("Creators", icons::PERSON);
                        },
                    );

                    Text(
                        "Live now",
                        Modifier::empty(),
                        ts(16.0, TEXT, FontWeight::BOLD),
                    );

                    Row(
                        Modifier::empty().fill_max_width(),
                        RowSpec::new().horizontal_arrangement(LinearArrangement::SpacedBy(10.0)),
                        {
                            let creators = creators.clone();
                            move || {
                                for (i, c) in creators.iter().take(3).enumerate() {
                                    live_card(c.user, i as u8);
                                }
                            }
                        },
                    );

                    Text(
                        "Creators to follow",
                        Modifier::empty(),
                        ts(16.0, TEXT, FontWeight::BOLD),
                    );

                    for (i, c) in creators.iter().enumerate() {
                        let follow = on_follow.clone();
                        creator_row(c, move || follow(i));
                    }

                    Text(
                        "Trending sounds",
                        Modifier::empty(),
                        ts(16.0, TEXT, FontWeight::BOLD),
                    );

                    for s in sounds.iter() {
                        sound_row(s);
                    }

                    Text(
                        "Trending now",
                        Modifier::empty(),
                        ts(16.0, TEXT, FontWeight::BOLD),
                    );

                    for (i, (title, author, views)) in TRENDING.iter().enumerate() {
                        trending_row(i as u8, title, author, *views);
                    }

                    Text(
                        "Popular hashtags",
                        Modifier::empty(),
                        ts(16.0, TEXT, FontWeight::BOLD),
                    );

                    Row(
                        Modifier::empty(),
                        RowSpec::new().horizontal_arrangement(LinearArrangement::SpacedBy(8.0)),
                        || {
                            for (i, (tag, n)) in DISCOVER_TAGS.iter().take(6).enumerate() {
                                tag_chip(i as u8, tag, *n);
                            }
                        },
                    );
                },
            );
        },
    )
}

fn live_card(user: &'static str, variant: u8) -> NodeId {
    let p = palette(variant);
    let letter = crate::components::first_letter(user);
    Box(
        Modifier::empty()
            .width(122.0)
            .height(170.0)
            .background(p.a.with_alpha(0.84))
            .border(
                1.0,
                TEXT.with_alpha(0.16),
                RoundedCornerShape::uniform(RADIUS_TILE),
            )
            .rounded_corners(RADIUS_TILE),
        BoxSpec::new(),
        move || {
            Column(
                Modifier::empty().fill_max_size().padding(8.0),
                ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(6.0)),
                move || {
                    Box(
                        Modifier::empty()
                            .height(18.0)
                            .background(PRIMARY)
                            .rounded_corners(4.0),
                        BoxSpec::new().content_alignment(Alignment::CENTER),
                        || {
                            Text("LIVE", Modifier::empty(), ts(10.0, TEXT, FontWeight::BOLD));
                        },
                    );
                    Box(Modifier::empty().fill_max_width(), BoxSpec::new(), || {});
                    crate::components::avatar(letter, variant, 46.0);
                    Text(user, Modifier::empty(), ts(12.0, TEXT, FontWeight::BOLD));
                },
            );
        },
    )
}

fn sound_row(s: &Sound) -> NodeId {
    let p = palette(s.variant);
    let title = s.title;
    let author = s.author;
    let uses = s.uses;
    Box(
        Modifier::empty()
            .fill_max_width()
            .height(64.0)
            .background(SURFACE.with_alpha(0.88))
            .border(
                1.0,
                TEXT.with_alpha(0.05),
                RoundedCornerShape::uniform(RADIUS_CARD),
            )
            .rounded_corners(RADIUS_CARD),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Row(
                Modifier::empty().padding(10.0),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(12.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                move || {
                    Box(
                        Modifier::empty()
                            .width(36.0)
                            .height(36.0)
                            .background(p.a.with_alpha(0.22))
                            .rounded_corners(8.0),
                        BoxSpec::new().content_alignment(Alignment::CENTER),
                        move || {
                            Icon(icons::MUSIC, None, 18.0, ACCENT);
                        },
                    );
                    Column(
                        Modifier::empty().width(250.0),
                        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(2.0)),
                        move || {
                            Text(title, Modifier::empty(), ts(14.0, TEXT, FontWeight::BOLD));
                            Text(
                                author,
                                Modifier::empty(),
                                ts(11.0, TEXT_FAINT, FontWeight::NORMAL),
                            );
                        },
                    );
                    Text(
                        fmt_count(uses),
                        Modifier::empty(),
                        ts(12.0, TEXT_DIM, FontWeight::MEDIUM),
                    );
                },
            );
        },
    )
}

fn creator_row(c: &Creator, on_follow: impl Fn() + 'static) -> NodeId {
    let on_follow = std::rc::Rc::new(on_follow);
    let letter = crate::components::first_letter(c.user);
    let user = c.user;
    let handle = c.handle;
    let tagline = c.tagline;
    let followers = c.followers;
    let variant = c.variant;
    let verified = c.verified;
    let following = c.following;
    Box(
        Modifier::empty()
            .fill_max_width()
            .height(60.0)
            .background(SURFACE)
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
                    crate::components::avatar(letter, variant, 42.0);
                    Column(
                        Modifier::empty().width(210.0),
                        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(2.0)),
                        move || {
                            Row(
                                Modifier::empty(),
                                RowSpec::new()
                                    .horizontal_arrangement(LinearArrangement::SpacedBy(4.0))
                                    .vertical_alignment(VerticalAlignment::CenterVertically),
                                move || {
                                    Text(user, Modifier::empty(), ts(14.0, TEXT, FontWeight::BOLD));
                                    if verified {
                                        Icon(icons::VERIFIED, None, 13.0, ACCENT);
                                    }
                                },
                            );
                            Text(
                                tagline,
                                Modifier::empty(),
                                ts(11.0, TEXT_FAINT, FontWeight::NORMAL),
                            );
                        },
                    );
                    Column(
                        Modifier::empty().width(56.0),
                        ColumnSpec::new()
                            .horizontal_alignment(HorizontalAlignment::End)
                            .vertical_arrangement(LinearArrangement::SpacedBy(4.0)),
                        move || {
                            Text(
                                fmt_count(followers),
                                Modifier::empty(),
                                ts(11.0, TEXT_DIM, FontWeight::MEDIUM),
                            );
                            let follow = follow.clone();
                            crate::components::follow_pill(following, move || follow());
                        },
                    );
                    let _ = handle;
                },
            );
        },
    )
}

fn search_bar() -> NodeId {
    Box(
        Modifier::empty()
            .fill_max_width()
            .height(46.0)
            .background(SURFACE_2.with_alpha(0.86))
            .border(
                1.0,
                TEXT.with_alpha(0.08),
                RoundedCornerShape::uniform(23.0),
            )
            .rounded_corners(23.0),
        BoxSpec::new().content_alignment(Alignment::CENTER_START),
        || {
            Row(
                Modifier::empty().padding(12.0),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(8.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                || {
                    Icon(icons::SEARCH, None, 18.0, TEXT_FAINT);
                    Text(
                        "Search creators, sounds, hashtags",
                        Modifier::empty(),
                        ts(13.0, TEXT_FAINT, FontWeight::NORMAL),
                    );
                },
            );
        },
    )
}

fn trend_chip(label: &'static str, path: &'static str) -> NodeId {
    Box(
        Modifier::empty()
            .height(32.0)
            .background(SURFACE_2)
            .rounded_corners(16.0),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Row(
                Modifier::empty().padding(10.0),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(6.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                move || {
                    Icon(path, None, 15.0, PRIMARY);
                    Text(label, Modifier::empty(), ts(13.0, TEXT, FontWeight::MEDIUM));
                },
            );
        },
    )
}

fn trending_row(variant: u8, title: &'static str, _author: &'static str, views: u32) -> NodeId {
    let p = palette(variant);
    Box(
        Modifier::empty()
            .fill_max_width()
            .height(72.0)
            .background(SURFACE.with_alpha(0.88))
            .border(
                1.0,
                TEXT.with_alpha(0.05),
                RoundedCornerShape::uniform(RADIUS_CARD),
            )
            .rounded_corners(RADIUS_CARD),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Row(
                Modifier::empty().padding(10.0),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(12.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                move || {
                    Box(
                        Modifier::empty()
                            .width(24.0)
                            .height(24.0)
                            .background(p.a)
                            .rounded_corners(6.0),
                        BoxSpec::new().content_alignment(Alignment::CENTER),
                        move || {
                            Icon(icons::TRENDING, None, 14.0, TEXT);
                        },
                    );
                    Column(
                        Modifier::empty().width(230.0),
                        ColumnSpec::new().vertical_arrangement(LinearArrangement::SpacedBy(2.0)),
                        move || {
                            Text(title, Modifier::empty(), ts(14.0, TEXT, FontWeight::BOLD));
                            Text(
                                "@author",
                                Modifier::empty(),
                                ts(12.0, TEXT_FAINT, FontWeight::NORMAL),
                            );
                        },
                    );
                    Column(
                        Modifier::empty().width(60.0),
                        ColumnSpec::new()
                            .horizontal_alignment(HorizontalAlignment::End)
                            .vertical_arrangement(LinearArrangement::SpacedBy(2.0)),
                        move || {
                            Text(
                                fmt_count(views),
                                Modifier::empty(),
                                ts(13.0, TEXT, FontWeight::BOLD),
                            );
                            Text(
                                "views",
                                Modifier::empty(),
                                ts(11.0, TEXT_FAINT, FontWeight::NORMAL),
                            );
                        },
                    );
                },
            );
        },
    )
}

fn tag_chip(variant: u8, tag: &'static str, n: u32) -> NodeId {
    let p = palette(variant);
    Box(
        Modifier::empty()
            .height(34.0)
            .background(p.a.with_alpha(0.20))
            .rounded_corners(17.0),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Row(
                Modifier::empty().padding(12.0),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(6.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                move || {
                    Text(tag, Modifier::empty(), ts(13.0, ACCENT, FontWeight::MEDIUM));
                    Text(
                        fmt_count(n),
                        Modifier::empty(),
                        ts(11.0, TEXT_FAINT, FontWeight::NORMAL),
                    );
                },
            );
        },
    )
}
