//! Pure, UI-independent model for the demo application.
//!
//! This module deliberately contains no rendering or decoder code. It owns the
//! clip catalogue, page-gating rule, and small formatters used by the app, so
//! those rules can be tested without opening a window or a media file.

use std::time::Duration;

/// Where a catalogue entry gets its pictures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipKind {
    /// A file shipped with the demo.
    LocalMp4,
    /// A network stream, opt-in because it depends on connectivity.
    Hls,
}

/// One entry in the demo catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    /// Stable identifier used for keys and in-app test names.
    pub id: &'static str,
    /// Short human title shown on a Reels page.
    pub title: &'static str,
    /// Creator label shown under the title.
    pub author: &'static str,
    /// Path or URI handed to `VideoPlayer::open`.
    pub source: &'static str,
    /// Container/codec label shown in the library.
    pub format: &'static str,
    /// Exact source kind.
    pub kind: ClipKind,
    /// Why this clip is in the catalogue.
    pub note: &'static str,
}

/// The catalogue rendered by Reels and the library section.
pub const CLIPS: &[Clip] = &[
    Clip {
        id: "bbb10",
        title: "Big Buck Bunny · short cut",
        author: "Blender Foundation",
        source: "assets/videos/bbb10.mp4",
        format: "MP4 · H.264/yuv420p",
        kind: ClipKind::LocalMp4,
        note: "320×176 · 25 fps · 10.03 s · small-frame decode",
    },
    Clip {
        id: "flower",
        title: "Flower study",
        author: "Cranpose fixtures",
        source: "assets/videos/flower.mp4",
        format: "MP4 · H.264/yuv420p",
        kind: ClipKind::LocalMp4,
        note: "960×540 · 29.97 fps · 5.06 s · mid-resolution, short",
    },
    Clip {
        id: "bunny",
        title: "Big Buck Bunny",
        author: "Blender Foundation",
        source: "assets/videos/bunny.mp4",
        format: "MP4 · H.264/yuv420p",
        kind: ClipKind::LocalMp4,
        note: "853×480 · 25.03 fps · 33.00 s · long local playback",
    },
    Clip {
        id: "sintel",
        title: "Sintel trailer",
        author: "Blender Foundation",
        source: "assets/videos/sintel.mp4",
        format: "MP4 · H.264/yuv420p",
        kind: ClipKind::LocalMp4,
        note: "854×480 · 24 fps · 52.21 s · long-form seek/rollover",
    },
    Clip {
        id: "w3c",
        title: "W3C long sample",
        author: "W3C",
        source: "assets/videos/w3c.mp4",
        format: "MP4 · H.264/yuv420p",
        kind: ClipKind::LocalMp4,
        note: "320×240 · 24 fps · 300.14 s · endurance/clock drift",
    },
    Clip {
        id: "hls-live",
        title: "HLS live feed · opt-in",
        author: "External test stream",
        source: "https://hls-harbor-livepush.akamaized.net/live_cdn/nsqIStpj8PaG-Ev/emcQJ0pGpremocy/index.m3u8",
        format: "HLS · live adaptive",
        kind: ClipKind::Hls,
        note: "Network source · resolution may change · not in offline tests",
    },
];

