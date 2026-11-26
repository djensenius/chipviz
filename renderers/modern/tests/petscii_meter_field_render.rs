//! Portable tests for the unregistered offscreen renderer's CPU seam.

#[expect(dead_code, reason = "the GPU fixture also uses the field oracle")]
#[path = "petscii_meter_field_expected/mod.rs"]
mod expected;
#[expect(dead_code, reason = "this fixture uses only the model's rendering API")]
#[path = "../src/scenes/petscii_meter_field.rs"]
mod petscii_meter_field;
#[expect(dead_code, reason = "the renderer uses the cell-level artwork API")]
#[path = "../src/scenes/petscii_meter_field_artwork.rs"]
mod petscii_meter_field_artwork;
#[expect(dead_code, reason = "this fixture does not construct a GPU renderer")]
#[path = "../src/scenes/petscii_meter_field_render.rs"]
mod petscii_meter_field_render;

use petscii_meter_field::{Cell, Inputs, PetsciiMeterField, WIDTH};
use petscii_meter_field_artwork::Pixel;
use petscii_meter_field_render::{
    BACKGROUND, CELL_COUNT, ComposeError, FRAME_BYTES, MAX_TARGET_DIMENSION, PALETTE, RenderError,
    UNKNOWN_CELL, Viewport, compose, compose_cells, compose_with, validate_limits, viewport,
};

fn input(spectrum: u8) -> Inputs {
    Inputs {
        spectrum: [spectrum; 8],
        scene: 0,
        palette: 0,
        beat: false,
        note: 6,
    }
}

fn code_changes(before: &PetsciiMeterField, after: &PetsciiMeterField) -> usize {
    (4..19)
        .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
        .filter(|&(x, y)| before.cell(x, y).map(|c| c.code) != after.cell(x, y).map(|c| c.code))
        .count()
}

#[test]
fn composition_matches_every_original_mark_and_illustrative_colour() {
    let mut cells = vec![None; CELL_COUNT];
    let codes = std::iter::once(32).chain(81..=88).chain(96..=111);
    for (index, (code, colour)) in codes
        .flat_map(|code| (0..16).map(move |colour| (code, colour)))
        .enumerate()
    {
        cells[index] = Some(Cell { code, colour });
    }
    let image = compose_cells(&cells).unwrap();
    assert_eq!(image.len(), FRAME_BYTES);
    for y in 0..200 {
        for x in 0..320 {
            let cell = cells[(y / 8) * WIDTH + x / 8];
            let offset = (y * 320 + x) * 4;
            assert_eq!(
                &image[offset..offset + 4],
                &expected::cell_pixel(cell, x % 8, y % 8),
                "pixel ({x},{y}), cell {cell:?}"
            );
        }
    }
    assert_ne!(UNKNOWN_CELL, BACKGROUND);
    assert_ne!(UNKNOWN_CELL, [0, 0, 0, 255]);
    assert!(PALETTE.iter().all(|colour| *colour != UNKNOWN_CELL));
    assert_ne!(PALETTE[0], BACKGROUND);
}

#[test]
fn malformed_counts_colours_and_unsupported_codes_are_errors() {
    for length in [0, CELL_COUNT - 1, CELL_COUNT + 1] {
        assert_eq!(
            compose_cells(&vec![None; length]),
            Err(ComposeError::CellCount {
                expected: CELL_COUNT,
                actual: length,
            })
        );
    }
    let mut cells = vec![None; CELL_COUNT];
    let index = 12 * WIDTH + 17;
    for code in [0, 31, 33, 80, 89, 95, 112, 255] {
        cells[index] = Some(Cell { code, colour: 5 });
        assert_eq!(
            compose_cells(&cells),
            Err(ComposeError::UnsupportedCode { x: 17, y: 12, code })
        );
    }
    for (code, colour) in [(32, 16), (102, 255)] {
        cells[index] = Some(Cell { code, colour });
        assert_eq!(
            compose_cells(&cells),
            Err(ComposeError::InvalidColour {
                x: 17,
                y: 12,
                colour,
            })
        );
    }
}

