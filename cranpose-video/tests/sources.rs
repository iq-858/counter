//! The values every build shares: how a source is read, and what a frame and a
//! frame rate state.

use std::time::Duration;

use cranpose_video::{FrameRate, Rotation, VideoFrame, VideoSource};

#[test]
fn a_url_carries_a_scheme_and_a_path_does_not() {
    assert_eq!(
        VideoSource::from("https://host/a.mp4"),
        VideoSource::url("https://host/a.mp4")
    );
    assert_eq!(
        VideoSource::from("file:///home/me/a.mp4"),
        VideoSource::url("file:///home/me/a.mp4")
    );
    assert_eq!(
        VideoSource::from("content://media/external/1"),
        VideoSource::url("content://media/external/1")
    );
    assert_eq!(
        VideoSource::from("/home/me/a.mp4"),
        VideoSource::path("/home/me/a.mp4")
    );
    // A Windows drive letter is one character before the colon, so it reads as
    // a path rather than as a scheme.
    assert_eq!(
        VideoSource::from(r"C:\clips\a.mp4"),
        VideoSource::path(r"C:\clips\a.mp4")
    );
}

#[test]
fn hls_classification_uses_the_url_path_not_the_query_or_fragment() {
    assert!(VideoSource::url("https://host/live/index.M3U8?token=abc#frag").is_hls());
    assert!(VideoSource::url("https://host/live/index%2Em3u8?token=abc").is_hls());
    assert!(!VideoSource::url("https://host/live/clip.mp4?playlist=m3u8").is_hls());
    assert!(!VideoSource::url("https://host/live/clip.mp4#index.m3u8").is_hls());
    assert!(!VideoSource::url("https://host/live/playlist").is_hls());
    assert!(!VideoSource::path("index.m3u8").is_hls());
    assert!(!VideoSource::bytes(vec![0; 4]).is_hls());
}

#[test]
fn a_frame_holds_exactly_the_pixels_its_size_states() {
    let size = 3 * 2 * 4;
    let frame = VideoFrame::from_rgba8(3, 2, Duration::from_millis(40), vec![7; size])
        .expect("a buffer of exactly the size is a frame");
    assert_eq!(frame.byte_len(), size);
    assert_eq!(frame.pts(), Duration::from_millis(40));
    assert_eq!(frame.width(), 3);
    assert_eq!(frame.height(), 2);
    assert!(frame.rgba().iter().all(|byte| *byte == 7));
    assert!(
        VideoFrame::from_rgba8(3, 2, Duration::ZERO, vec![7; size - 1]).is_none(),
        "a short buffer is not a frame"
    );
    assert!(
        VideoFrame::from_rgba8(3, 2, Duration::ZERO, vec![7; size + 1]).is_none(),
        "a long buffer is not a frame"
    );
}

#[test]
fn a_frame_rate_keeps_the_ratio_the_container_states() {
    let rate = FrameRate {
        numerator: 30_000,
        denominator: 1001,
    };
    assert!((rate.frames_per_second() - 29.97).abs() < 0.01);
    let frame = rate.frame_duration().expect("a stated rate lasts a frame");
    assert!((frame.as_secs_f64() - 1.0 / 29.97).abs() < 0.0001);
    assert_eq!(
        FrameRate {
            numerator: 0,
            denominator: 0
        }
        .frame_duration(),
        None,
        "a rate the container does not state has no frame duration"
    );
}

#[test]
fn a_quarter_turn_swaps_the_axes() {
    assert_eq!(Rotation::from_degrees(90.0), Rotation::Clockwise90);
    assert_eq!(Rotation::from_degrees(180.0), Rotation::Clockwise180);
    assert_eq!(Rotation::from_degrees(270.0), Rotation::Clockwise270);
    assert_eq!(Rotation::from_degrees(-90.0), Rotation::Clockwise270);
    assert_eq!(Rotation::from_degrees(45.0), Rotation::None);
    assert_eq!(Rotation::Clockwise90.display_size(1920, 1080), (1080, 1920));
    assert_eq!(
        Rotation::Clockwise180.display_size(1920, 1080),
        (1920, 1080)
    );
}

#[test]
fn the_build_names_its_decoder() {
    assert!(!cranpose_video::decoder_name().is_empty());
    assert!(!cranpose_video::platform().name().is_empty());
    assert_eq!(
        cranpose_video::is_supported(),
        cranpose_video::decoder_name() != "none",
        "only a build without a decoder reports that it decodes nothing"
    );
}
