pub const VIDEO_FILES: [&'static str; 5] = ["bbb10", "flower", "sintel", "bunny", "w3c"];

pub fn video_for(post: &Post) -> &'static str {
    VIDEO_FILES[post.id % VIDEO_FILES.len()]
}

#[derive(Clone, PartialEq)]
pub struct Post {
    pub id: usize,
    pub user: &'static str,
    pub handle: &'static str,
    pub caption: &'static str,
    pub tags: &'static str,
    pub music: &'static str,
    pub likes: u32,
    pub comments: u32,
    pub shares: u32,
    pub variant: u8,
    pub verified: bool,
    pub liked: bool,
    pub following: bool,
}

#[derive(Clone, PartialEq)]
pub struct Comment {
    pub user: &'static str,
    pub text: &'static str,
    pub likes: u32,
    pub ago: &'static str,
}

#[derive(Clone, PartialEq)]
pub struct Notification {
    pub kind: u8,
    pub user: &'static str,
    pub text: &'static str,
    pub ago: &'static str,
}

#[derive(Clone, PartialEq)]
pub struct Creator {
    pub user: &'static str,
    pub handle: &'static str,
    pub tagline: &'static str,
    pub followers: u32,
    pub variant: u8,
    pub verified: bool,
    pub following: bool,
}

#[derive(Clone, PartialEq)]
pub struct Sound {
    pub title: &'static str,
    pub author: &'static str,
    pub uses: u32,
    pub variant: u8,
}

pub fn posts() -> Vec<Post> {
    vec![
        Post {
            id: 0,
            user: "Nova Rivers",
            handle: "@novarivers",
            caption: "late night city lights hit different",
            tags: "#citylights #nightvibes #cinematic",
            music: "original sound - Nova Rivers",
            likes: 128_400,
            comments: 2_341,
            shares: 8_902,
            variant: 0,
            verified: true,
            liked: false,
            following: false,
        },
        Post {
            id: 1,
            user: "Kai Motion",
            handle: "@kaimotion",
            caption: "sound on for this one",
            tags: "#bassdrop #edm #visuals",
            music: "Midnight Pulse - DJ Kalo",
            likes: 892_100,
            comments: 14_502,
            shares: 41_338,
            variant: 1,
            verified: false,
            liked: true,
            following: true,
        },
        Post {
            id: 2,
            user: "Sage Studio",
            handle: "@sagestudio",
            caption: "3 color theory tricks nobody taught you",
            tags: "#design #arttips #creative",
            music: "lofi beats - Sage Studio",
            likes: 45_670,
            comments: 1_209,
            shares: 3_441,
            variant: 2,
            verified: true,
            liked: false,
            following: true,
        },
        Post {
            id: 3,
            user: "Rue Chen",
            handle: "@ruechen",
            caption: "pov: you finally beat the level",
            tags: "#gaming #speedrun #clutch",
            music: "Victory Fanfare - RetroWave",
            likes: 312_880,
            comments: 6_773,
            shares: 19_204,
            variant: 3,
            verified: false,
            liked: false,
            following: false,
        },
        Post {
            id: 4,
            user: "Mira Sol",
            handle: "@mirasol",
            caption: "golden hour in the hills",
            tags: "#travel #goldenhour #nature",
            music: "Sunset Drive - Ambients",
            likes: 71_530,
            comments: 908,
            shares: 5_120,
            variant: 4,
            verified: true,
            liked: true,
            following: true,
        },
        Post {
            id: 5,
            user: "Echo Lab",
            handle: "@echolab",
            caption: "this algorithm is oddly satisfying",
            tags: "#coding #satisfying #devtok",
            music: "Lo-Fi Loop - Echo Lab",
            likes: 203_910,
            comments: 3_388,
            shares: 12_760,
            variant: 0,
            verified: false,
            liked: false,
            following: false,
        },
        Post {
            id: 6,
            user: "Iris Vale",
            handle: "@irisvale",
            caption: "neon reflections on wet asphalt",
            tags: "#rainy #neon #aesthetic",
            music: "Tokyo Rain - Iris Vale",
            likes: 154_220,
            comments: 2_870,
            shares: 9_640,
            variant: 1,
            verified: true,
            liked: false,
            following: true,
        },
        Post {
            id: 7,
            user: "Dune Atlas",
            handle: "@duneatlas",
            caption: "drone shot took 4 hours to line up",
            tags: "#drone #cinematic #landscape",
            music: "Wide Open - Atlas Sound",
            likes: 388_760,
            comments: 5_112,
            shares: 22_845,
            variant: 2,
            verified: false,
            liked: false,
            following: false,
        },
        Post {
            id: 8,
            user: "Pixel Forge",
            handle: "@pixelforge",
            caption: "pixel art timelapse, 1 frame at a time",
            tags: "#pixelart #indiedev #retro",
            music: "8-bit Dreams - Forge",
            likes: 97_340,
            comments: 1_744,
            shares: 4_902,
            variant: 3,
            verified: true,
            liked: true,
            following: true,
        },
        Post {
            id: 9,
            user: "Lena Frost",
            handle: "@lenafrost",
            caption: "iced coffee and code, my two loves",
            tags: "#devlife #productivity #cozy",
            music: "Morning Focus - Lena",
            likes: 61_890,
            comments: 1_033,
            shares: 2_988,
            variant: 4,
            verified: false,
            liked: false,
            following: false,
        },
        Post {
            id: 10,
            user: "Orbit Nine",
            handle: "@orbitnine",
            caption: "synthwave visualizer for the new track",
            tags: "#synthwave #music #retrowave",
            music: "Neon Drive - Orbit Nine",
            likes: 266_410,
            comments: 4_226,
            shares: 15_330,
            variant: 0,
            verified: true,
            liked: false,
            following: true,
        },
        Post {
            id: 11,
            user: "Taro Sun",
            handle: "@tarosun",
            caption: "cooking is just chemistry you can eat",
            tags: "#foodtok #cooking #asmr",
            music: "Kitchen Loops - Taro",
            likes: 118_660,
            comments: 2_105,
            shares: 7_421,
            variant: 1,
            verified: false,
            liked: false,
            following: false,
        },
        Post {
            id: 12,
            user: "Wren Lyric",
            handle: "@wrenlyric",
            caption: "wrote this hook at 3am, no regrets",
            tags: "#singing #original #songwriter",
            music: "original sound - Wren Lyric",
            likes: 442_980,
            comments: 8_914,
            shares: 31_207,
            variant: 2,
            verified: true,
            liked: true,
            following: true,
        },
        Post {
            id: 13,
            user: "Hex Vector",
            handle: "@hexvector",
            caption: "shader breakdown in the comments",
            tags: "#gamedev #shaders #graphics",
            music: "Deep Space - Hex",
            likes: 85_120,
            comments: 1_596,
            shares: 6_044,
            variant: 3,
            verified: false,
            liked: false,
            following: false,
        },
    ]
}