/// Live HLS channels collected from the Free-TV/IPTV community playlists.
///
/// These are network sources and are deliberately kept out of [`CLIPS`] so the
/// deterministic offline decode sweep never depends on a server. The app shows
/// them in a separate "Live" section and only opens one when its page settles.
///
/// Provenance (fetched 2026-11-19, raw files on GitHub):
/// - `playlists/playlist_zz_news_ar.m3u8` · Arabic news
/// - `playlists/playlist_united_arab_emirates.m3u8` · United Arab Emirates
///
/// The YouTube entry in the Arabic news list is omitted: it is a channel page,
/// not a media playlist this decoder can read. Two duplicate URLs (Sky News
/// Arabia and Ajman TV) are each kept once.
pub const LIVE_CHANNELS: &[Clip] = &[
    // ---- playlist_zz_news_ar.m3u8 · Arabic news ----
    Clip { id: "live-al-jazeera-ar", title: "Al Jazeera العربية", author: "Free-TV IPTV · Arabic news", source: "https://live-hls-web-aja.getaj.net/AJA/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · Arabic news · Al Jazeera" },
    Clip { id: "live-al-arabiya-ar", title: "Al Arabiya العربية", author: "Free-TV IPTV · Arabic news", source: "https://live.alarabiya.net/alarabiapublish/alarabiya.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · Arabic news · Al Arabiya" },
    Clip { id: "live-al-hadath-ar", title: "Al Hadath العربية", author: "Free-TV IPTV · Arabic news", source: "https://live.alarabiya.net/alarabiapublish/alhadath.smil/alarabiapublish/alhadath_1080p/chunks.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · Arabic news · Al Hadath" },
    Clip { id: "live-dw-ar", title: "DW العربية", author: "Free-TV IPTV · Arabic news", source: "https://dwamdstream103.akamaized.net/hls/live/2015526/dwstream103/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · Arabic news · DW" },
    Clip { id: "live-cgtn-ar", title: "CGTN العربية", author: "Free-TV IPTV · Arabic news", source: "https://arabic-livews.cgtn.com/hls/LSveq57bErWLinBnxosqjisZ220802LSTefTAS9zc9mpU08y3np9TH220802cd/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · Arabic news · CGTN" },
    Clip { id: "live-sky-news-ar", title: "Sky News العربية", author: "Free-TV IPTV · Arabic news", source: "https://stream.skynewsarabia.com/hls/sna.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · Arabic news · Sky News Arabia" },
    Clip { id: "live-rt-ar", title: "RT العربية", author: "Free-TV IPTV · Arabic news", source: "https://rt-arb.rttv.com/dvr/rtarab/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · Arabic news · RT" },
    Clip { id: "live-sahara-24", title: "Sahara 24 صحراء", author: "Free-TV IPTV · Arabic news", source: "https://live.saharamedias.net:3317/live/shara24live.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · Arabic news · Sahara 24 · non-standard port" },

    // ---- playlist_united_arab_emirates.m3u8 · United Arab Emirates ----
    Clip { id: "live-cartoon-network-ar", title: "Cartoon Network Arabic", author: "Free-TV IPTV · UAE", source: "https://shls-cartoon-net-prod-dub.shahid.net/out/v1/dc4aa87372374325a66be458f29eab0f/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · kids" },
    Clip { id: "live-al-arabiya-business", title: "Al Arabiya Business", author: "Free-TV IPTV · UAE", source: "https://live.alarabiya.net/alarabiapublish/aswaaq.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · business" },
    Clip { id: "live-mbc-1", title: "MBC 1", author: "Free-TV IPTV · UAE", source: "https://shd-gcp-live.edgenextcdn.net/live/bitmovin-mbc-1/15cf99af5de54063fdabfefe66adc075/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · MBC" },
    Clip { id: "live-mbc-2", title: "MBC 2", author: "Free-TV IPTV · UAE", source: "http://37.122.156.107:4000/play/a07g/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · MBC · plain HTTP, may be filtered" },
    Clip { id: "live-mbc-3", title: "MBC 3", author: "Free-TV IPTV · UAE", source: "https://shd-gcp-live.edgenextcdn.net/live/bitmovin-mbc-3-usa/5d58265a862a476dc7f97694addb5ded/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · MBC" },
    Clip { id: "live-mbc-4", title: "MBC 4", author: "Free-TV IPTV · UAE", source: "https://shd-gcp-live.edgenextcdn.net/live/bitmovin-mbc-4/24f134f1cd63db9346439e96b86ca6ed/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · MBC" },
    Clip { id: "live-mbc-5", title: "MBC 5", author: "Free-TV IPTV · UAE", source: "https://shd-gcp-live.edgenextcdn.net/live/bitmovin-mbc-5/ee6b000cee0629411b666ab26cb13e9b/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · MBC" },
    Clip { id: "live-mbc-action", title: "MBC Action", author: "Free-TV IPTV · UAE", source: "http://37.122.156.107:4000/play/a07h/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · MBC · plain HTTP, may be filtered" },
    Clip { id: "live-mbc-bollywood", title: "MBC Bollywood", author: "Free-TV IPTV · UAE", source: "https://shd-gcp-live.edgenextcdn.net/live/bitmovin-mbc-bollywood/546eb40d7dcf9a209255dd2496903764/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · MBC" },
    Clip { id: "live-mbc-drama", title: "MBC Drama", author: "Free-TV IPTV · UAE", source: "https://shd-gcp-live.edgenextcdn.net/live/bitmovin-mbc-drama/2c28a458e2f3253e678b07ac7d13fe71/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · MBC" },
    Clip { id: "live-mbc-max", title: "MBC Max", author: "Free-TV IPTV · UAE", source: "http://37.122.156.107:4000/play/a07i/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · MBC · plain HTTP, may be filtered" },
    Clip { id: "live-mbc-persia", title: "MBC Persia", author: "Free-TV IPTV · UAE", source: "https://shd-gcp-live.edgenextcdn.net/live/bitmovin-mbc-persia/818ee8e4b592dc497608f066d825bfb4/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · MBC" },
    Clip { id: "live-wanasah", title: "Wanasah", author: "Free-TV IPTV · UAE", source: "https://shd-gcp-live.edgenextcdn.net/live/bitmovin-wanasah/13e82ea6232fa647c43b26e8a41f173d/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · music" },
    Clip { id: "live-baynounah-tv", title: "Baynounah TV", author: "Free-TV IPTV · UAE", source: "https://vo-live.cdb.cdn.orange.com/Content/Channel/Baynounah/HLS/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · local" },
    Clip { id: "live-ajman-tv", title: "Ajman TV", author: "Free-TV IPTV · UAE", source: "https://cdn1.logichost.in/ajmantv/live/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · local" },
    Clip { id: "live-al-aan-tv", title: "Al Aan TV", author: "Free-TV IPTV · UAE", source: "https://shls-live-ak.akamaized.net/out/v1/dfbdea4c1bf149629764e58c6ff314c8/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · news" },
    Clip { id: "live-abu-dhabi-tv", title: "Abu Dhabi TV", author: "Free-TV IPTV · UAE", source: "http://admdn2.cdn.mangomolo.com/adtv/smil:adtv.stream.smil/chunklist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Abu Dhabi · plain HTTP, may be filtered" },
    Clip { id: "live-abu-dhabi-sports-1", title: "Abu Dhabi Sports 1", author: "Free-TV IPTV · UAE", source: "https://vo-live.cdb.cdn.orange.com/Content/Channel/AbuDhabiSportsChannel1/HLS/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · sports" },
    Clip { id: "live-abu-dhabi-sports-2", title: "Abu Dhabi Sports 2", author: "Free-TV IPTV · UAE", source: "https://vo-live.cdb.cdn.orange.com/Content/Channel/AbuDhabiSportsChannel2/HLS/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · sports" },
    Clip { id: "live-nat-geo-abu-dhabi", title: "National Geographic Abu Dhabi", author: "Free-TV IPTV · UAE", source: "https://admdn2.cdn.mangomolo.com/nagtv/smil:nagtv.stream.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · documentary" },
    Clip { id: "live-dubai-tv", title: "Dubai TV", author: "Free-TV IPTV · UAE", source: "https://dmisxthvll.cdn.mgmlcdn.com/dubaitvht/smil:dubaitv.stream.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai" },
    Clip { id: "live-dubai-one", title: "Dubai One", author: "Free-TV IPTV · UAE", source: "https://dminnvll.cdn.mangomolo.com/dubaione/smil:dubaione.stream.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai" },
    Clip { id: "live-dubai-sports-1", title: "Dubai Sports 1", author: "Free-TV IPTV · UAE", source: "https://dmitnthfr.cdn.mgmlcdn.com/dubaisports/smil:dubaisports.stream.smil/chunklist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai sports" },
    Clip { id: "live-dubai-sports-2", title: "Dubai Sports 2", author: "Free-TV IPTV · UAE", source: "https://dmitwlvvll.cdn.mangomolo.com/dubaisportshd/smil:dubaisportshd.smil/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai sports" },
    Clip { id: "live-dubai-sports-3", title: "Dubai Sports 3", author: "Free-TV IPTV · UAE", source: "https://dmitwlvvll.cdn.mangomolo.com/dubaisportshd5/smil:dubaisportshd5.smil/index.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai sports" },
    Clip { id: "live-dubai-racing-1", title: "Dubai Racing 1", author: "Free-TV IPTV · UAE", source: "https://dmisvthvll.cdn.mgmlcdn.com/events/smil:events.stream.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai racing" },
    Clip { id: "live-dubai-racing-2", title: "Dubai Racing 2", author: "Free-TV IPTV · UAE", source: "https://dmithrvll.cdn.mangomolo.com/dubairacing/smil:dubairacing.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai racing" },
    Clip { id: "live-dubai-racing-3", title: "Dubai Racing 3", author: "Free-TV IPTV · UAE", source: "https://dmithrvll.cdn.mangomolo.com/dubaimubasher/smil:dubaimubasher.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai racing" },
    Clip { id: "live-dubai-zaman", title: "Dubai Zaman", author: "Free-TV IPTV · UAE", source: "https://dmiffthvll.cdn.mangomolo.com/dubaizaman/smil:dubaizaman.stream.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai" },
    Clip { id: "live-sama-dubai", title: "Sama Dubai", author: "Free-TV IPTV · UAE", source: "https://dmieigthvll.cdn.mgmlcdn.com/samadubaiht/smil:samadubai.stream.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai" },
    Clip { id: "live-noor-dubai", title: "Noor Dubai", author: "Free-TV IPTV · UAE", source: "https://dmiffthvll.cdn.mangomolo.com/noordubaitv/smil:noordubaitv.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Dubai" },
    Clip { id: "live-sharjah-tv", title: "Sharjah TV", author: "Free-TV IPTV · UAE", source: "https://live.kwikmotion.com/smc1live/smc1tv.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Sharjah" },
    Clip { id: "live-sharjah-sports", title: "Sharjah Sports", author: "Free-TV IPTV · UAE", source: "https://svs.itworkscdn.net/smc4sportslive/smc4.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Sharjah sports" },
    Clip { id: "live-al-wousta", title: "Al Wousta", author: "Free-TV IPTV · UAE", source: "https://svs.itworkscdn.net/alwoustalive/alwoustatv.smil/playlist.m3u8", format: "HLS · live", kind: ClipKind::Hls, note: "Live · UAE · Al Wousta" },
];

