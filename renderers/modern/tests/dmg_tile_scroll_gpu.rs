//! Offscreen prototype regression; this does not register a selectable scene.
//!
//! Once the serialized heavy slot is available, run from the repository root:
//! `cargo test --manifest-path renderers/modern/Cargo.toml --features dmg-preview --test dmg_tile_scroll_gpu -- --ignored --nocapture`
//! It requires a headless WGPU PRIMARY adapter (e.g. Metal on M1 or Vulkan on
//! Pi) supporting R8Unorm texture binding and RGBA8 UNORM and sRGB render
//! attachments. It prints the actual adapter and fails if none is available.
//! On integration, register this ignored test in a standard GPU-capable CI
//! target; an ordinary `cargo test` does not run it.
//!
//! The cycle diagnostic reports directional relative-luminance changes, not
//! flashes, WCAG compliance, certification, or safety. A cyclic window of
//! 60 transitions is not a time interval; this test has no playback clock.
//! It reads 160x144 offscreen bytes, not a presented or composited recording.
//! Scaling, display colour management, physical Pi behaviour, standard test
//! patterns and independent flash assessment remain unverified. The oracle
//! is manually derived from the native source and must be re-audited if it changes.

#[expect(
    dead_code,
    reason = "this fixture uses only the model API needed for GPU readback"
)]
#[path = "../src/scenes/dmg_tile_scroll.rs"]
mod dmg_tile_scroll;
#[path = "../src/scenes/dmg_tile_scroll_render.rs"]
mod dmg_tile_scroll_render;
// Compile the same preview acquisition/clock seam used by the window path.
#[cfg(feature = "dmg-preview")]
use crate as scenes;
#[cfg(feature = "dmg-preview")]
#[path = "../src/dmg_preview.rs"]
mod dmg_preview;

#[cfg(feature = "dmg-preview")]
use dmg_preview::{DmgPresenter, DmgTarget};
use dmg_tile_scroll::{Display, DmgTileScroll};
use dmg_tile_scroll_render::DmgTileRenderer;
#[cfg(feature = "dmg-preview")]
use std::cell::Cell;
#[cfg(feature = "dmg-preview")]
use std::time::{Duration, Instant};

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
        self.read_texture(&texture, size, encoder)
    }

    #[cfg(feature = "dmg-preview")]
    fn render_preview(
        &self,
        presenter: &mut DmgPresenter,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        timing: (Instant, Instant, Duration),
        expected_wait: Duration,
    ) -> Vec<u8> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("DMG preview offscreen target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let calls = Cell::new(0);
        let waited = Cell::new(Duration::ZERO);
        let (acquired, encoded, present_delay) = timing;
        assert!(
            presenter
                .render(
                    &self.device,
                    &self.queue,
                    size,
                    DmgTarget {
                        acquire: || Ok::<_, ()>(&texture),
                        view: |texture: &&wgpu::Texture| {
                            texture.create_view(&wgpu::TextureViewDescriptor::default())
                        },
                        present: |_| {},
                    },
                    || {
                        let call = calls.get();
                        calls.set(call + 1);
                        match call {
                            0 => acquired,
                            1 | 2 => encoded + waited.get(),
                            3 => encoded + waited.get() + present_delay,
                            _ => panic!("unexpected clock sample"),
                        }
                    },
                    |duration| waited.set(duration),
                )
                .unwrap()
        );
        assert_eq!(calls.get(), 4);
        assert_eq!(waited.get(), expected_wait);
        let encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("DMG preview readback encoder"),
            });
        self.read_texture(&texture, size, encoder)
    }

    fn read_texture(
        &self,
        texture: &wgpu::Texture,
        size: (u32, u32),
        mut encoder: wgpu::CommandEncoder,
    ) -> Vec<u8> {
        let (width, height) = size;
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let row_bytes = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DMG test readback"),
            size: u64::from(row_bytes) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
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

// Independently derive the displayed native registers from NativeStep and
// ApplyDisplay, using arithmetic and explicit palette tables rather than the
// model's bit rotation, BGP calculation or shade buffer.
fn cycle_example(step: usize) -> Example {
    let counter = (step % 256) as u8;
    let (bgp, shades) = [
        (0xe4, [255, 170, 85, 0]),
        (0xec, [255, 0, 85, 0]),
        (0xf4, [255, 170, 0, 0]),
        (0xfc, [255, 0, 0, 0]),
    ][usize::from(counter / 16 % 4)];
    Example {
        ticks: step + 1,
        scx: counter / 2 + (counter % 2) * 128,
        scy: counter,
        bgp,
        shades,
    }
}

fn linear_srgb_lut() -> [f64; 256] {
    std::array::from_fn(|code| {
        let encoded = code as f64 / 255.0;
        if encoded <= 0.04045 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        }
    })
}

