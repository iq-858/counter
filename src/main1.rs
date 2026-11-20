mod components;
mod data;
mod icons;
mod scene;
mod screens;
mod theme;
mod video;

use cranpose::liquid::prelude::*;
use cranpose::prelude::*;
use cranpose_animation::animateFloatAsState;
use std::sync::OnceLock;

use data::posts;
use screens::{discover, feed, inbox, profile};
use theme::*;

fn root(use_liquid_bar: bool) -> NodeId {
    let use_liquid_bar = use_liquid_bar && liquid_bar_supported();
    let tab = rememberMutableStateOf(|| 0usize);
    let tab_motion = animateFloatAsState(
        tab.get() as f32,
        cranpose_animation::spring(0.86, 420.0),
        "screen-transition",
    )
    .get();
    let comments_for = rememberMutableStateOf(|| None::<usize>);
    let feed_posts = rememberMutableStateOf(posts);
    let discover_creators = rememberMutableStateOf(data::creators);
    let notifs = rememberMutableStateOf(data::notifications);
    let notif_following = rememberMutableStateOf(|| vec![false; data::notifications().len()]);

    Box(
        Modifier::empty().fill_max_size().background(BG),
        BoxSpec::new(),
        move || {
            let active_tab = tab.get();

            Box(
                Modifier::empty()
                    .fill_max_size()
                    .padding(0.0)
                    .graphics_layer_value({
                        let distance = (tab_motion - active_tab as f32).clamp(-1.0, 1.0);
                        GraphicsLayer {
                            alpha: 1.0 - distance.abs() * 0.08,
                            translation_x: distance * 26.0,
                            ..GraphicsLayer::DEFAULT
                        }
                    }),
                BoxSpec::new(),
                move || {
                    let fp = feed_posts.clone();
                    let cf = comments_for.clone();
                    let dc = discover_creators.clone();
                    let nf = notifs.clone();
                    let nfol = notif_following.clone();

                    let fp2 = fp.clone();
                    let cf2 = cf.clone();
                    let dc2 = dc.clone();
                    let nf2 = nf.clone();
                    let nfol2 = nfol.clone();

                    match active_tab {
                        0 => {
                            let fp = fp2.clone();
                            let cf = cf2.clone();
                            let on_like = {
                                let fp = fp.clone();
                                move |i: usize| {
                                    fp.update(|v| {
                                        if let Some(p) = v.get_mut(i) {
                                            p.liked = !p.liked;
                                            if p.liked {
                                                p.likes += 1;
                                            } else {
                                                p.likes = p.likes.saturating_sub(1);
                                            }
                                        }
                                    });
                                }
                            };
                            let on_comment = {
                                let cf = cf.clone();
                                move |i: usize| cf.set(Some(i))
                            };
                            let on_follow = move |i: usize| {
                                fp.update(|v| {
                                    if let Some(p) = v.get_mut(i) {
                                        p.following = !p.following;
                                    }
                                });
                            };
                            feed::feed_screen(fp.get(), on_like, on_comment, on_follow);
                        }
                        1 => {
                            let dc = dc2.clone();
                            let on_follow = move |i: usize| {
                                dc.update(|v| {
                                    if let Some(c) = v.get_mut(i) {
                                        c.following = !c.following;
                                    }
                                });
                            };
                            discover::discover_screen(dc.get(), data::sounds(), on_follow);
                        }
                        2 => {
                            let nfol = nfol2.clone();
                            let on_follow = move |i: usize| {
                                nfol.update(|v| {
                                    if let Some(f) = v.get_mut(i) {
                                        *f = !*f;
                                    }
                                });
                            };
                            inbox::inbox_screen(nf2.get(), nfol.get(), on_follow);
                        }
                        _ => {
                            profile::profile_screen();
                        }
                    }
                },
            );

            Box(
                Modifier::empty()
                    .fill_max_width()
                    .align(Alignment::BOTTOM_CENTER),
                BoxSpec::new(),
                move || {
                    if use_liquid_bar {
                        let tab_set = tab.clone();
                        LiquidTabBar(
                            Modifier::empty(),
                            LiquidTabBarSpec::default(),
                            active_tab,
                            move |i| tab_set.set(i),
                            |scope| {
                                scope.tab(icons::HOME, "Home");
                                scope.tab(icons::COMPASS, "Discover");
                                scope.tab(icons::INBOX, "Inbox");
                                scope.tab(icons::PERSON, "Profile");
                            },
                        );
                    } else {
                        let bar_active = active_tab;
                        components::nav_bar(bar_active, move |i| tab.set(i), move || tab.set(3));
                    }
                },
            );

            if let Some(idx) = comments_for.get() {
                let list = data::comments(idx);
                components::comment_sheet(idx, list, move || comments_for.set(None));
            }
        },
    )
}

fn liquid_bar_supported() -> bool {
    static FALLBACK_NOTICE: OnceLock<()> = OnceLock::new();

    let Some(gpu) = cranpose::gpu_context() else {
        FALLBACK_NOTICE.get_or_init(|| {
            eprintln!("Liquid tab bar disabled: GPU information is unavailable.");
        });
        return false;
    };
    let adapter = gpu.adapter.get_info();
    let name = adapter.name.to_ascii_lowercase();
    let legacy_intel = [
        "hd graphics 2000",
        "hd graphics 2500",
        "hd graphics 3000",
        "hd graphics 4000",
    ]
    .iter()
    .any(|model| name.contains(model));

    if legacy_intel {
        FALLBACK_NOTICE.get_or_init(|| {
            eprintln!(
                "Liquid tab bar disabled on {} because its legacy Intel GLSL driver can crash; using the compatible glass-style bar.",
                adapter.name
            );
        });
    }

    !legacy_intel
}

fn main() -> anyhow::Result<()> {

    if std::env::args().any(|a| a == "--fast-warm") {
        // Background pipeline threads contend for the WGL context and its
        // one-second lock timeout, which crashes the first frames on legacy
        // Intel drivers; restore them only when debugging warm-up behavior.
        unsafe { std::env::set_var("CRANPOSE_BACKGROUND_PIPELINES", "1") };
    } else {
        unsafe { std::env::set_var("CRANPOSE_BACKGROUND_PIPELINES", "0") };
    }
    let skip_media = std::env::args().any(|a| a == "--no-media");
    if !skip_media {
        let _ = cranpose_media::install();
    }
    // --liquid-bar requests the native Liquid tab bar; unsupported legacy
    // Intel adapters fall back to the GLES-safe glass-style bar.
    let use_liquid_bar = std::env::args().any(|a| a == "--liquid-bar");
    AppLauncher::new()
        .with_title("ClipTok")
        .with_size(420, 700)
        .with_developer_inspector(false)
        .with_gpu_requirements(cranpose::GpuRequirements {
            adapter_limits: true,
            ..Default::default()
        })
        .try_run(move || {
            if std::env::args().any(|a| a == "--bare") {
                root(false);
            } else {
                LiquidTheme(
                    LiquidThemeSpec {
                        scheme: SchemeMode::Dark,
                        accent: ACCENT,
                        ..Default::default()
                    },
                    || {
                        root(use_liquid_bar);
                    },
                );
            }
        })?;
    Ok(())
}
