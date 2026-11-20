use cranpose::liquid::{LiquidModalBottomSheet, rememberLiquidSheetState};
use cranpose::prelude::*;
use cranpose::widgets::Icon;
use cranpose_animation::{animateFloatAsState, spring};
use std::rc::Rc;

use crate::data::{Comment, Post, fmt_count};
use crate::icons;
use crate::scene::palette;
use crate::theme::*;

pub fn avatar(letter: &'static str, variant: u8, size: f32) -> NodeId {
    let p = palette(variant);
    Box(
        Modifier::empty()
            .width(size)
            .height(size)
            .background(p.a.with_alpha(0.85))
            .rounded_corners(size * 0.5),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Text(
                letter,
                Modifier::empty(),
                ts(size * 0.42, TEXT, FontWeight::BOLD),
            );
        },
    )
}

pub fn action_rail(
    post: &Post,
    on_like: impl Fn() + 'static,
    on_comment: impl Fn() + 'static,
    on_follow: impl Fn() + 'static,
) -> NodeId {
    let variant = post.variant;
    let p = palette(variant);
    let liked = post.liked;
    let likes = post.likes;
    let comments = post.comments;
    let shares = post.shares;
    let following = post.following;
    let letter = first_letter(post.user);
    let on_like = Rc::new(on_like);
    let on_comment = Rc::new(on_comment);
    let on_follow = Rc::new(on_follow);

    Box(
        Modifier::empty(),
        BoxSpec::new().content_alignment(Alignment::BOTTOM_END),
        move || {
            let on_like = on_like.clone();
            let on_comment = on_comment.clone();
            let on_follow = on_follow.clone();
            Column(
                Modifier::empty(),
                ColumnSpec::new()
                    .horizontal_alignment(HorizontalAlignment::CenterHorizontally)
                    .vertical_arrangement(LinearArrangement::SpacedBy(20.0)),
                move || {
                    avatar_badge(letter, variant, following, {
                        let follow = on_follow.clone();
                        move || follow()
                    });
                    rail_item(
                        icons::HEART,
                        fmt_count(likes),
                        if liked { PRIMARY } else { TEXT },
                        liked,
                        {
                            let like = on_like.clone();
                            move || like()
                        },
                    );
                    rail_item(icons::COMMENT, fmt_count(comments), TEXT, false, {
                        let cmt = on_comment.clone();
                        move || cmt()
                    });
                    rail_item(icons::SHARE, fmt_count(shares), TEXT, false, || {});
                    Box(
                        Modifier::empty()
                            .width(46.0)
                            .height(46.0)
                            .background(p.b.with_alpha(0.85))
                            .rounded_corners(23.0),
                        BoxSpec::new().content_alignment(Alignment::CENTER),
                        || {
                            Icon(icons::MUSIC, None, 22.0, BG);
                        },
                    );
                },
            );
        },
    )
}

/// Avatar with a follow badge. The badge collapses to a checkmark (with a
/// springy pop) once the creator is followed; unfollowing restores the plus.
fn avatar_badge(
    letter: &'static str,
    variant: u8,
    following: bool,
    on_follow: impl Fn() + 'static,
) -> NodeId {
    let on_follow = Rc::new(on_follow);
    Box(
        Modifier::empty(),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            let on_follow = on_follow.clone();
            Column(
                Modifier::empty(),
                ColumnSpec::new()
                    .horizontal_alignment(HorizontalAlignment::CenterHorizontally)
                    .vertical_arrangement(LinearArrangement::SpacedBy(6.0)),
                move || {
                    avatar(letter, variant, 46.0);
                    Box(
                        Modifier::empty()
                            .width(18.0)
                            .height(18.0)
                            .scale(if following { 0.94 } else { 1.0 })
                            .background(if following { SURFACE_2 } else { PRIMARY })
                            .rounded_corners(9.0)
                            .clickable({
                                let follow = on_follow.clone();
                                move |_| follow()
                            }),
                        BoxSpec::new().content_alignment(Alignment::CENTER),
                        move || {
                            let (path, color) = if following {
                                (icons::CHECK, TEXT_DIM)
                            } else {
                                (icons::PLUS, TEXT)
                            };
                            Icon(path, None, 12.0, color);
                        },
                    );
                },
            );
        },
    )
}