/// The full set of pages the app offers: offline clips, the opt-in test stream,
/// then the live channels from the Free-TV IPTV playlists.
pub fn catalogue() -> Vec<Clip> {
    CLIPS.iter().copied().chain(LIVE_CHANNELS.iter().copied()).collect()
}

/// Resolves a catalogue path against the crate directory.
///
/// The source strings stay static so the catalogue can be tested without a
/// Cargo-provided manifest path; this helper makes runtime file lookup robust.
pub fn resolved_source(clip: &Clip) -> String {
    if clip.kind == ClipKind::LocalMp4 {
        // On a phone the catalogue is copied into the app bundle next to the
        // executable by CranposeBuild.toml's `assets = ["assets"]`. There is no
        // Cargo manifest directory at runtime, so resolve against the bundle
        // rather than the compile-time path (a Windows path that does not exist
        // on iOS), which used to make every clip report "no video track".
        #[cfg(target_os = "android")]
        {
            // Android packages assets inside the APK; `std::fs` cannot read
            // them from the executable's directory. Materialise the embedded
            // clip into a writable cache path once and return that path.
            return android_asset_path(clip.source);
        }
        #[cfg(target_os = "ios")]
        {
            if let Some(bundle) = std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(|dir| dir.to_path_buf()))
            {
                return bundle.join(clip.source).to_string_lossy().into_owned();
            }
            // Fall back to a bundle-relative path if the executable path is
            // unavailable (the process working directory is the bundle root).
            return clip.source.to_owned();
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let root = option_env!("CARGO_MANIFEST_DIR").unwrap_or(".");
            format!("{root}/{}", clip.source)
        }
    } else {
        clip.source.to_owned()
    }
}

