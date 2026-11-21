//! Offscreen prototype regression; this does not register a selectable scene.
//!
//! Once the serialized heavy slot is available, run from the repository root:
//! `cargo test --manifest-path renderers/modern/Cargo.toml --test dmg_tile_scroll_gpu -- --ignored --nocapture`
//! It requires a headless WGPU PRIMARY adapter (e.g. Metal on M1 or Vulkan on
//! Pi) supporting R8Unorm texture binding and RGBA8 UNORM and sRGB render
//! attachments. It prints the actual adapter and fails if none is available.
//! On integration, register this ignored test in a standard GPU-capable CI
//! target; an ordinary `cargo test` does not run it.

#[expect(
    dead_code,
    reason = "this fixture uses only the model API needed for GPU readback"
)]
#[path = "../src/scenes/dmg_tile_scroll.rs"]
mod dmg_tile_scroll;
#[path = "../src/scenes/dmg_tile_scroll_render.rs"]
mod dmg_tile_scroll_render;

use dmg_tile_scroll::{Display, DmgTileScroll};
use dmg_tile_scroll_render::DmgTileRenderer;

// Independently decoded from cores/gb/homebrew/main.asm's Tiles: each ASCII
// digit is a DMG colour number in left-to-right order, not a byte plane or a
// copy of the renderer's shade buffer. The rows are top-to-bottom.
const TILES: [[&str; 8]; 4] = [
    [
        "00000000", "00033000", "00333300", "03333330", "33333333", "03333330", "00333300",
        "00033000",
    ],
    [
        "30000003", "03000030", "00300300", "00033000", "00033000", "00300300", "03000030",
        "30000003",
    ],
    [
        "11111111", "12222221", "12111121", "12122121", "12122121", "12111121", "12222221",
        "11111111",
    ],
    [
        "22212221", "22122212", "21222122", "12221222", "22212221", "22122212", "21222122",
        "12221222",
    ],
];

#[derive(Clone, Copy)]
struct Example {
    ticks: usize,
    scx: u8,
    scy: u8,
    bgp: u8,
    // Explicit BGP colour-number-to-sRGB-byte mapping.
    shades: [u8; 4],
}

// NativeStep increments wFrame, rotates it right once, then masks/or's BGP;
// ApplyDisplay exposes the previous step at VBlank. Values here were worked
// out from the assembly, independently of the model's registers.
const EXAMPLES: [Example; 7] = [
    Example {
        ticks: 0,
        scx: 0,
        scy: 0,
        bgp: 0xe4,
        shades: [255, 170, 85, 0],
    },
    Example {
        ticks: 2,
        scx: 128,
        scy: 1,
        bgp: 0xe4,
        shades: [255, 170, 85, 0],
    },
    Example {
        ticks: 17,
        scx: 8,
        scy: 16,
        bgp: 0xec,
        shades: [255, 0, 85, 0],
    },
    Example {
        ticks: 33,
        scx: 16,
        scy: 32,
        bgp: 0xf4,
        shades: [255, 170, 0, 0],
    },
    Example {
        ticks: 49,
        scx: 24,
        scy: 48,
        bgp: 0xfc,
        shades: [255, 0, 0, 0],
    },
    Example {
        ticks: 256,
        scx: 255,
        scy: 255,
        bgp: 0xfc,
        shades: [255, 0, 0, 0],
    },
    Example {
        ticks: 257,
        scx: 0,
        scy: 0,
        bgp: 0xe4,
        shades: [255, 170, 85, 0],
    },
];

