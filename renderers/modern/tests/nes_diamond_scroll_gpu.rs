//! GPU readback against a separate, source-derived indexed-pixel oracle.
//! Run explicitly when the serialized GPU slot is available.

#[path = "../src/scenes/nes_diamond_scroll.rs"]
mod nes_diamond_scroll;
#[path = "../src/scenes/nes_diamond_scroll_gpu.rs"]
mod nes_diamond_scroll_gpu;

use nes_diamond_scroll::{DiamondScroll, Viewport};
use nes_diamond_scroll_gpu::DiamondGpu;

// Independently decoded from the two CHR bitplanes in main.s. Neither this
// matrix nor the palette arrays call the implementation's pixel/RGB helpers.
const TILE: [[usize; 8]; 8] = [
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 1, 1, 0, 0, 0],
    [0, 0, 1, 3, 3, 1, 0, 0],
    [0, 1, 3, 3, 3, 3, 1, 0],
    [1, 3, 3, 3, 3, 3, 3, 1],
    [0, 1, 3, 3, 3, 3, 1, 0],
    [0, 0, 1, 3, 3, 1, 0, 0],
    [0, 0, 0, 1, 1, 0, 0, 0],
];
const BLACK: [u8; 3] = [0, 0, 0];
// Approximate RGB convention, not a measured NES palette. Entry 2 does not
// occur in this CHR tile; the foreground phase only affects palette 0 entry 1.
const FOREGROUND: [[u8; 3]; 8] = [
    [0, 120, 248],
    [0, 88, 248],
    [104, 68, 252],
    [216, 0, 204],
    [228, 0, 88],
    [216, 0, 204],
    [104, 68, 252],
    [0, 88, 248],
];
const ONE: [[u8; 3]; 4] = [[0, 120, 248], [172, 0, 124], [0, 88, 0], [0, 64, 88]];
const THREE: [[u8; 3]; 4] = [
    [164, 228, 252],
    [248, 88, 152],
    [88, 216, 84],
    [56, 248, 248],
];

fn oracle(frame: u8, x: u32, y: u32) -> [u8; 4] {
    // Both vertical nametables begin with the same $e4 attribute bytes.
    let y = (y + u32::from(frame / 2)) % 240;
    let tile_entry = TILE[(y % 8) as usize][(x % 8) as usize];
    let quadrant = match (x % 32 < 16, y % 32 < 16) {
        (true, true) => 0,
        (false, true) => 1,
        (true, false) => 2,
        (false, false) => 3,
    };
    let rgb = match (quadrant, tile_entry) {
        (_, 0) => BLACK,
        (0, 1) => FOREGROUND[usize::from(frame / 32)],
        (_, 1) => ONE[quadrant],
        (_, 3) => THREE[quadrant],
        _ => panic!("unexpected CHR entry"),
    };
    [rgb[0], rgb[1], rgb[2], 255]
}

fn read_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &DiamondGpu,
    scene: &DiamondScroll,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("diamond test readback"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("diamond test encoder"),
    });
    renderer.encode(queue, &mut encoder, scene);
    encoder.copy_texture_to_buffer(
        renderer.target().as_image_copy(),
        wgpu::ImageCopyBuffer {
            buffer: &readback,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));
    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::Maintain::Wait);
    receiver
        .recv()
        .expect("map callback")
        .expect("readback map");
    let bytes = slice.get_mapped_range().to_vec();
    readback.unmap();
    bytes
}

#[test]
#[ignore = "requires a GPU adapter and the serialized GPU slot"]
fn gpu_matches_independent_rom_oracle_on_srgb_and_unorm_targets() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("diamond test device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
        },
        None,
    ))
    .expect("GPU device");

    // Explicit viewport bounds avoid asking the model for expected placement.
    for (width, height, bounds) in [
        (256, 240, (0, 0, 256, 240)),
        (640, 480, (64, 0, 512, 480)),
        (960, 540, (224, 30, 512, 480)),
        (128, 120, (0, 0, 128, 120)),
        (257, 241, (0, 0, 256, 240)),
    ] {
        assert_eq!(
            Viewport::fit(width, height)
                .unwrap()
                .source_pixel(bounds.0, bounds.1),
            Some((0, 0))
        );
        for format in [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ] {
            let renderer = DiamondGpu::new(&device, width, height, format).unwrap();
            let mut scene = DiamondScroll::default();
            let mut previous = 0;
            for step in [0, 1, 2, 31, 32, 64, 128, 255, 256] {
                for _ in previous..step {
                    scene.nmi(true);
                }
                previous = step;
                let expected_frame = step as u8;
                assert_eq!(scene.frame(), expected_frame);
                assert_eq!(scene.scroll_y(), expected_frame / 2);
                assert_eq!(
                    scene.foreground(),
                    [0x11, 0x12, 0x13, 0x14, 0x15, 0x14, 0x13, 0x12]
                        [usize::from(expected_frame / 32)]
                );
                let bytes = read_frame(&device, &queue, &renderer, &scene, width, height);
                let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                    * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                let (left, top, w, h) = bounds;
                for y in 0..height {
                    for x in 0..width {
                        let expected = if x < left || x >= left + w || y < top || y >= top + h {
                            [0, 0, 0, 255]
                        } else {
                            oracle(expected_frame, (x - left) * 256 / w, (y - top) * 240 / h)
                        };
                        let offset = (y * row + x * 4) as usize;
                        let actual = &bytes[offset..offset + 4];
                        assert_eq!(actual[3], 255);
                        for channel in 0..3 {
                            assert!(
                                actual[channel].abs_diff(expected[channel]) <= 1,
                                "{format:?} {width}x{height} step={step} ({x},{y}) channel={channel}: actual={actual:?} expected={expected:?}"
                            );
                        }
                    }
                }
            }
            if (width, height) == (256, 240) {
                scene.leave_demo();
                for _ in 0..31 {
                    scene.nmi(true);
                }
                scene.queue_reentry();
                let offset = ((18 * width + 2) * 4) as usize;
                let hidden = read_frame(&device, &queue, &renderer, &scene, width, height);
                assert_eq!(&hidden[offset..offset + 4], &[0, 0, 0, 255]);
                scene.nmi(true);
                let restored = read_frame(&device, &queue, &renderer, &scene, width, height);
                let expected = [0, 88, 248, 255]; // Frame 32, scrolled into palette 0.
                for channel in 0..4 {
                    assert!(restored[offset + channel].abs_diff(expected[channel]) <= 1);
                }
            }
        }
    }
}