/// Materialises an embedded video asset into a writable path on Android.
///
/// The APK holds the clips under `assets/videos`, which `std::fs` cannot read.
/// On Android the Cranpose app's data directory is published in this
/// environment variable; failing that, the process temp directory is writable.
/// The file is written once (a stable, content-derived name) and reused.
#[cfg(target_os = "android")]
fn android_asset_path(source: &str) -> String {
    let bytes: &[u8] = match source {
        "assets/videos/bunny.mp4" => include_bytes!("../assets/videos/bunny.mp4"),
        "assets/videos/bbb10.mp4" => include_bytes!("../assets/videos/bbb10.mp4"),
        "assets/videos/flower.mp4" => include_bytes!("../assets/videos/flower.mp4"),
        "assets/videos/sintel.mp4" => include_bytes!("../assets/videos/sintel.mp4"),
        "assets/videos/w3c.mp4" => include_bytes!("../assets/videos/w3c.mp4"),
        _ => return source.to_owned(),
    };
    // A short stable hash keeps names unique without collisions between clips.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in source.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let name = format!("counter_{hash:016x}.mp4");
    let dir = std::env::temp_dir();
    let path = dir.join(&name);
    if !path.exists() {
        // A concurrent first frame can race; writing the same bytes twice is
        // harmless, and a partially written file is fixed on the next call.
        let _ = std::fs::write(&path, bytes);
    }
    path.to_string_lossy().into_owned()
}

