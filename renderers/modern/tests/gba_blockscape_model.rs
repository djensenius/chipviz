//! Host-only comparisons against the native source; no emulator or GPU.

#[path = "gba_blockscape_expected/mod.rs"]
mod expected;
#[path = "../src/scenes/gba_blockscape.rs"]
mod gba_blockscape;

use expected::NativeOracle;
use gba_blockscape::{
    ComposeError, DemoState, HEIGHT, KEY_A, KEY_B, KEY_SELECT, KEY_START, WIDTH, compose,
};

fn state(scene: u8, palette: u8, energy: u8, beat: bool, note: u8, spectrum: [u8; 8]) -> DemoState {
    DemoState {
        spectrum,
        scene,
        palette,
        energy,
        beat,
        note,
    }
}

#[test]
fn composition_matches_the_native_source_for_variants_overlays_and_counter_edges() {
    let oracle = NativeOracle::compile();
    let inputs = [
        (0, 0, 0, false, 0, [0; 8], 0),
        (1, 0, 0, false, 0, [0; 8], 0),
        (16, 0, 0, false, 48, [0; 8], 0),
        (65535, 0, 0, false, 0, [0; 8], 0),
        (1, 7, 255, true, 255, [255; 8], 0),
        (
            97,
            3,
            137,
            false,
            64,
            [0, 31, 32, 63, 64, 127, 128, 255],
            KEY_START,
        ),
        (
            65535,
            6,
            200,
            true,
            83,
            [200, 1, 80, 255, 43, 111, 32, 199],
            KEY_A | KEY_B | KEY_SELECT,
        ),
        (
            8193,
            5,
            48,
            false,
            91,
            [9, 18, 27, 36, 45, 54, 63, 72],
            KEY_A | KEY_B | KEY_SELECT | KEY_START | 0x3f0,
        ),
    ];
    for scene in 0..4 {
        for &(frame, palette, energy, beat, note, spectrum, keys) in &inputs {
            let state = state(scene, palette, energy, beat, note, spectrum);
            let actual = compose(frame, state, keys).unwrap();
            let expected = oracle.compose(frame, state, keys);
            assert_eq!(actual.indices.len(), expected.indices.len());
            let first_difference = actual
                .indices
                .iter()
                .zip(&expected.indices)
                .position(|(a, b)| a != b);
            assert_eq!(
                first_difference, None,
                "pixel mismatch: scene={scene} frame={frame} keys={keys:#x}"
            );
            assert_eq!(
                actual.palette, expected.palette,
                "scene={scene} frame={frame}"
            );
        }
    }
}

#[test]
fn invalid_state_is_rejected_and_unrelated_key_bits_are_ignored() {
    let mut state = state(4, 0, 0, false, 0, [0; 8]);
    assert_eq!(compose(0, state, 0), Err(ComposeError::InvalidScene(4)));
    state.scene = 0;
    state.palette = 8;
    assert_eq!(compose(0, state, 0), Err(ComposeError::InvalidPalette(8)));
    state.palette = 0;
    let frame = compose(0, state, 0).unwrap();
    assert_eq!(frame.indices.len(), WIDTH * HEIGHT);
    assert_eq!(frame, compose(0, state, 0xfff0).unwrap());
}

#[test]
fn counter_and_spectrum_change_the_offscreen_composition() {
    let quiet = state(0, 0, 0, false, 48, [0; 8]);
    let mut energetic = quiet;
    energetic.spectrum = [255; 8];
    let first = compose(0, quiet, 0).unwrap();
    let later = compose(16, quiet, 0).unwrap();
    let before_wrap = compose(u16::MAX, quiet, 0).unwrap();
    let driven = compose(0, energetic, 0).unwrap();
    assert_ne!(first.indices, later.indices);
    assert_ne!(before_wrap.indices, first.indices);
    assert_ne!(first.indices, driven.indices);
}