const COMMENT_POOL: &[(&str, &str, u32, &str)] = &[
    ("pixel_pusher", "the transitions though 🔥", 1_204, "2h"),
    ("moonlit.ash", "watched this like 20 times ngl", 842, "3h"),
    ("devbeats", "what did you shoot this on?", 611, "4h"),
    ("aria.codes", "saving this for inspo", 407, "5h"),
    ("thevoid_walker", "this is a masterpiece", 388, "6h"),
    ("lofi.hana", "the audio choice is perfect", 255, "7h"),
    ("quinn.builds", "tutorial when??", 199, "9h"),
    ("static_noise", "ok this is actually insane", 142, "11h"),
    ("velvet_kite", "algorithm blessed me today", 88, "13h"),
    ("byte_jam", "no way this is real", 61, "15h"),
    ("echo_river", "the color grading is unreal", 530, "1h"),
    ("nova.fan", "putting this on my moodboard", 476, "2h"),
    ("mint_graph", "how is this even edited", 321, "3h"),
    (
        "sable.dawn",
        "came for the audio, stayed for the visuals",
        298,
        "4h",
    ),
    ("coffee_then_code", "the pacing is *chef's kiss*", 244, "6h"),
    ("glass_pine", "sending this to my group chat rn", 187, "8h"),
    ("arcade_kid", "this beat goes so hard", 165, "10h"),
    ("vapor_sky", "the loop is seamless wow", 133, "12h"),
    ("quiet_storm", "genuinely soothing fr", 97, "14h"),
    ("radio_silent", "10/10 would scroll past again", 54, "16h"),
];

pub fn comments(seed: usize) -> Vec<Comment> {
    let n = COMMENT_POOL.len();
    let start = seed % n;
    (0..n)
        .map(|i| {
            let (user, text, likes, ago) = COMMENT_POOL[(start + i) % n];
            Comment {
                user,
                text,
                likes,
                ago,
            }
        })
        .collect()
}

pub fn notification_pool() -> Vec<Notification> {
    vec![
        Notification {
            kind: 0,
            user: "Kai Motion",
            text: "started following you",
            ago: "2m",
        },
        Notification {
            kind: 1,
            user: "pixel_pusher",
            text: "liked your video",
            ago: "18m",
        },
        Notification {
            kind: 2,
            user: "aria.codes",
            text: "commented: \"absolutely fire 🔥\"",
            ago: "1h",
        },
        Notification {
            kind: 1,
            user: "moonlit.ash",
            text: "and 43 others liked your video",
            ago: "2h",
        },
        Notification {
            kind: 0,
            user: "Sage Studio",
            text: "started following you",
            ago: "5h",
        },
        Notification {
            kind: 3,
            user: "ClipTok",
            text: "your video hit 100K views!",
            ago: "8h",
        },
        Notification {
            kind: 2,
            user: "thevoid_walker",
            text: "commented: \"the vibes are immaculate\"",
            ago: "12h",
        },
        Notification {
            kind: 1,
            user: "lofi.hana",
            text: "liked your comment",
            ago: "1d",
        },
        Notification {
            kind: 0,
            user: "velvet_kite",
            text: "started following you",
            ago: "1d",
        },
        Notification {
            kind: 3,
            user: "ClipTok",
            text: "you're on a 7-day streak!",
            ago: "2d",
        },
        Notification {
            kind: 0,
            user: "Dune Atlas",
            text: "started following you",
            ago: "2d",
        },
        Notification {
            kind: 1,
            user: "echo_river",
            text: "and 128 others liked your video",
            ago: "3d",
        },
        Notification {
            kind: 2,
            user: "mint_graph",
            text: "commented: \"how is this edited\"",
            ago: "3d",
        },
        Notification {
            kind: 3,
            user: "ClipTok",
            text: "your sound was used 2.4K times",
            ago: "4d",
        },
        Notification {
            kind: 0,
            user: "Wren Lyric",
            text: "started following you",
            ago: "5d",
        },
    ]
}

