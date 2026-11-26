//! Opt-in headless readback. Run only while holding the shared exclusive
//! heavy-work lock for both the build and the GPU test.
//!
//! This compares actual offscreen bytes with independent geometric row masks;
//! it is not presented output, a physical Pi test, or a safety assessment.

#[path = "petscii_meter_field_expected/mod.rs"]
mod expected;
#[expect(dead_code, reason = "this fixture uses only the model's rendering API")]
#[path = "../src/scenes/petscii_meter_field.rs"]
mod petscii_meter_field;
#[expect(dead_code, reason = "the renderer uses the cell-level artwork API")]
#[path = "../src/scenes/petscii_meter_field_artwork.rs"]
mod petscii_meter_field_artwork;
#[expect(dead_code, reason = "the portable fixture also exercises composition")]
#[path = "../src/scenes/petscii_meter_field_render.rs"]
mod petscii_meter_field_render;

use petscii_meter_field::{Cell, Inputs, PetsciiMeterField, WIDTH};
use petscii_meter_field_render::{
    CELL_COUNT, ComposeError, PetsciiMeterFieldRenderer, RenderError,
};

struct Offscreen {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Offscreen {
    fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("a headless WGPU PRIMARY adapter is required");
        eprintln!("PETSCII offscreen adapter: {:?}", adapter.get_info());
        for format in [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ] {
            let usage = adapter.get_texture_format_features(format).allowed_usages;
            assert!(
                usage.contains(
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
                ),
                "{format:?} lacks required target/readback usage: {usage:?}"
            );
        }
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("PETSCII offscreen test device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .expect("request PETSCII offscreen test device");
        Self { device, queue }
    }

    fn target(
        &self,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
        layers: u32,
        mip_levels: u32,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PETSCII offscreen test target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: layers,
            },
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    }

    fn read(&self, texture: &wgpu::Texture) -> Vec<u8> {
        let size = texture.size();
        let row_bytes = (size.width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PETSCII offscreen test readback"),
            size: u64::from(row_bytes) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("PETSCII offscreen test readback encoder"),
            });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );
        self.queue.submit(std::iter::once(encoder.finish()));
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .expect("map callback")
            .expect("map readback");
        let mut image = Vec::with_capacity((size.width * size.height * 4) as usize);
        {
            let mapped = slice.get_mapped_range();
            for row in mapped.chunks_exact(row_bytes as usize) {
                image.extend_from_slice(&row[..(size.width * 4) as usize]);
            }
        }
        buffer.unmap();
        image
    }
}

#[derive(Clone, Copy)]
struct Case {
    size: (u32, u32),
    origin: (u32, u32),
    covered: (u32, u32),
    scale: (u32, u32),
}

// Explicit coverage and rational scales, independently of the renderer's
// viewport helper. Fractional cases avoid sample centres on texel boundaries.
const CASES: [Case; 5] = [
    Case {
        size: (320, 200),
        origin: (0, 0),
        covered: (320, 200),
        scale: (1, 1),
    },
    Case {
        size: (643, 403),
        origin: (1, 1),
        covered: (640, 400),
        scale: (2, 1),
    },
    Case {
        size: (325, 501),
        origin: (2, 150),
        covered: (320, 200),
        scale: (1, 1),
    },
    Case {
        size: (192, 124),
        origin: (0, 2),
        covered: (192, 120),
        scale: (3, 5),
    },
    Case {
        size: (128, 220),
        origin: (0, 70),
        covered: (128, 80),
        scale: (2, 5),
    },
];

fn assert_pixels(
    image: &[u8],
    format: wgpu::TextureFormat,
    field: &PetsciiMeterField,
    case: Case,
    label: &str,
) {
    assert_eq!(image.len(), (case.size.0 * case.size.1 * 4) as usize);
    for (index, actual) in image.chunks_exact(4).enumerate() {
        let x = index as u32 % case.size.0;
        let y = index as u32 / case.size.0;
        let inside = x >= case.origin.0
            && x < case.origin.0 + case.covered.0
            && y >= case.origin.1
            && y < case.origin.1 + case.covered.1;
        let expected = if inside {
            let source_x = (2 * (x - case.origin.0) + 1) * case.scale.1 / (2 * case.scale.0);
            let source_y = (2 * (y - case.origin.1) + 1) * case.scale.1 / (2 * case.scale.0);
            expected::field_pixel(field, source_x, source_y)
        } else {
            [0, 0, 0, 255]
        };
        for channel in 0..4 {
            let tolerance = if format.is_srgb()
                && channel < 3
                && expected[channel] != 0
                && expected[channel] != 255
            {
                1
            } else {
                0
            };
            assert!(
                actual[channel].abs_diff(expected[channel]) <= tolerance,
                "{format:?} {label} {:?} ({x},{y}) channel={channel}: actual={actual:?}, expected={expected:?}",
                case.size
            );
        }
    }
}