#[test]
fn contradictory_or_missing_artwork_evidence_is_never_valid_art() {
    let mut cells = vec![None; CELL_COUNT];
    let error = Err(ComposeError::InconsistentArtwork {
        x: 0,
        y: 0,
        pixel_x: 0,
        pixel_y: 0,
    });
    for evidence in [
        None,
        Some(Pixel::Background),
        Some(Pixel::Foreground(0)),
        Some(Pixel::UnknownCode(5)),
    ] {
        assert_eq!(compose_with(&cells, |_, _, _| evidence), error);
    }
    cells[0] = Some(Cell {
        code: 102,
        colour: 4,
    });
    for evidence in [
        None,
        Some(Pixel::UnknownCell),
        Some(Pixel::UnknownCode(102)),
        Some(Pixel::Background),
        Some(Pixel::Foreground(5)),
        Some(Pixel::Foreground(16)),
    ] {
        assert_eq!(compose_with(&cells, |_, _, _| evidence), error);
    }
    cells[0] = Some(Cell {
        code: 32,
        colour: 0,
    });
    assert_eq!(
        compose_with(&cells, |_, _, _| Some(Pixel::Foreground(0))),
        error
    );

    cells[0] = Some(Cell {
        code: 96,
        colour: 4,
    });
    for (pixel_x, evidence) in [(0, Pixel::Background), (2, Pixel::Foreground(4))] {
        assert_eq!(
            compose_with(&cells, |cell, x, y| {
                if (x, y) == (pixel_x, 0) {
                    Some(evidence)
                } else {
                    petscii_meter_field_artwork::cell_pixel(cell, x, y)
                }
            }),
            Err(ComposeError::InconsistentArtwork {
                x: 0,
                y: 0,
                pixel_x,
                pixel_y: 0,
            })
        );
    }
}

