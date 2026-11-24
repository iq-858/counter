//! Composition smoke tests for the picture pump and the ready-made controls.
//!
//! The frame pump and the status tick are driven by `LaunchedEffect`s that only
//! run inside a composition. These tests compose the widgets and drive a layout
//! pass so those effects are created and advanced.

#![cfg(feature = "widget")]

use cranpose_ui::{Modifier, Size, run_test_composition};
use cranpose_video::{Video, VideoWithControls};

const CLIP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/clip.mp4");

fn compose_and_measure(build: impl FnMut() + 'static) {
    let mut composition = run_test_composition(build);
    if let Some(root) = composition.root() {
        composition
            .measure_layout(root, Size::new(640.0, 480.0))
            .expect("layout measurement succeeds");
    }
}

#[test]
fn the_picture_pump_composes_and_advances() {
    compose_and_measure(move || {
        Video(CLIP, Modifier::empty());
    });
}

#[test]
fn the_controls_compose_with_a_status_tick() {
    compose_and_measure(move || {
        VideoWithControls(CLIP, Modifier::empty());
    });
}