fn rail_item(
    path: &'static str,
    count: String,
    color: Color,
    liked: bool,
    on_click: impl Fn() + 'static,
) -> NodeId {
    let on_click = Rc::new(on_click);
    Box(
        Modifier::empty(),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            let on_click = on_click.clone();
            let count = count.clone();
            Column(
                Modifier::empty(),
                ColumnSpec::new()
                    .horizontal_alignment(HorizontalAlignment::CenterHorizontally)
                    .vertical_arrangement(LinearArrangement::SpacedBy(4.0)),
                move || {
                    Box(
                        Modifier::empty().clickable({
                            let click = on_click.clone();
                            move |_| click()
                        }),
                        BoxSpec::new(),
                        move || {
                            Box(
                                Modifier::empty()
                                    .scale(if liked { 1.06 } else { 1.0 })
                                    .padding_each(2.0, 2.0, 2.0, 2.0),
                                BoxSpec::new(),
                                move || {
                                    Icon(path, None, 34.0, color);
                                },
                            );
                        },
                    );
                    Box(Modifier::empty(), BoxSpec::new(), {
                        let c = count.clone();
                        move || {
                            let weight = if liked {
                                FontWeight::BOLD
                            } else {
                                FontWeight::MEDIUM
                            };
                            Text(c.clone(), Modifier::empty(), ts(13.0, TEXT, weight));
                        }
                    });
                },
            );
        },
    )
}

pub fn first_letter(name: &str) -> &'static str {
    match name.as_bytes().first() {
        Some(b'A') => "A",
        Some(b'B') => "B",
        Some(b'C') => "C",
        Some(b'D') => "D",
        Some(b'E') => "E",
        Some(b'K') => "K",
        Some(b'M') => "M",
        Some(b'N') => "N",
        Some(b'R') => "R",
        Some(b'S') => "S",
        _ => "?",
    }
}

pub fn post_overlay(post: &Post, on_follow: impl Fn() + 'static) -> NodeId {
    let caption = post.caption;
    let tags = post.tags;
    let music = post.music;
    let verified = post.verified;
    let variant = post.variant;
    let user = post.user;
    let letter = first_letter(user);
    let following = post.following;
    let on_follow = Rc::new(on_follow);

    Box(
        Modifier::empty(),
        BoxSpec::new().content_alignment(Alignment::BOTTOM_START),
        move || {
            let on_follow = on_follow.clone();
            Column(
                Modifier::empty().width(300.0).padding(16.0),
                ColumnSpec::new()
                    .horizontal_alignment(HorizontalAlignment::Start)
                    .vertical_arrangement(LinearArrangement::SpacedBy(8.0)),
                move || {
                    let follow = on_follow.clone();
                    Row(
                        Modifier::empty(),
                        RowSpec::new()
                            .horizontal_arrangement(LinearArrangement::SpacedBy(8.0))
                            .vertical_alignment(VerticalAlignment::CenterVertically),
                        move || {
                            avatar(letter, variant, 26.0);
                            Text(user, Modifier::empty(), ts(15.0, TEXT, FontWeight::BOLD));
                            if verified {
                                Icon(icons::VERIFIED, None, 15.0, ACCENT);
                            }
                            let follow = follow.clone();
                            follow_pill(following, move || follow());
                        },
                    );
                    Text(
                        caption,
                        Modifier::empty(),
                        ts(14.0, TEXT, FontWeight::NORMAL),
                    );
                    Text(
                        tags,
                        Modifier::empty(),
                        ts(13.0, ACCENT, FontWeight::MEDIUM),
                    );
                    Row(
                        Modifier::empty(),
                        RowSpec::new()
                            .horizontal_arrangement(LinearArrangement::SpacedBy(6.0))
                            .vertical_alignment(VerticalAlignment::CenterVertically),
                        move || {
                            Icon(icons::MUSIC, None, 14.0, TEXT);
                            Text(
                                music,
                                Modifier::empty(),
                                ts(12.0, TEXT_DIM, FontWeight::NORMAL),
                            );
                        },
                    );
                },
            );
        },
    )
}

