// Exercise the prototype without registering it in the application.
#[path = "../src/scenes/petscii_meter_field.rs"]
mod petscii_meter_field;

use petscii_meter_field::{Cell, Inputs, PetsciiMeterField};

fn input(spectrum: [u8; 8]) -> Inputs {
    Inputs {
        spectrum,
        scene: 0,
        palette: 0,
        beat: false,
        note: 6,
    }
}

fn cell(code: u8, colour: u8) -> Option<Cell> {
    Some(Cell { code, colour })
}

#[test]
fn seed_covers_only_the_grid_and_masks_colour_ram() {
    let field = PetsciiMeterField::default();
    assert_eq!(field.frame(), 0);
    assert_eq!(field.cell(0, 4), cell(96, 1));
    assert_eq!(field.cell(39, 21), cell(104, 9));
    // The byte written is 16; the effective colour RAM value is zero.
    assert_eq!(field.cell(35, 12), cell(107, 0));
    for (x, y) in [
        (0, 0),
        (1, 1),
        (39, 3),
        (0, 22),
        (0, 23),
        (0, 24),
        (40, 4),
        (0, 25),
    ] {
        assert_eq!(field.cell(x, y), None, "({x},{y})");
    }
}

#[test]
fn each_phase_updates_only_its_rows_before_the_meter_overlay() {
    let mut data = input([64; 8]);
    data.palette = 2;
    for (phase, expected) in [
        (0, cell(87, 7)),
        (1, cell(97, 8)),
        (2, cell(98, 9)),
        (3, cell(99, 10)),
    ] {
        let mut field = PetsciiMeterField::default();
        for _ in 0..phase {
            field.skip_frame();
        }
        let before: Vec<_> = (0..18).map(|y| field.cell(0, y + 4)).collect();
        field.step(&data);
        assert_eq!(field.frame(), phase + 1);
        assert_eq!(field.cell(0, usize::from(phase) + 4), expected);
        for y in 0..18 {
            if y % 4 != usize::from(phase) {
                assert_eq!(field.cell(0, y + 4), before[y], "phase={phase} y={y}");
            } else {
                assert_ne!(field.cell(0, y + 4), before[y], "phase={phase} y={y}");
            }
        }
        if phase == 0 {
            // Logical row 16 is also in phase zero.
            assert_eq!(field.cell(0, 20), cell(96, 7));
        }
        if phase == 1 {
            assert_eq!(field.cell(0, 21), cell(97, 8));
        }
    }
}

#[test]
fn meters_have_two_columns_five_rows_and_override_seeded_or_updated_cells() {
    let mut field = PetsciiMeterField::default();
    let mut data = input([0, 31, 32, 63, 64, 96, 128, 255]);
    data.palette = 15;
    field.step(&data);
    for (band, height) in [1, 1, 2, 2, 3, 4, 5, 5].into_iter().enumerate() {
        for y in 0..5 {
            let expected = cell(if y < height { 102 } else { 32 }, band as u8);
            assert_eq!(field.cell(2 + band * 4, 23 - y), expected);
            assert_eq!(field.cell(3 + band * 4, 23 - y), expected);
        }
    }
    // The phase-zero row update at (2,20) is replaced by the empty meter.
    assert_eq!(field.cell(2, 20), cell(32, 0));
    // Unselected row 15 is also overwritten by the meter.
    assert_eq!(field.cell(2, 19), cell(32, 0));
    assert_eq!(field.cell(0, 19), cell(111, 0));
    assert_eq!(field.cell(34, 23), None);
    assert_eq!(field.cell(2, 24), None);
}

#[test]
fn beat_note_scene_spectrum_and_palette_affect_rows_independently() {
    let quiet = input([0; 8]);
    let mut field = PetsciiMeterField::default();
    field.step(&quiet);
    assert_eq!(field.cell(0, 4), cell(87, 1));
    assert_eq!(field.cell(2, 4), cell(98, 1)); // threshold 6 is not below 4
    assert_eq!(field.cell(5, 4), cell(101, 1));

    let mut accented = quiet;
    accented.beat = true;
    accented.note = 13;
    accented.scene = 2;
    accented.palette = 15;
    accented.spectrum[1] = 255;
    let mut field = PetsciiMeterField::default();
    field.step(&accented);
    assert_eq!(field.cell(2, 4), cell(86, 0)); // threshold 6 is below 8
    assert_eq!(field.cell(4, 4), cell(106, 0)); // scene adds six
    assert_eq!(field.cell(5, 4), cell(107, 15)); // 15 + 15 wraps
    assert_eq!(field.cell(6, 4), cell(108, 15));
    assert_eq!(field.cell(6, 23), cell(102, 1)); // second meter, palette wraps

    let mut field = PetsciiMeterField::default();
    field.step(&input([255; 8]));
    assert_eq!(field.cell(0, 4), cell(96, 0)); // threshold 31, colour byte 16
    assert_eq!(field.cell(1, 4), cell(87, 0)); // threshold 2
}

#[test]
fn demo_inputs_are_deterministic_at_phrase_and_palette_boundaries() {
    let first = Inputs::demo(0);
    assert_eq!(first.spectrum, [48, 77, 106, 88, 59, 65, 94, 100]);
    assert_eq!(
        (first.scene, first.palette, first.beat, first.note),
        (0, 0, true, 48)
    );
    let phrase = Inputs::demo(96);
    assert_eq!(
        (phrase.scene, phrase.palette, phrase.beat, phrase.note),
        (1, 3, true, 54)
    );
    assert_eq!(&phrase.spectrum[..2], &[96, 93]);
    assert_eq!(Inputs::demo(255).palette, 7);
    assert_eq!(Inputs::demo(256).palette, 0);
    assert!(!Inputs::demo(4).beat);
    assert_eq!(Inputs::demo(16).note, 49);
}

#[test]
fn skipped_frames_preserve_cells_and_frame_wrap_uses_old_frame_first() {
    let mut field = PetsciiMeterField::default();
    let data = input([0; 8]);
    for _ in 0..u16::MAX {
        field.skip_frame();
    }
    assert_eq!(field.frame(), u16::MAX);
    assert_eq!(field.cell(0, 7), cell(99, 4));
    assert_eq!(field.cell(2, 23), None);
    field.step(&data);
    assert_eq!(field.frame(), 0);
    assert_eq!(field.cell(0, 7), cell(98, 4)); // phase 3, frame / 4 = 16383
    assert_eq!(field.cell(0, 4), cell(96, 1));
    field.step(&data);
    assert_eq!(field.frame(), 1);
    assert_eq!(field.cell(0, 4), cell(87, 1));
    assert_eq!(field.cell(0, 7), cell(98, 4));
}
