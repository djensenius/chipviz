//! Opt-in frozen SameBoy fixture versus the unselectable DMG model and GPU.
//! See tests/README-dmg-parity.md for prerequisites and limitations.

#[expect(dead_code, reason = "only the model API used by this test is needed")]
#[path = "../src/scenes/dmg_tile_scroll.rs"]
mod dmg_tile_scroll;
#[path = "../src/scenes/dmg_tile_scroll_render.rs"]
mod dmg_tile_scroll_render;

use dmg_tile_scroll::{Display, DmgTileScroll};
use dmg_tile_scroll_render::DmgTileRenderer;

const STEPS: [usize; 19] = [
    1, 2, 15, 16, 17, 31, 32, 33, 47, 48, 49, 63, 64, 65, 127, 128, 255, 256, 257,
];
const WIDTH: u32 = 160;
const HEIGHT: u32 = 144;
const RGB_BYTES: usize = WIDTH as usize * HEIGHT as usize * 3;

fn frames(data: &[u8]) -> Result<Vec<&[u8]>, &'static str> {
    if data.len() != STEPS.len() * RGB_BYTES {
        return Err("missing, truncated or extra capture bytes");
    }
    Ok(data.chunks_exact(RGB_BYTES).collect())
}

#[test]
fn capture_protocol_rejects_missing_truncated_and_extra_frames() {
    assert!(frames(&[]).is_err());
    let mut data = vec![0; STEPS.len() * RGB_BYTES];
    assert_eq!(frames(&data).unwrap().len(), STEPS.len());
    data.pop();
    assert!(frames(&data).is_err());
    data.extend_from_slice(&[0, 0]);
    assert!(frames(&data).is_err());
}

#[test]
fn frozen_step_mapping_applies_prepared_registers_without_an_extra_step() {
    let mut scene = DmgTileScroll::default();
    for step in 1..=257 {
        scene.tick();
        if [1, 15, 16, 31, 32, 47, 48, 63, 64, 255, 256, 257].contains(&step) {
            let counter = step as u8;
            let scx = counter.rotate_right(1);
            let expected = Display {
                scx,
                scy: counter,
                bgp: 0xe4 | (scx & 0x18),
            };
            assert_eq!(scene.counter(), counter);
            assert_eq!(scene.prepared(), expected);
            let mut shown = scene.clone();
            shown.tick();
            assert_eq!(shown.shown(), expected);
            assert_eq!(scene.counter(), counter);
        }
    }
}

// Based on the existing dmg_tile_scroll_gpu test's offscreen WGPU readback.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &DmgTileRenderer,
    format: wgpu::TextureFormat,
    display: Display,
) -> Vec<u8> {
    let extent = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("DMG parity target"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let row = (WIDTH * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("DMG parity readback"),
        size: u64::from(row * HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    assert!(renderer.encode(
        queue,
        &mut encoder,
        &texture.create_view(&wgpu::TextureViewDescriptor::default()),
        (WIDTH, HEIGHT),
        display,
    ));
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::ImageCopyBuffer {
            buffer: &buffer,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(HEIGHT),
            },
        },
        extent,
    );
    queue.submit(std::iter::once(encoder.finish()));
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).expect("readback receiver");
    });
    device.poll(wgpu::Maintain::Wait);
    receiver
        .recv()
        .expect("map callback")
        .expect("map readback");
    let mapped = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for row in mapped.chunks_exact(row as usize) {
        rgba.extend_from_slice(&row[..(WIDTH * 4) as usize]);
    }
    drop(mapped);
    buffer.unmap();
    rgba
}

#[test]
#[ignore = "requires RGBDS, SameBoy, a GPU adapter and the serialized heavy slot"]
fn frozen_native_fixture_matches_model_and_offscreen_gpu() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    let output = std::process::Command::new(python)
        .arg(root.join("scripts/gb_dmg_parity_capture.py"))
        .output()
        .expect("run SameBoy fixture capture");
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "fixture capture failed: {}",
        output.status
    );
    let captures = frames(&output.stdout).expect("fixture capture protocol");
    eprintln!("DMG parity selected steps ({}): {STEPS:?}", STEPS.len());

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
    eprintln!("DMG parity adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("DMG parity device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
        },
        None,
    ))
    .expect("request DMG parity device");
    for format in [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Rgba8Unorm,
    ] {
        let renderer = DmgTileRenderer::new(&device, format);
        let mut scene = DmgTileScroll::default();
        let mut count = 0;
        for (&step, native) in STEPS.iter().zip(&captures) {
            while count < step {
                scene.tick();
                count += 1;
            }
            assert_eq!(scene.counter(), step as u8);
            // Fixture freezes after Step and applies its shadows at a later
            // VBlank. A clone's next tick displays those prepared registers;
            // do not advance the real model's counter for the next sample.
            let mut shown = scene.clone();
            shown.tick();
            assert_eq!(shown.shown(), scene.prepared());
            let display = shown.shown();
            let rgba = render(&device, &queue, &renderer, format, display);
            assert_eq!(rgba.len(), WIDTH as usize * HEIGHT as usize * 4);
            for (index, (rgb, pixel)) in
                native.chunks_exact(3).zip(rgba.chunks_exact(4)).enumerate()
            {
                let x = (index % WIDTH as usize) as u8;
                let y = (index / WIDTH as usize) as u8;
                let expected = 255 - 85 * display.shade_at(x, y);
                for channel in 0..3 {
                    // SameBoy greys must match the model exactly. The GPU
                    // may round intermediate sRGB codes by one.
                    assert_eq!(rgb[channel], expected, "native step={step} pixel={index}");
                    let tolerance = u8::from(expected != 0 && expected != 255);
                    assert!(
                        pixel[channel].abs_diff(rgb[channel]) <= tolerance,
                        "{format:?} step={step} pixel={index} native={rgb:?} gpu={pixel:?}"
                    );
                }
                assert_eq!(pixel[3], 255, "{format:?} step={step} pixel={index}");
            }
        }
        eprintln!(
            "DMG parity {format:?}: {} frozen frames matched",
            STEPS.len()
        );
    }
}