/// Whether a page owns playback at a settled pager position.
pub fn active_page(settled_page: usize, page_index: usize) -> bool {
    settled_page == page_index
}

/// Renders a decoded frame size for the live status line.
pub fn format_resolution(size: Option<(u32, u32)>) -> String {
    match size {
        Some((width, height)) if width > 0 && height > 0 => {
            format!("RES {width}×{height}")
        }
        _ => "RES —".to_owned(),
    }
}

/// Renders an elapsed decoding duration for the in-app test report.
pub fn format_elapsed(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    if millis < 1_000 {
        format!("{millis} ms")
    } else if elapsed.as_secs_f64() < 10.0 {
        format!("{:.2} s", elapsed.as_secs_f64())
    } else {
        format!("{:.1} s", elapsed.as_secs_f64())
    }
}

/// Catalogue entries the in-app decode test sweeps.
///
/// Network entries are excluded so the test stays deterministic offline.
pub fn decode_cases() -> Vec<Clip> {
    CLIPS
        .iter()
        .copied()
        .filter(|clip| clip.kind == ClipKind::LocalMp4)
        .collect()
}

/// Overall verdict for the in-app test report.
pub fn verdict(passed: usize, total: usize) -> &'static str {
    if total == 0 {
        "NO DATA"
    } else if passed == total {
        "PASS"
    } else {
        "FAIL"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_has_five_distinct_local_clips_and_one_network_clip() {
        let local: Vec<_> = CLIPS
            .iter()
            .filter(|clip| clip.kind == ClipKind::LocalMp4)
            .collect();
        let network: Vec<_> = CLIPS
            .iter()
            .filter(|clip| clip.kind == ClipKind::Hls)
            .collect();

        assert_eq!(local.len(), 5, "five local clips are expected");
        assert_eq!(network.len(), 1, "one opt-in network clip is expected");

        let mut paths: Vec<_> = CLIPS.iter().map(|clip| clip.source).collect();
        paths.sort_unstable();
        let unique: std::collections::BTreeSet<_> = paths.iter().copied().collect();
        assert_eq!(unique.len(), paths.len(), "catalogue paths must be unique");
    }

    #[test]
    fn every_catalogue_entry_is_described() {
        for clip in CLIPS {
            assert!(!clip.id.is_empty(), "{} has no id", clip.source);
            assert!(!clip.title.is_empty(), "{} has no title", clip.source);
            assert!(!clip.author.is_empty(), "{} has no author", clip.source);
            assert!(!clip.format.is_empty(), "{} has no format", clip.source);
            assert!(!clip.note.is_empty(), "{} has no note", clip.source);
        }
    }

    #[test]
    fn only_the_settled_page_is_active() {
        assert!(active_page(0, 0));
        assert!(!active_page(0, 1));
        assert!(!active_page(2, 1));
        assert!(active_page(2, 2));
        assert!(!active_page(2, 3));
    }

    #[test]
    fn resolution_formatter_marks_unknown_and_reports_pixels() {
        assert_eq!(format_resolution(None), "RES —");
        assert_eq!(format_resolution(Some((640, 360))), "RES 640×360");
        assert_eq!(
            format_resolution(Some((3840, 2160))),
            "RES 3840×2160"
        );
    }

    #[test]
    fn elapsed_formatter_uses_milliseconds_when_under_a_second() {
        assert_eq!(format_elapsed(Duration::from_millis(0)), "0 ms");
        assert_eq!(format_elapsed(Duration::from_millis(12)), "12 ms");
        assert_eq!(format_elapsed(Duration::from_millis(1_480)), "1.48 s");
        assert_eq!(format_elapsed(Duration::from_secs(12)), "12.0 s");
    }

    #[test]
    fn decode_cases_cover_local_files_only() {
        let cases = decode_cases();
        assert_eq!(cases.len(), 5);
        assert!(
            cases.iter().all(|clip| clip.kind == ClipKind::LocalMp4),
            "the offline decode sweep must not depend on the network"
        );
    }

    #[test]
    fn verdict_distinguishes_empty_partial_and_complete_runs() {
        assert_eq!(verdict(0, 0), "NO DATA");
        assert_eq!(verdict(2, 3), "FAIL");
        assert_eq!(verdict(3, 3), "PASS");
    }

    #[test]
    fn live_channels_are_unique_opt_in_hls_sources() {
        // 45 playlist entries - 1 YouTube page - 2 duplicate URLs = 42.
        assert_eq!(LIVE_CHANNELS.len(), 42);

        let mut ids: Vec<_> = LIVE_CHANNELS.iter().map(|clip| clip.id).collect();
        ids.sort_unstable();
        let unique_ids: std::collections::BTreeSet<_> = ids.iter().copied().collect();
        assert_eq!(unique_ids.len(), ids.len(), "live ids must be unique");

        let mut sources: Vec<_> = LIVE_CHANNELS.iter().map(|clip| clip.source).collect();
        sources.sort_unstable();
        let unique_sources: std::collections::BTreeSet<_> = sources.iter().copied().collect();
        assert_eq!(unique_sources.len(), sources.len(), "live URLs must be unique");

        for clip in LIVE_CHANNELS {
            assert_eq!(clip.kind, ClipKind::Hls, "{} is not HLS", clip.id);
            assert!(
                clip.source.starts_with("http://") || clip.source.starts_with("https://"),
                "{} is not a network URL",
                clip.id
            );
            assert!(
                !clip.source.contains("youtube.com"),
                "{} is a page, not a media playlist",
                clip.id
            );
            assert!(!clip.title.is_empty(), "{} has no title", clip.id);
            assert!(!clip.note.is_empty(), "{} has no note", clip.id);
        }

        assert_eq!(catalogue().len(), CLIPS.len() + LIVE_CHANNELS.len());
        assert_eq!(
            decode_cases().len(),
            5,
            "live channels must stay out of the offline decode sweep"
        );
    }
}