pub fn notifications() -> Vec<Notification> {
    notification_pool()
}

pub fn creators() -> Vec<Creator> {
    vec![
        Creator {
            user: "Nova Rivers",
            handle: "@novarivers",
            tagline: "Cinematic city loops",
            followers: 1_200_000,
            variant: 0,
            verified: true,
            following: false,
        },
        Creator {
            user: "Kai Motion",
            handle: "@kaimotion",
            tagline: "Bass & visuals",
            followers: 2_450_000,
            variant: 1,
            verified: false,
            following: true,
        },
        Creator {
            user: "Sage Studio",
            handle: "@sagestudio",
            tagline: "Design tips daily",
            followers: 884_000,
            variant: 2,
            verified: true,
            following: true,
        },
        Creator {
            user: "Rue Chen",
            handle: "@ruechen",
            tagline: "Speedruns & clutches",
            followers: 612_000,
            variant: 3,
            verified: false,
            following: false,
        },
        Creator {
            user: "Mira Sol",
            handle: "@mirasol",
            tagline: "Golden hour travel",
            followers: 1_760_000,
            variant: 4,
            verified: true,
            following: true,
        },
        Creator {
            user: "Echo Lab",
            handle: "@echolab",
            tagline: "Satisfying code",
            followers: 398_000,
            variant: 0,
            verified: false,
            following: false,
        },
        Creator {
            user: "Iris Vale",
            handle: "@irisvale",
            tagline: "Rainy neon aesthetics",
            followers: 745_000,
            variant: 1,
            verified: true,
            following: true,
        },
        Creator {
            user: "Orbit Nine",
            handle: "@orbitnine",
            tagline: "Synthwave producer",
            followers: 1_090_000,
            variant: 0,
            verified: true,
            following: true,
        },
        Creator {
            user: "Wren Lyric",
            handle: "@wrenlyric",
            tagline: "Songs written at 3am",
            followers: 3_120_000,
            variant: 2,
            verified: true,
            following: true,
        },
        Creator {
            user: "Hex Vector",
            handle: "@hexvector",
            tagline: "Shaders & gamedev",
            followers: 276_000,
            variant: 3,
            verified: false,
            following: false,
        },
    ]
}

pub fn sounds() -> Vec<Sound> {
    vec![
        Sound {
            title: "Midnight Pulse",
            author: "DJ Kalo",
            uses: 412_000,
            variant: 1,
        },
        Sound {
            title: "Sunset Drive",
            author: "Ambients",
            uses: 288_000,
            variant: 4,
        },
        Sound {
            title: "Neon Drive",
            author: "Orbit Nine",
            uses: 654_000,
            variant: 0,
        },
        Sound {
            title: "Lo-Fi Loop",
            author: "Echo Lab",
            uses: 190_000,
            variant: 2,
        },
        Sound {
            title: "Tokyo Rain",
            author: "Iris Vale",
            uses: 233_000,
            variant: 1,
        },
        Sound {
            title: "8-bit Dreams",
            author: "Pixel Forge",
            uses: 121_000,
            variant: 3,
        },
    ]
}

pub const DISCOVER_TAGS: &[(&str, u32)] = &[
    ("#fyp", 1_412),
    ("#viral", 988),
    ("#trending", 755),
    ("#cinematic", 674),
    ("#lofi", 541),
    ("#design", 498),
    ("#gaming", 476),
    ("#satisfying", 454),
    ("#coding", 322),
    ("#travel", 310),
    ("#music", 298),
    ("#art", 287),
    ("#devtok", 241),
    ("#retrowave", 208),
];

pub const TRENDING: &[(&str, &str, u32)] = &[
    ("Golden hour edits", "mirasol", 1_240),
    ("Neon city loops", "novarivers", 988),
    ("3D type tricks", "sagestudio", 754),
    ("Speedrun clutch", "ruechen", 612),
    ("Lo-fi loops", "echolab", 540),
    ("Color theory 101", "sagestudio", 433),
    ("Synthwave visuals", "orbitnine", 398),
    ("Shader breakdown", "hexvector", 322),
    ("Pixel timelapse", "pixelforge", 287),
    ("3am songwriting", "wrenlyric", 265),
];

pub fn fmt_count(n: u32) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
