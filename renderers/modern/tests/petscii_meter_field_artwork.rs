// Exercise the artwork seam without registering it in the application.
#[path = "../src/scenes/petscii_meter_field.rs"]
mod petscii_meter_field;
#[path = "../src/scenes/petscii_meter_field_artwork.rs"]
mod petscii_meter_field_artwork;

use std::collections::HashSet;

use petscii_meter_field::{Cell, HEIGHT, Inputs, PetsciiMeterField, WIDTH};
use petscii_meter_field_artwork::{CELL_SIZE, PIXEL_HEIGHT, PIXEL_WIDTH, Pixel, cell_pixel, pixel};

fn input(spectrum: [u8; 8], note: u8, beat: bool) -> Inputs {
    Inputs {
        spectrum,
        scene: 2,
        palette: 15,
        beat,
        note,
    }
}

#[test]
fn all_emitted_codes_have_distinct_geometric_artwork() {
    let mut patterns = HashSet::new();
    for code in std::iter::once(32).chain(81..=88).chain(96..=111) {
        let cell = Some(Cell { code, colour: 7 });
        let pattern: Vec<_> = (0..CELL_SIZE)
            .flat_map(|y| (0..CELL_SIZE).map(move |x| cell_pixel(cell, x, y).unwrap()))
            .collect();
        assert!(patterns.insert(pattern.clone()), "duplicate code {code}");
        if code == 32 {
            assert!(pattern.iter().all(|p| *p == Pixel::Background));
        } else if code == 102 {
            assert!(pattern.iter().all(|p| *p == Pixel::Foreground(7)));
        } else {
            assert!(pattern.contains(&Pixel::Background), "code {code}");
            assert!(pattern.contains(&Pixel::Foreground(7)), "code {code}");
        }
    }
}

#[test]
fn blank_unknown_zero_colour_and_bounds_are_distinct() {
    assert_eq!(cell_pixel(None, 0, 0), Some(Pixel::UnknownCell));
    assert_eq!(
        cell_pixel(
            Some(Cell {
                code: 32,
                colour: 0
            }),
            0,
            0
        ),
        Some(Pixel::Background)
    );
    assert_eq!(
        cell_pixel(
            Some(Cell {
                code: 102,
                colour: 0
            }),
            0,
            0
        ),
        Some(Pixel::Foreground(0))
    );
    assert_eq!(
        cell_pixel(
            Some(Cell {
                code: 102,
                colour: 16
            }),
            0,
            0
        ),
        Some(Pixel::Foreground(0))
    );
    assert_eq!(
        cell_pixel(
            Some(Cell {
                code: 255,
                colour: 0
            }),
            0,
            0
        ),
        Some(Pixel::UnknownCode(255))
    );
    assert_eq!(cell_pixel(None, CELL_SIZE, 0), None);
    assert_eq!(cell_pixel(None, 0, usize::MAX), None);

    let field = PetsciiMeterField::default();
    assert_eq!(pixel(&field, 0, 0), Some(Pixel::UnknownCell));
    assert_eq!(pixel(&field, PIXEL_WIDTH, 0), None);
    assert_eq!(pixel(&field, 0, PIXEL_HEIGHT), None);
    assert_eq!(pixel(&field, usize::MAX, usize::MAX), None);
    assert_eq!(
        pixel(&field, PIXEL_WIDTH - 1, PIXEL_HEIGHT - 1),
        Some(Pixel::UnknownCell)
    );
}

#[test]
fn demo_compositing_is_unchanged_by_a_skipped_frame() {
    let mut field = PetsciiMeterField::default();
    field.step(&Inputs::demo(96));
    let before: Vec<_> = (0..PIXEL_HEIGHT)
        .flat_map(|y| (0..PIXEL_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| pixel(&field, x, y))
        .collect();
    field.skip_frame();
    let after: Vec<_> = (0..PIXEL_HEIGHT)
        .flat_map(|y| (0..PIXEL_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| pixel(&field, x, y))
        .collect();
    assert_eq!(before, after);
}

#[test]
fn quiet_and_energetic_models_compose_deterministically() {
    let mut seen_codes = HashSet::new();
    for data in [input([0; 8], 6, false), input([255; 8], 5, true)] {
        let mut field = PetsciiMeterField::default();
        let mut repeat = PetsciiMeterField::default();
        assert_eq!(field.frame(), 0);
        for _ in 0..4 {
            field.step(&data);
            repeat.step(&data);
        }
        assert_eq!(field.frame(), 4);
        for y in 0..PIXEL_HEIGHT {
            for x in 0..PIXEL_WIDTH {
                let cell = field.cell(x / CELL_SIZE, y / CELL_SIZE);
                if let Some(cell) = cell {
                    seen_codes.insert(cell.code);
                }
                let actual = pixel(&field, x, y);
                assert_eq!(actual, pixel(&repeat, x, y), "({x},{y})");
                assert_eq!(actual, cell_pixel(cell, x % CELL_SIZE, y % CELL_SIZE));
                assert!(!matches!(actual, Some(Pixel::UnknownCode(_))));
                if let Some(Pixel::Foreground(colour)) = actual {
                    assert!(colour < 16);
                }
            }
        }
        assert_eq!(pixel(&field, 0, 0), Some(Pixel::UnknownCell));
        let meter = field.cell(2, 23).unwrap();
        assert_eq!(meter.code, 102);
        assert_eq!(
            pixel(&field, 2 * CELL_SIZE, 23 * CELL_SIZE),
            Some(Pixel::Foreground(0))
        );
        if data.spectrum[0] == 0 {
            assert_eq!(field.cell(2, 19).unwrap().code, 32);
            assert_eq!(
                pixel(&field, 2 * CELL_SIZE, 19 * CELL_SIZE),
                Some(Pixel::Background)
            );
        } else {
            assert_eq!(field.cell(2, 19).unwrap().code, 102);
            assert_eq!(
                pixel(&field, 2 * CELL_SIZE, 19 * CELL_SIZE),
                Some(Pixel::Foreground(0))
            );
        }
    }
    assert!(seen_codes.contains(&32));
    assert!(seen_codes.contains(&102));
    assert!((96..=111).all(|code| seen_codes.contains(&code)));

    // Each note selects a different accent in actual model output.
    for note in 0..8 {
        let mut field = PetsciiMeterField::default();
        field.step(&input([0; 8], note, true));
        let accent = 81 + note;
        assert!(
            (0..HEIGHT).any(|y| (0..WIDTH).any(|x| {
                field.cell(x, y).is_some_and(|cell| cell.code == accent)
                    && (0..CELL_SIZE).any(|py| {
                        (0..CELL_SIZE).any(|px| {
                            matches!(
                                pixel(&field, x * CELL_SIZE + px, y * CELL_SIZE + py),
                                Some(Pixel::Foreground(_))
                            )
                        })
                    })
            })),
            "accent {accent}"
        );
    }
}