fn input(spectrum: u8, energetic: bool) -> Inputs {
    Inputs {
        spectrum: [spectrum; 8],
        scene: u8::from(energetic),
        palette: if energetic { 2 } else { 0 },
        beat: false,
        note: if energetic { 5 } else { 6 },
    }
}

fn code_changes(before: &PetsciiMeterField, after: &PetsciiMeterField) -> usize {
    (4..19)
        .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
        .filter(|&(x, y)| before.cell(x, y).map(|c| c.code) != after.cell(x, y).map(|c| c.code))
        .count()
}

#[test]
#[ignore = "requires a headless GPU adapter and the serialized heavy-work lock"]
fn gpu_matches_independent_quiet_and_energetic_composition() {
    let gpu = Offscreen::new();
    assert!(matches!(
        PetsciiMeterFieldRenderer::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm),
        Err(RenderError::UnsupportedFormat(
            wgpu::TextureFormat::Bgra8Unorm
        ))
    ));

    let mut field = PetsciiMeterField::default();
    let mut states = vec![("seed", field.clone())];
    for (label, data) in [
        ("quiet 1", input(0, false)),
        ("quiet 2", input(0, false)),
        ("energetic 1", input(255, true)),
        ("energetic 2", input(255, true)),
        ("quiet again", input(0, false)),
    ] {
        field.step(&data);
        assert!(
            code_changes(&states.last().unwrap().1, &field) > 0,
            "{label}"
        );
        states.push((label, field.clone()));
    }
    assert_eq!(states[1].1.cell(2, 19).unwrap().code, 32);
    assert_eq!(states[3].1.cell(2, 19).unwrap().code, 102);
    assert_eq!(states[5].1.cell(2, 19).unwrap().code, 32);

    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        let mut renderer = PetsciiMeterFieldRenderer::new(&gpu.device, &gpu.queue, format).unwrap();
        for case in CASES {
            let target = gpu.target(case.size, format, usage, 1, 1);
            let mut previous = None;
            for (label, state) in &states {
                let frame = state.frame();
                renderer.render(&target, state).unwrap();
                assert_eq!(state.frame(), frame, "render must not step the model");
                let image = gpu.read(&target);
                assert_pixels(&image, format, state, case, label);
                if let Some(previous) = &previous {
                    assert_ne!(&image, previous, "{format:?} {label} {:?}", case.size);
                }
                previous = Some(image);
            }
        }

        // Two different targets are rendered before either is read. Each
        // submission must retain its own frame despite the shared upload texture.
        let first = gpu.target(CASES[0].size, format, usage, 1, 1);
        let second = gpu.target(CASES[0].size, format, usage, 1, 1);
        renderer.render(&first, &states[1].1).unwrap();
        renderer.render(&second, &states[3].1).unwrap();
        assert_pixels(&gpu.read(&first), format, &states[1].1, CASES[0], "first");
        assert_pixels(&gpu.read(&second), format, &states[3].1, CASES[0], "second");

        let mut cells = vec![None; CELL_COUNT];
        cells[17] = Some(Cell {
            code: 200,
            colour: 2,
        });
        assert_eq!(
            renderer.render_cells(&first, &cells),
            Err(RenderError::Composition(ComposeError::UnsupportedCode {
                x: 17,
                y: 0,
                code: 200,
            }))
        );
        cells[17] = Some(Cell {
            code: 102,
            colour: 16,
        });
        assert_eq!(
            renderer.render_cells(&first, &cells),
            Err(RenderError::Composition(ComposeError::InvalidColour {
                x: 17,
                y: 0,
                colour: 16,
            }))
        );
        assert_eq!(
            renderer.render_cells(&first, &[]),
            Err(RenderError::Composition(ComposeError::CellCount {
                expected: CELL_COUNT,
                actual: 0,
            }))
        );
        assert_pixels(
            &gpu.read(&first),
            format,
            &states[1].1,
            CASES[0],
            "rejected evidence left target unchanged",
        );

        let other_format = if format.is_srgb() {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        };
        let wrong_format = gpu.target(CASES[0].size, other_format, usage, 1, 1);
        assert_eq!(
            renderer.render(&wrong_format, &states[1].1),
            Err(RenderError::TargetFormat {
                expected: format,
                actual: other_format,
            })
        );
        let no_attachment = gpu.target(CASES[0].size, format, wgpu::TextureUsages::COPY_SRC, 1, 1);
        assert_eq!(
            renderer.render(&no_attachment, &states[1].1),
            Err(RenderError::TargetUsage)
        );
        for (layers, mip_levels) in [(2, 1), (1, 2)] {
            let wrong_shape = gpu.target(CASES[0].size, format, usage, layers, mip_levels);
            assert_eq!(
                renderer.render(&wrong_shape, &states[1].1),
                Err(RenderError::TargetShape {
                    dimension: wgpu::TextureDimension::D2,
                    layers,
                    mip_levels,
                    samples: 1,
                })
            );
        }
    }
    eprintln!(
        "PETSCII offscreen readback: both formats, six model states, five viewport cases; \
         no presentation, physical Pi, or safety conclusion"
    );
}