/// Small outlined "Follow"/"Following" toggle that sits next to the handle.
pub fn follow_pill(following: bool, on_follow: impl Fn() + 'static) -> NodeId {
    let on_follow = Rc::new(on_follow);
    let (label, border, fg) = if following {
        ("Following", TEXT_FAINT, TEXT_DIM)
    } else {
        ("Follow", PRIMARY, TEXT)
    };
    Box(
        Modifier::empty()
            .height(24.0)
            .rounded_corners(12.0)
            .background(SURFACE.with_alpha(0.35))
            .clickable({
                let follow = on_follow.clone();
                move |_| follow()
            }),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Row(
                Modifier::empty().padding_horizontal(12.0),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(4.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                move || {
                    if !following {
                        Icon(icons::PLUS, None, 12.0, border);
                    }
                    Text(label, Modifier::empty(), ts(12.0, fg, FontWeight::MEDIUM));
                },
            );
        },
    )
}

/// TikTok-style double-tap heart: pops in with an overshoot, holds, then fades
/// while scaling up. A ring of particles flies outward for a physical feel.
pub fn heart_burst(x: f32, y: f32, t: f32) -> NodeId {
    let t = t.clamp(0.0, 1.0);
    // Pop over the first 30% with a slight overshoot (damped sine).
    let pop = (t / 0.30).min(1.0);
    let overshoot = 1.0 - (-6.0 * pop).exp() * (6.0 * pop).cos();
    let scale = 0.3 + 1.05 * overshoot.max(0.0) + 0.30 * t;
    let alpha = if t < 0.55 {
        1.0
    } else {
        (1.0 - (t - 0.55) / 0.45).clamp(0.0, 1.0)
    };
    // Particles travel outward more slowly than the heart pops.
    let ring_t = ((t - 0.05).max(0.0) / 0.55).clamp(0.0, 1.0);
    let ring_radius = 20.0 + 78.0 * ring_t;
    let ring_alpha = (1.0 - ring_t) * 0.9;

    Box(
        Modifier::empty()
            .offset(x - 90.0, y - 90.0)
            .width(180.0)
            .height(180.0),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            // Ring particles: 8 dots on a circle plus 2 leading sparkles.
            for i in 0..10 {
                let ang = i as f32 * std::f32::consts::TAU / 10.0 - 0.3;
                let px = 90.0 + ring_radius * ang.cos();
                let py = 90.0 + ring_radius * ang.sin();
                let dot = 5.0 + 3.0 * (1.0 - ring_t);
                Box(
                    Modifier::empty()
                        .offset(px - dot * 0.5, py - dot * 0.5)
                        .width(dot)
                        .height(dot)
                        .background(PRIMARY.with_alpha(ring_alpha))
                        .rounded_corners(dot * 0.5),
                    BoxSpec::new(),
                    || {},
                );
            }
            Box(
                Modifier::empty().scale(scale),
                BoxSpec::new().content_alignment(Alignment::CENTER),
                move || {
                    Icon(icons::HEART, None, 110.0, TEXT.with_alpha(alpha));
                },
            );
        },
    )
}

/// Determinate, TikTok-style segmented scrubber driven by the real playback
/// fraction (0..1) from the media player.
pub fn video_progress(frac: f32) -> NodeId {
    const SEGMENTS: usize = 12;
    let filled = (frac.clamp(0.0, 1.0) * SEGMENTS as f32) as usize;
    Box(
        Modifier::empty().fill_max_width().height(34.0).padding(8.0),
        BoxSpec::new().content_alignment(Alignment::CENTER),
        move || {
            Row(
                Modifier::empty().fill_max_width(),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(3.0))
                    .vertical_alignment(VerticalAlignment::CenterVertically),
                move || {
                    for i in 0..SEGMENTS {
                        let on = i < filled;
                        Box(
                            Modifier::empty()
                                .weight(1.0)
                                .height(2.5)
                                .background(if on {
                                    TEXT
                                } else {
                                    TEXT_FAINT.with_alpha(0.35)
                                })
                                .rounded_corners(1.25),
                            BoxSpec::new(),
                            || {},
                        );
                    }
                },
            );
        },
    )
}

