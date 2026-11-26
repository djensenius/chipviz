//! Opt-in GPU readback against the native C oracle. Acquire the shared
//! heavy-work lock before building or running either ignored test.
//! These offscreen bytes do not establish presentation or flash safety.

#[path = "gba_blockscape_expected/mod.rs"]
mod expected;
#[path = "../src/scenes/gba_blockscape.rs"]
mod gba_blockscape;
#[path = "../src/scenes/gba_blockscape_render.rs"]
mod gba_blockscape_render;

use expected::NativeOracle;
use gba_blockscape::{
    DemoState, Frame, HEIGHT, KEY_A, KEY_B, KEY_SELECT, KEY_START, WIDTH, compose,
};
use gba_blockscape_render::{GbaBlockscapeRenderer, RenderError};

static GPU_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Offscreen {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Offscreen {
    fn new(backends: wgpu::Backends) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("a headless adapter for the requested WGPU backend is required");
        let info = adapter.get_info();
        if backends == wgpu::Backends::GL {
            assert_eq!(info.backend, wgpu::Backend::Gl);
        }
        eprintln!("GBA Blockscape offscreen adapter: {info:?}");
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
                label: Some("GBA Blockscape offscreen test device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .expect("request offscreen test device");
        Self { device, queue }
    }

    fn target(
        &self,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
        layers: u32,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("GBA Blockscape offscreen test target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
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
            label: Some("GBA Blockscape offscreen test readback"),
            size: u64::from(row_bytes) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("GBA Blockscape offscreen readback encoder"),
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

fn expected_pixel(native: &Frame, size: (u32, u32), x: u32, y: u32) -> [u8; 4] {
    let fit = (f64::from(size.0) / WIDTH as f64).min(f64::from(size.1) / HEIGHT as f64);
    let scale = if fit >= 1.0 { fit.floor() } else { fit };
    let left = (f64::from(size.0) - WIDTH as f64 * scale) / 2.0;
    let top = (f64::from(size.1) - HEIGHT as f64 * scale) / 2.0;
    let left = if fit >= 1.0 { left.floor() } else { left };
    let top = if fit >= 1.0 { top.floor() } else { top };
    let source_x = (f64::from(x) + 0.5 - left) / scale;
    let source_y = (f64::from(y) + 0.5 - top) / scale;
    if source_x < 0.0 || source_y < 0.0 || source_x >= WIDTH as f64 || source_y >= HEIGHT as f64 {
        return [0, 0, 0, 255];
    }
    let source_x = (source_x + 1e-9).floor() as usize;
    let source_y = (source_y + 1e-9).floor() as usize;
    let colour = native.palette[usize::from(native.indices[source_y * WIDTH + source_x])];
    let convert = |shift: u32| ((u32::from((colour >> shift) & 31u16) * 255 + 15) / 31) as u8;
    [convert(0), convert(5), convert(10), 255]
}

fn assert_matches_native(
    image: &[u8],
    native: &Frame,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) {
    assert_eq!(image.len(), (size.0 * size.1 * 4) as usize);
    for y in 0..size.1 {
        for x in 0..size.0 {
            let offset = ((y * size.0 + x) * 4) as usize;
            let expected = expected_pixel(native, size, x, y);
            let actual = &image[offset..offset + 4];
            assert!(
                actual[0].abs_diff(expected[0]) <= 1
                    && actual[1].abs_diff(expected[1]) <= 1
                    && actual[2].abs_diff(expected[2]) <= 1
                    && actual[3] == expected[3],
                "{format:?} {size:?} pixel ({x},{y}): got {actual:?}, expected {expected:?}"
            );
        }
    }
}

#[test]
#[ignore = "requires a headless GPU adapter and the serialized heavy-work lock"]
fn gpu_matches_native_source_and_fractional_scaling() {
    check_gpu(wgpu::Backends::PRIMARY);
}

#[test]
#[ignore = "requires a headless GL adapter and the serialized heavy-work lock"]
fn gpu_gl_matches_native_source_and_fractional_scaling() {
    check_gpu(wgpu::Backends::GL);
}

fn check_gpu(backends: wgpu::Backends) {
    let _guard = GPU_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let offscreen = Offscreen::new(backends);
    let oracle = NativeOracle::compile();
    let cases = [
        ((240, 160), 0, 0, 0, false, 0),
        ((480, 320), 1, 97, 3, true, KEY_A),
        ((241, 161), 2, 8193, 5, false, KEY_START),
        ((120, 80), 3, 65535, 7, true, KEY_A | KEY_B | KEY_SELECT),
        ((1, 1), 0, 16, 0, false, 0),
        ((239, 1), 1, 37, 2, false, 0),
        ((1, 239), 2, 81, 6, false, 0),
        ((200, 100), 3, 19, 4, false, KEY_B),
        ((73, 40), 0, 400, 1, false, KEY_SELECT),
    ];
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        let mut renderer =
            GbaBlockscapeRenderer::new(&offscreen.device, &offscreen.queue, format).unwrap();
        assert!(matches!(
            GbaBlockscapeRenderer::new(
                &offscreen.device,
                &offscreen.queue,
                wgpu::TextureFormat::Bgra8Unorm
            ),
            Err(RenderError::UnsupportedFormat(
                wgpu::TextureFormat::Bgra8Unorm
            ))
        ));
        for &(size, scene, frame, palette, beat, keys) in &cases {
            let state = DemoState {
                spectrum: [0, 31, 32, 63, 64, 127, 128, 255],
                scene,
                palette,
                energy: 173,
                beat,
                note: 73,
            };
            let model = compose(frame, state, keys).unwrap();
            let native = oracle.compose(frame, state, keys);
            let target = offscreen.target(size, format, usage, 1);
            renderer.render(&target, &model).unwrap();
            let image = offscreen.read(&target);
            assert_matches_native(&image, &native, size, format);

            let mut malformed = model;
            malformed.indices.pop();
            assert!(matches!(
                renderer.render(&target, &malformed),
                Err(RenderError::FrameLength { .. })
            ));
            assert_eq!(
                offscreen.read(&target),
                image,
                "rejected frame changed target"
            );
        }
        let state = DemoState {
            spectrum: [0; 8],
            scene: 0,
            palette: 0,
            energy: 0,
            beat: false,
            note: 0,
        };
        let frame = compose(0, state, 0).unwrap();
        let no_attachment = offscreen.target((4, 4), format, wgpu::TextureUsages::COPY_SRC, 1);
        assert_eq!(
            renderer.render(&no_attachment, &frame),
            Err(RenderError::TargetUsage)
        );
        let array = offscreen.target((4, 4), format, usage, 2);
        assert_eq!(
            renderer.render(&array, &frame),
            Err(RenderError::TargetShape)
        );
        let wrong_format = match format {
            wgpu::TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8UnormSrgb,
            _ => wgpu::TextureFormat::Rgba8Unorm,
        };
        let wrong = offscreen.target((4, 4), wrong_format, usage, 1);
        assert!(matches!(
            renderer.render(&wrong, &frame),
            Err(RenderError::TargetFormat { .. })
        ));
    }
}