fn expected_pixel(example: Example, x: u32, y: u32) -> u8 {
    // LoadMap writes tile IDs 0,1,2,3 through 32 columns on every row.
    let map_x = (x + u32::from(example.scx)) % 256;
    let map_y = (y + u32::from(example.scy)) % 256;
    let tile = ((map_x / 8) % 4) as usize;
    let colour = TILES[tile][(map_y % 8) as usize].as_bytes()[(map_x % 8) as usize] - b'0';
    example.shades[colour as usize]
}

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
        .expect("headless WGPU PRIMARY adapter required");
        eprintln!("DMG offscreen adapter: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("DMG offscreen test device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .expect("request DMG test device");
        Self { device, queue }
    }

    fn render(
        &self,
        renderer: &DmgTileRenderer,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        display: Display,
        check_zero: bool,
    ) -> Vec<u8> {
        let (width, height) = size;
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("DMG test target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let row_bytes = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DMG test readback"),
            size: u64::from(row_bytes) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("DMG test encoder"),
            });
        assert!(renderer.encode(&self.queue, &mut encoder, &view, size, display));
        if check_zero {
            // These invalid sizes must neither upload nor add a clearing pass.
            // A different all-white display exposes an unintended upload.
            // The final readback must still match the valid preceding draw.
            let unexpected = Display {
                scx: 0,
                scy: 0,
                bgp: 0,
            };
            for empty in [(0, height), (width, 0), (0, 0)] {
                assert!(!renderer.encode(&self.queue, &mut encoder, &view, empty, unexpected));
            }
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(height),
                },
            },
            extent,
        );
        self.queue.submit(std::iter::once(encoder.finish()));
        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .expect("map callback")
            .expect("map readback");
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        {
            let mapped = slice.get_mapped_range();
            for row in mapped.chunks_exact(row_bytes as usize) {
                rgba.extend_from_slice(&row[..(width * 4) as usize]);
            }
        }
        readback.unmap();
        rgba
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn dmg_shader_matches_native_examples_on_srgb_and_unorm_targets() {
    let gpu = Offscreen::new();
    for format in [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Rgba8Unorm,
    ] {
        let renderer = DmgTileRenderer::new(&gpu.device, format);
        let mut scene = DmgTileScroll::default();
        let mut ticks = 0;
        for example in EXAMPLES {
            while ticks < example.ticks {
                scene.tick();
                ticks += 1;
            }
            assert_eq!(
                scene.shown(),
                Display {
                    scx: example.scx,
                    scy: example.scy,
                    bgp: example.bgp
                },
                "native display at tick {ticks}"
            );
            // Explicit pixel coverage and rational scales, independently of
            // the renderer's viewport helper. At 3/5 scale the 86.4-high
            // image starts at y=1: 86 pixel centres are covered, leaving one
            // black row above and three below. At 4/5 it is 115.2 high, so
            // 115 centres are covered and the final row is black. Neither
            // fractional scale puts a sample centre on a texel boundary.
            // The portrait case has asymmetric horizontal and vertical bars.
            for (size, origin, covered, scale_num, scale_den) in [
                ((160, 144), (0, 0), (160, 144), 1, 1),
                ((323, 291), (1, 1), (320, 288), 2, 1),
                ((96, 90), (0, 1), (96, 86), 3, 5),
                ((128, 116), (0, 0), (128, 115), 4, 5),
                ((163, 293), (1, 74), (160, 144), 1, 1),
            ] {
                let rgba = gpu.render(&renderer, format, size, scene.shown(), ticks == 0);
                for y in 0..size.1 {
                    for x in 0..size.0 {
                        let inside = x >= origin.0
                            && x < origin.0 + covered.0
                            && y >= origin.1
                            && y < origin.1 + covered.1;
                        let expected = if inside {
                            // Floor the source coordinate at each output
                            // pixel centre using exact integer arithmetic.
                            let source_x = (2 * (x - origin.0) + 1) * scale_den / (2 * scale_num);
                            let source_y = (2 * (y - origin.1) + 1) * scale_den / (2 * scale_num);
                            expected_pixel(example, source_x, source_y)
                        } else {
                            0
                        };
                        let offset = ((y * size.0 + x) * 4) as usize;
                        let pixel = &rgba[offset..offset + 4];
                        // Allow a single code of sRGB conversion rounding for
                        // intermediate greys, but require exact black/white.
                        let tolerance = if expected == 0 || expected == 255 {
                            0
                        } else {
                            1
                        };
                        for &channel in &pixel[..3] {
                            assert!(
                                channel.abs_diff(expected) <= tolerance,
                                "{format:?} tick={ticks} size={size:?} ({x},{y}) expected {expected}, got {pixel:?}"
                            );
                        }
                        assert_eq!(pixel[3], 255, "{format:?} tick={ticks} ({x},{y})");
                    }
                }
            }
        }
    }
}