/// Apple-crafted bottom bar: a layered translucent "frosted" material with a
/// top hairline, plus a spring-animated ink pill that glides to the active tab.
/// GLES-safe (no real backdrop blur) — depth is faked with stacked alpha layers.
pub fn nav_bar(
    active: usize,
    on_select: impl Fn(usize) + 'static,
    on_profile: impl Fn() + 'static,
) -> NodeId {
    const SLOTS: usize = 4;
    const BAR_H: f32 = 64.0;
    let items: [(&'static str, &'static str); SLOTS] = [
        (icons::HOME, "Home"),
        (icons::COMPASS, "Discover"),
        (icons::INBOX, "Inbox"),
        (icons::PERSON, "Profile"),
    ];
    let on_select = Rc::new(on_select);
    let on_profile = Rc::new(on_profile);

    // The sliding ink pill: a springy, iOS-feeling glide between slots.
    let pill = animateFloatAsState(active as f32, spring(0.72, 320.0), "nav_pill");
    let pill_f = Rc::new(pill);

    Box(
        Modifier::empty()
            .fill_max_width()
            .height(BAR_H + 18.0)
            .padding_each(14.0, 0.0, 14.0, 8.0),
        BoxSpec::new().content_alignment(Alignment::BOTTOM_CENTER),
        move || {
            let select0 = on_select.clone();
            let profile0 = on_profile.clone();
            let pill0 = pill_f.clone();

            Box(
                Modifier::empty()
                    .fill_max_width()
                    .height(BAR_H)
                    .background(SURFACE.with_alpha(0.48))
                    .border(
                        1.0,
                        TEXT.with_alpha(0.30),
                        RoundedCornerShape::uniform(28.0),
                    )
                    .rounded_corners(28.0),
                BoxSpec::new().content_alignment(Alignment::TOP_CENTER),
                move || {
                    let select1 = select0.clone();
                    let profile1 = profile0.clone();
                    let pill1 = pill0.clone();
                    Box(
                        Modifier::empty()
                            .fill_max_width()
                            .height(1.0)
                            .background(TEXT.with_alpha(0.42)),
                        BoxSpec::new(),
                        || {},
                    );
                    Box(
                        Modifier::empty()
                            .fill_max_width()
                            .height(26.0)
                            .background(TEXT.with_alpha(0.055)),
                        BoxSpec::new(),
                        || {},
                    );
                    Box(
                        Modifier::empty()
                            .fill_max_size()
                            .padding_each(8.0, 4.0, 8.0, 4.0),
                        BoxSpec::new(),
                        move || {
                            let select2 = select1.clone();
                            let profile2 = profile1.clone();
                            let pill2 = pill1.clone();
                            Box(
                                Modifier::empty().fill_max_size(),
                                BoxSpec::new().content_alignment(Alignment::CENTER),
                                move || {
                                    let select_base = select2.clone();
                                    let profile_base = profile2.clone();
                                    let ink_x = pill2.get();

                                    // Sliding ink pill behind the active slot.
                                    let slot_w = 1.0 / SLOTS as f32;
                                    let pill_center = (ink_x + 0.5) * slot_w;
                                    Box(
                                        Modifier::empty()
                                            .fill_max_size()
                                            .offset(0.0, 0.0)
                                            .padding(0.0),
                                        BoxSpec::new().content_alignment(Alignment::CENTER_START),
                                        {
                                            move || {
                                                let _ = pill_center;
                                                // Fractional "align" via a spacer row.
                                                Row(
                                                    Modifier::empty().fill_max_size(),
                                                    RowSpec::new().horizontal_arrangement(
                                                        LinearArrangement::Start,
                                                    ),
                                                    move || {
                                                        // Left spacer takes (center - halfpill) weight.
                                                        const PILL_W: f32 = 0.30;
                                                        let left = (pill_center - PILL_W * 0.5)
                                                            .clamp(0.0, 1.0 - PILL_W);
                                                        let right = 1.0 - left - PILL_W;
                                                        Box(
                                                            Modifier::empty()
                                                                .weight(left.max(0.0001))
                                                                .height(1.0),
                                                            BoxSpec::new(),
                                                            || {},
                                                        );
                                                        Box(
                                                            Modifier::empty()
                                                                .weight(PILL_W)
                                                                .height(42.0)
                                                                .background(ACCENT.with_alpha(0.18))
                                                                .border(
                                                                    1.0,
                                                                    TEXT.with_alpha(0.42),
                                                                    RoundedCornerShape::uniform(
                                                                        21.0,
                                                                    ),
                                                                )
                                                                .rounded_corners(21.0),
                                                            BoxSpec::new(),
                                                            || {
                                                                Box(
                                                                    Modifier::empty()
                                                                        .fill_max_width()
                                                                        .height(1.0)
                                                                        .background(
                                                                            TEXT.with_alpha(0.38),
                                                                        ),
                                                                    BoxSpec::new(),
                                                                    || {},
                                                                );
                                                            },
                                                        );
                                                        Box(
                                                            Modifier::empty()
                                                                .weight(right.max(0.0001))
                                                                .height(1.0),
                                                            BoxSpec::new(),
                                                            || {},
                                                        );
                                                    },
                                                );
                                            }
                                        },
                                    );

                                    // Tab icons + labels, drawn above the pill.
                                    Row(
                                        Modifier::empty().fill_max_size(),
                                        RowSpec::new()
                                            .horizontal_arrangement(LinearArrangement::SpaceEvenly)
                                            .vertical_alignment(
                                                VerticalAlignment::CenterVertically,
                                            ),
                                        move || {
                                            for i in 0..SLOTS {
                                                let (path, name) = items[i];
                                                let select = select_base.clone();
                                                let profile = profile_base.clone();
                                                let is_active = i == active;
                                                let color = if is_active { TEXT } else { TEXT_DIM };
                                                let icon_scale = animateFloatAsState(
                                                    if is_active { 1.08 } else { 0.94 },
                                                    spring(0.78, 480.0),
                                                    "nav_icon",
                                                )
                                                .get();
                                                Box(
                                                    Modifier::empty().width(64.0).clickable(
                                                        move |_| {
                                                            if i == 3 {
                                                                profile()
                                                            } else {
                                                                select(i)
                                                            }
                                                        },
                                                    ),
                                                    BoxSpec::new()
                                                        .content_alignment(Alignment::CENTER),
                                                    move || {
                                                        Column(
                                                            Modifier::empty(),
                                                            ColumnSpec::new()
                                                                .horizontal_alignment(
                                                                    HorizontalAlignment::CenterHorizontally,
                                                                )
                                                                .vertical_arrangement(
                                                                    LinearArrangement::SpacedBy(3.0),
                                                                ),
                                                            move || {
                                                                Box(
                                                                    Modifier::empty()
                                                                        .scale(icon_scale),
                                                                    BoxSpec::new(),
                                                                    move || {
                                                                        Icon(path, None, 24.0, color);
                                                                    },
                                                                );
                                                                Text(
                                                                    name,
                                                                    Modifier::empty(),
                                                                    ts(
                                                                        9.5,
                                                                        color,
                                                                        if is_active {
                                                                            FontWeight::BOLD
                                                                        } else {
                                                                            FontWeight::MEDIUM
                                                                        },
                                                                    ),
                                                                );
                                                            },
                                                        );
                                                    },
                                                );
                                            }
                                        },
                                    );
                                },
                            );
                        },
                    );
                },
            );
        },
    )
}