#[test]
fn insufficient_device_limits_are_rejected_before_resource_creation() {
    let adequate = wgpu::Limits {
        max_texture_dimension_2d: 320,
        max_texture_array_layers: 1,
        max_bind_groups: 1,
        max_bindings_per_bind_group: 1,
        max_sampled_textures_per_shader_stage: 1,
        max_inter_stage_shader_components: 2,
        max_color_attachments: 1,
        max_color_attachment_bytes_per_sample: 8,
        ..wgpu::Limits::default()
    };
    assert_eq!(validate_limits(&adequate), Ok(320));
    type LimitCase = (&'static str, u32, fn(&mut wgpu::Limits));
    let cases: [LimitCase; 8] = [
        ("max_texture_dimension_2d", 320, |l| {
            l.max_texture_dimension_2d = 319
        }),
        ("max_texture_array_layers", 1, |l| {
            l.max_texture_array_layers = 0
        }),
        ("max_bind_groups", 1, |l| l.max_bind_groups = 0),
        ("max_bindings_per_bind_group", 1, |l| {
            l.max_bindings_per_bind_group = 0
        }),
        ("max_sampled_textures_per_shader_stage", 1, |l| {
            l.max_sampled_textures_per_shader_stage = 0
        }),
        ("max_inter_stage_shader_components", 2, |l| {
            l.max_inter_stage_shader_components = 1
        }),
        ("max_color_attachments", 1, |l| l.max_color_attachments = 0),
        ("max_color_attachment_bytes_per_sample", 8, |l| {
            l.max_color_attachment_bytes_per_sample = 7
        }),
    ];
    for (name, required, reduce) in cases {
        let mut limits = adequate.clone();
        reduce(&mut limits);
        assert_eq!(
            validate_limits(&limits),
            Err(RenderError::DeviceLimit {
                name,
                required,
                available: required - 1,
            })
        );
    }
    let generous = wgpu::Limits {
        max_texture_dimension_2d: MAX_TARGET_DIMENSION + 1,
        ..wgpu::Limits::default()
    };
    assert_eq!(validate_limits(&generous), Ok(MAX_TARGET_DIMENSION));
}

#[test]
fn caller_driven_quiet_and_energetic_steps_move_cells_and_meters() {
    let mut field = PetsciiMeterField::default();
    let initial = field.clone();
    let initial_image = compose(&field).unwrap();
    assert_eq!(field.frame(), 0);

    field.step(&input(0));
    let quiet = field.clone();
    let quiet_image = compose(&field).unwrap();
    assert_eq!(field.frame(), 1);
    assert!(code_changes(&initial, &quiet) > 0);
    assert_eq!(field.cell(2, 19).unwrap().code, 32);
    assert_eq!(field.cell(2, 23).unwrap().code, 102);
    assert_ne!(initial_image, quiet_image);

    field.step(&input(0));
    let quiet_again = field.clone();
    assert_eq!(field.frame(), 2);
    assert!(code_changes(&quiet, &quiet_again) > 0);
    assert_ne!(quiet_image, compose(&field).unwrap());

    field.step(&input(255));
    let energetic = field.clone();
    let energetic_image = compose(&field).unwrap();
    assert_eq!(field.frame(), 3);
    assert!(code_changes(&quiet_again, &energetic) > 0);
    assert_eq!(field.cell(2, 19).unwrap().code, 102);
    assert_ne!(quiet_image, energetic_image);

    field.step(&input(0));
    assert_eq!(field.frame(), 4);
    assert!(code_changes(&energetic, &field) > 0);
    assert_eq!(field.cell(2, 19).unwrap().code, 32);
    assert_ne!(energetic_image, compose(&field).unwrap());
    let before_skip = compose(&field).unwrap();
    field.skip_frame();
    assert_eq!(field.frame(), 5);
    assert_eq!(before_skip, compose(&field).unwrap());
}

#[test]
fn viewport_preserves_aspect_bounds_and_integer_scaling() {
    for size in [
        (0, 200),
        (320, 0),
        (0, 0),
        (MAX_TARGET_DIMENSION + 1, 200),
        (320, u32::MAX),
    ] {
        assert_eq!(viewport(size.0, size.1), None, "{size:?}");
    }
    for (size, expected) in [
        (
            (320, 200),
            Viewport {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 200.0,
            },
        ),
        (
            (643, 403),
            Viewport {
                x: 1.0,
                y: 1.0,
                width: 640.0,
                height: 400.0,
            },
        ),
        (
            (3840, 2160),
            Viewport {
                x: 320.0,
                y: 80.0,
                width: 3200.0,
                height: 2000.0,
            },
        ),
        (
            (192, 124),
            Viewport {
                x: 0.0,
                y: 2.0,
                width: 192.0,
                height: 120.0,
            },
        ),
        (
            (128, 220),
            Viewport {
                x: 0.0,
                y: 70.0,
                width: 128.0,
                height: 80.0,
            },
        ),
    ] {
        assert_eq!(viewport(size.0, size.1), Some(expected), "{size:?}");
    }
    for (width, height) in [
        (1, 1),
        (1, MAX_TARGET_DIMENSION),
        (MAX_TARGET_DIMENSION, 1),
        (MAX_TARGET_DIMENSION, MAX_TARGET_DIMENSION),
        (319, 199),
        (321, 201),
        (325, 501),
    ] {
        let view = viewport(width, height).unwrap();
        assert!(view.x >= 0.0 && view.y >= 0.0);
        assert!(view.width > 0.0 && view.height > 0.0);
        assert!(view.x + view.width <= width as f32);
        assert!(view.y + view.height <= height as f32);
        assert!((view.width / view.height - 1.6).abs() < 0.00001);
    }
}
