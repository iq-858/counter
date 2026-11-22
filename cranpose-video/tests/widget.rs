//! Composition of the ready-made controls and the flexible progress slider.
#![cfg(feature = "widget")]

use std::{cell::Cell, rc::Rc, time::Duration};

use cranpose_ui::{Modifier, Size, run_test_composition, widgets::slider::SliderSpec};
use cranpose_video::{
    GlassStyle, ProControl, VideoControls, VideoControlsOptions, VideoControlsPro,
    VideoLoopButton, VideoMuteButton, VideoProgressSliderWith, VideoSpeedButton, VideoStopButton,
    VideoVolumeSlider, rememberVideoPlayer,
};

const CLIP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/clip.mp4");

/// `Slider` composes its caller-supplied content inside a `BoxWithConstraints`,
/// which only sub-composes during a layout pass. A bare `run_test_composition`
/// therefore never invokes the slider content; drive one measurement so the
/// content closures actually render.
fn compose_and_measure(build: impl FnMut() + 'static) {
    let mut composition = run_test_composition(build);
    if let Some(root) = composition.root() {
        composition
            .measure_layout(root, Size::new(1000.0, 60.0))
            .expect("layout measurement succeeds");
    }
}

#[test]
fn ready_made_controls_compose_from_an_existing_handle() {
    run_test_composition(move || {
        let handle = rememberVideoPlayer(CLIP);
        VideoControls(handle, Modifier::empty());
    });
}

#[test]
fn pro_controls_compose_from_a_caller_owned_handle() {
    // The extended bar consumes a handle and opens no extra decoder. Composition
    // plus a layout pass exercises every button/slider content closure.
    compose_and_measure(move || {
        let handle = rememberVideoPlayer(CLIP);
        VideoControlsPro(
            handle,
            Modifier::empty(),
            VideoControlsOptions::new()
                .style(GlassStyle::Floating)
                .elements(&[ProControl::PlayPause, ProControl::Mute, ProControl::Stop]),
        );
    });
}

#[test]
fn individual_pro_controls_compose() {
    compose_and_measure(move || {
        let handle = rememberVideoPlayer(CLIP);
        VideoMuteButton(handle.clone(), Modifier::empty());
        VideoLoopButton(handle.clone(), Modifier::empty());
        VideoStopButton(handle.clone(), Modifier::empty());
        VideoSpeedButton(handle.clone(), Modifier::empty(), &[0.5, 1.0, 2.0]);
        VideoVolumeSlider(handle, Modifier::empty(), SliderSpec::default());
    });
}

#[test]
fn a_custom_progress_slider_receives_a_normalised_scope() {
    let calls = Rc::new(Cell::new(0usize));
    let sink = Rc::clone(&calls);
    compose_and_measure(move || {
        let sink = Rc::clone(&sink);
        let handle = rememberVideoPlayer(CLIP);
        VideoProgressSliderWith(
            handle,
            Modifier::empty(),
            SliderSpec::default(),
            move |scope| {
                assert!((0.0..=1.0).contains(&scope.value()));
                assert!(scope.track_extent().is_finite());
                sink.set(sink.get() + 1);
            },
        );
    });
    assert!(calls.get() > 0, "the custom slider content never composed");
}

#[test]
fn the_progress_helper_clamps_a_missing_or_empty_duration() {
    assert_eq!(
        cranpose_video::progress_fraction_for(Duration::from_secs(3), None),
        0.0
    );
    assert_eq!(
        cranpose_video::progress_fraction_for(Duration::from_secs(3), Some(Duration::ZERO)),
        0.0
    );
}

#[test]
fn the_progress_helper_maps_position_to_the_whole_duration() {
    let duration = Duration::from_secs(10);
    let cases = [
        (Duration::ZERO, 0.0),
        (Duration::from_secs(5), 0.5),
        (Duration::from_secs(10), 1.0),
        (Duration::from_secs(12), 1.0),
    ];
    for (position, expected) in cases {
        assert_eq!(
            cranpose_video::progress_fraction_for(position, Some(duration)),
            expected,
            "position {position:?} of {duration:?}"
        );
    }
}