pub fn comment_sheet(seed: usize, list: Vec<Comment>, on_close: impl Fn() + 'static) -> NodeId {
    let sheet = rememberLiquidSheetState(false);
    let close = Rc::new(on_close);
    let list = Rc::new(list);
    Box(
        Modifier::empty()
            .fill_max_size()
            .background(BG.with_alpha(0.66)),
        BoxSpec::new().content_alignment(Alignment::BOTTOM_CENTER),
        move || {
            let close_panel = close.clone();
            let sheet_state = sheet.clone();
            let list = list.clone();
            LiquidModalBottomSheet(
                sheet_state,
                move || close_panel(),
                move || {
                    let rows = list.clone();
                    Column(
                        Modifier::empty()
                            .fill_max_width()
                            .height(470.0)
                            .background(SURFACE)
                            .rounded_corners(18.0),
                        ColumnSpec::new()
                            .horizontal_alignment(HorizontalAlignment::Start)
                            .vertical_arrangement(LinearArrangement::SpacedBy(0.0)),
                        move || {
                            Box(
                                Modifier::empty().fill_max_width().height(52.0),
                                BoxSpec::new().content_alignment(Alignment::CENTER),
                                || {
                                    Text(
                                        "Comments",
                                        Modifier::empty(),
                                        ts(15.0, TEXT, FontWeight::BOLD),
                                    );
                                },
                            );
                            Box(
                                Modifier::empty()
                                    .fill_max_width()
                                    .height(1.0)
                                    .background(DIVIDER),
                                BoxSpec::new(),
                                || {},
                            );
                            let rows = rows.clone();
                            let scroll = remember(|| ScrollState::new(0.0)).with(|s| *s);
                            Column(
                                Modifier::empty()
                                    .fill_max_width()
                                    .height(416.0)
                                    .vertical_scroll(scroll, false),
                                ColumnSpec::new()
                                    .horizontal_alignment(HorizontalAlignment::Start)
                                    .vertical_arrangement(LinearArrangement::SpacedBy(0.0)),
                                move || {
                                    let rows = rows.clone();
                                    for (idx, c) in rows.iter().enumerate() {
                                        comment_row(idx, c, seed);
                                    }
                                },
                            );
                        },
                    );
                },
            );
        },
    )
}