fn pixel_luminance(pixel: &[u8], linear: &[f64; 256]) -> f64 {
    0.2126 * linear[usize::from(pixel[0])]
        + 0.7152 * linear[usize::from(pixel[1])]
        + 0.0722 * linear[usize::from(pixel[2])]
}

/// Count changed pixel positions from actual readback, not the native oracle.
/// A direction qualifies when |delta relative luminance| >= 0.1 and the
/// darker value is < 0.8. Scrolling and palette changes both contribute.
fn directional_areas(before: &[u8], after: &[u8], linear: &[f64; 256]) -> (usize, usize) {
    assert_eq!(before.len(), after.len());
    assert!(before.len().is_multiple_of(4));
    let mut brighter = 0;
    let mut darker = 0;
    for (a, b) in before.chunks_exact(4).zip(after.chunks_exact(4)) {
        let old = pixel_luminance(a, linear);
        let new = pixel_luminance(b, linear);
        if old.min(new) < 0.8 {
            if new - old >= 0.1 {
                brighter += 1;
            } else if old - new >= 0.1 {
                darker += 1;
            }
        }
    }
    (brighter, darker)
}

fn max_large_transitions(transitions: &[(usize, usize)], pixels: usize) -> (usize, usize, usize) {
    assert!(!transitions.is_empty());
    let mut max_bright = 0;
    let mut max_dark = 0;
    let mut max_either = 0;
    for start in 0..transitions.len() {
        let mut bright = 0;
        let mut dark = 0;
        let mut either = 0;
        for offset in 0..60 {
            let (brighter, darker) = transitions[(start + offset) % transitions.len()];
            let bright_large = brighter * 4 > pixels;
            let dark_large = darker * 4 > pixels;
            bright += usize::from(bright_large);
            dark += usize::from(dark_large);
            either += usize::from(bright_large || dark_large);
        }
        max_bright = max_bright.max(bright);
        max_dark = max_dark.max(dark);
        max_either = max_either.max(either);
    }
    (max_bright, max_dark, max_either)
}

#[test]
fn directional_diagnostic_counts_thresholds_and_cyclic_windows() {
    let linear = linear_srgb_lut();
    let before = [
        0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 85, 85, 85, 255, 240, 240, 240, 255,
    ];
    let after = [
        255, 255, 255, 255, 0, 0, 0, 255, 85, 85, 85, 255, 170, 170, 170, 255, 255, 255, 255, 255,
    ];
    // 0 -> 85 is only ~0.091 linear luminance; 85 -> 170 exceeds 0.1.
    // 240 -> 255 exceeds 0.1, but its darker luminance is above 0.8.
    assert_eq!(directional_areas(&before, &after, &linear), (2, 1));
    let mut transitions = vec![(0, 0); 256];
    transitions[250] = (26, 0);
    transitions[1] = (0, 26);
    transitions[2] = (25, 25); // exactly 25% is not greater than 25%
    assert_eq!(max_large_transitions(&transitions, 100), (1, 1, 2));
}