fn comment_row(idx: usize, c: &Comment, seed: usize) -> NodeId {
    let user = c.user;
    let text = c.text;
    let base_likes = c.likes;
    let ago = c.ago;
    let variant = ((idx + seed) % 5) as u8;
    let letter = first_letter(user);
    let liked0 = (idx + seed) % 3 == 0;

    Box(
        Modifier::empty()
            .fill_max_width()
            .height(58.0)
            .padding(12.0),
        BoxSpec::new().content_alignment(Alignment::TOP_START),
        move || {
            let liked = rememberMutableStateOf(move || liked0);
            let pop = animateFloatAsState(
                if liked.get() { 1.0 } else { 0.0 },
                spring(0.5, 320.0),
                "comment_like",
            );
            let liked_now = liked.get();
            let heart_color = if liked_now { PRIMARY } else { TEXT_FAINT };
            let heart_scale = 1.0 + pop.get() * 0.28;

            Row(
                Modifier::empty(),
                RowSpec::new()
                    .horizontal_arrangement(LinearArrangement::SpacedBy(10.0))
                    .vertical_alignment(VerticalAlignment::Top),
                move || {
                    avatar(letter, variant, 36.0);
                    Column(
                        Modifier::empty().width(232.0),
                        ColumnSpec::new()
                            .horizontal_alignment(HorizontalAlignment::Start)
                            .vertical_arrangement(LinearArrangement::SpacedBy(3.0)),
                        move || {
                            Row(
                                Modifier::empty(),
                                RowSpec::new()
                                    .horizontal_arrangement(LinearArrangement::SpacedBy(6.0))
                                    .vertical_alignment(VerticalAlignment::CenterVertically),
                                move || {
                                    Text(
                                        user,
                                        Modifier::empty(),
                                        ts(13.0, TEXT_DIM, FontWeight::MEDIUM),
                                    );
                                    Text(
                                        ago,
                                        Modifier::empty(),
                                        ts(11.0, TEXT_FAINT, FontWeight::NORMAL),
                                    );
                                },
                            );
                            Text(text, Modifier::empty(), ts(14.0, TEXT, FontWeight::NORMAL));
                            Box(
                                Modifier::empty()
                                    .height(20.0)
                                    .rounded_corners(10.0)
                                    .clickable(|_| {}),
                                BoxSpec::new().content_alignment(Alignment::CENTER),
                                || {
                                    Text(
                                        "Reply",
                                        Modifier::empty(),
                                        ts(12.0, TEXT_FAINT, FontWeight::MEDIUM),
                                    );
                                },
                            );
                        },
                    );
                    Column(
                        Modifier::empty().width(32.0),
                        ColumnSpec::new()
                            .horizontal_alignment(HorizontalAlignment::CenterHorizontally),
                        move || {
                            let toggle = {
                                let liked = liked.clone();
                                move |_| liked.set(!liked.get())
                            };
                            let likes_show = if liked.get() {
                                fmt_count(base_likes + 1)
                            } else {
                                fmt_count(base_likes)
                            };
                            Box(
                                Modifier::empty()
                                    .size(Size::new(32.0, 32.0))
                                    .clickable(toggle),
                                BoxSpec::new().content_alignment(Alignment::CENTER),
                                move || {
                                    Box(
                                        Modifier::empty().scale(heart_scale),
                                        BoxSpec::new(),
                                        move || {
                                            Icon(icons::HEART, None, 16.0, heart_color);
                                        },
                                    );
                                },
                            );
                            Text(
                                likes_show,
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

/// Screen entrance: fade in while sliding up a few pixels. `p` runs 0 → 1.
pub fn entrance_layer(p: f32) -> GraphicsLayer {
    let p = p.clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - p) * (1.0 - p);
    GraphicsLayer {
        alpha: eased,
        translation_y: (1.0 - eased) * 16.0,
        ..GraphicsLayer::DEFAULT
    }
}