#[cfg(feature = "dmg-preview")]
#[test]
#[ignore = "requires a GPU adapter"]
fn preview_acquisition_clock_and_offscreen_renderer_match_native_oracle() {
    let gpu = Offscreen::new();
    for format in [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Rgba8Unorm,
    ] {
        let mut presenter = DmgPresenter::new(&gpu.device, format);
        let start = Instant::now();
        // The real orchestration must skip zero-sized targets before
        // acquisition and must not sample time on acquisition failure.
        assert_eq!(
            presenter.render(
                &gpu.device,
                &gpu.queue,
                (0, 144),
                DmgTarget {
                    acquire: || -> Result<(), &str> { panic!("zero-size acquisition") },
                    view: |_: &()| panic!("zero-size view"),
                    present: |_| panic!("zero-size present"),
                },
                || panic!("zero-size clock"),
                |_| panic!("zero-size wait"),
            ),
            Ok(false)
        );
        assert_eq!(
            presenter.render(
                &gpu.device,
                &gpu.queue,
                (160, 144),
                DmgTarget {
                    acquire: || Err::<(), _>("lost"),
                    view: |_: &()| panic!("failed acquisition view"),
                    present: |_| panic!("failed acquisition present"),
                },
                || panic!("failed acquisition sampled time"),
                |_| panic!("failed acquisition wait"),
            ),
            Err("lost")
        );
        for (acquired, encoded, present_delay, wait, step, size, scale) in [
            (
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::ZERO,
                Duration::ZERO,
                0,
                (160, 144),
                1,
            ),
            (
                Duration::from_nanos(116_742_706),
                Duration::from_nanos(116_742_706),
                Duration::ZERO,
                Duration::ZERO,
                0,
                (320, 288),
                2,
            ),
            (
                Duration::from_nanos(116_742_707),
                Duration::from_nanos(116_742_707),
                Duration::ZERO,
                Duration::ZERO,
                1,
                (160, 144),
                1,
            ),
            (
                Duration::from_nanos(150_228_118),
                Duration::from_nanos(150_228_118),
                Duration::from_millis(8),
                Duration::ZERO,
                2,
                (160, 144),
                1,
            ),
            (
                Duration::from_nanos(158_228_119),
                Duration::from_nanos(158_228_119),
                Duration::ZERO,
                Duration::from_nanos(7_999_999),
                3,
                (160, 144),
                1,
            ),
            (
                Duration::from_secs(10),
                Duration::from_secs(10),
                Duration::ZERO,
                Duration::ZERO,
                4,
                (320, 288),
                2,
            ),
            (
                Duration::from_secs(10),
                Duration::from_secs(10),
                Duration::ZERO,
                Duration::ZERO,
                4,
                (160, 144),
                1,
            ),
        ] {
            let rgba = gpu.render_preview(
                &mut presenter,
                format,
                size,
                (start + acquired, start + encoded, present_delay),
                wait,
            );
            let example = cycle_example(step);
            for y in 0..size.1 {
                for x in 0..size.0 {
                    let expected = expected_pixel(example, x / scale, y / scale);
                    let offset = ((y * size.0 + x) * 4) as usize;
                    let pixel = &rgba[offset..offset + 4];
                    let tolerance = if expected == 0 || expected == 255 {
                        0
                    } else {
                        1
                    };
                    for &channel in &pixel[..3] {
                        assert!(
                            channel.abs_diff(expected) <= tolerance,
                            "{format:?} step={step} size={size:?} ({x},{y}) expected {expected}, got {pixel:?}"
                        );
                    }
                    assert_eq!(pixel[3], 255);
                }
            }
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn dmg_offscreen_cycle_matches_native_oracle_and_reports_directional_changes() {
    const PIXELS: usize = 160 * 144;
    const CYCLE: usize = 256;
    let gpu = Offscreen::new();
    let linear = linear_srgb_lut();
    eprintln!(
        "DMG OFFSCREEN DIAGNOSTIC ONLY: 160x144 readback; \
         not presented/composited output, a standardized flash test, certification, or a safety assessment. \
         Scaling, colour management, physical Pi and standard patterns are unverified. \
         A 60-transition window is not a time interval; there is no playback clock."
    );
    eprintln!(
        "DMG metric: sRGB-decoded Rec.709 relative luminance; per-pixel |delta Y| >= 0.1 \
         with darker Y < 0.8, counted separately by direction across {PIXELS} pixels."
    );
    for format in [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Rgba8Unorm,
    ] {
        let renderer = DmgTileRenderer::new(&gpu.device, format);
        let mut scene = DmgTileScroll::default();
        let mut previous: Option<Vec<u8>> = None;
        let mut transitions = Vec::with_capacity(CYCLE);
        // tick 1 displays counter 0; tick 257 displays wrapped counter 0.
        for step in 0..=CYCLE {
            scene.tick();
            let example = cycle_example(step);
            assert_eq!(
                scene.shown(),
                Display {
                    scx: example.scx,
                    scy: example.scy,
                    bgp: example.bgp,
                },
                "{format:?} displayed counter at cycle step {step}"
            );
            let rgba = gpu.render(&renderer, format, (160, 144), scene.shown(), false);
            assert_eq!(rgba.len(), PIXELS * 4);
            for (index, pixel) in rgba.chunks_exact(4).enumerate() {
                let expected = expected_pixel(example, (index % 160) as u32, (index / 160) as u32);
                let tolerance = if expected == 0 || expected == 255 {
                    0
                } else {
                    1
                };
                for &channel in &pixel[..3] {
                    assert!(
                        channel.abs_diff(expected) <= tolerance,
                        "{format:?} step={step} pixel={index} expected {expected}, got {pixel:?}"
                    );
                }
                assert_eq!(pixel[3], 255, "{format:?} step={step} pixel={index}");
            }
            if let Some(before) = previous.as_deref() {
                let (brighter, darker) = directional_areas(before, &rgba, &linear);
                transitions.push((brighter, darker));
                eprintln!(
                    "DMG DIAGNOSTIC {format:?} counter {}->{}: brighter={brighter}/{PIXELS} darker={darker}/{PIXELS} pixels",
                    step - 1,
                    step % CYCLE
                );
            }
            previous = Some(rgba);
        }
        assert_eq!(transitions.len(), CYCLE);
        let (max_bright, max_dark, max_either) = max_large_transitions(&transitions, PIXELS);
        eprintln!(
            "DMG DIAGNOSTIC {format:?}: maximum transitions with >25% directional area \
             in any cyclic 60-transition window: brighter={max_bright}, darker={max_dark}, \
             either={max_either}; these are transition counts, not flash counts or a time interval"
        );
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
