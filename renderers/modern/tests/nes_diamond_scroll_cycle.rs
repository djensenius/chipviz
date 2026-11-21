//! Draft full-cycle offscreen diagnostic; not a safety assessment.
//!
//! The 256 transitions start at the reset image, advance one accepted NMI
//! each, and include 255 -> 0. With no scene changes these states repeat:
//! reset and wrap both have scroll 0 and foreground $11. Dropped NMIs and
//! re-entry are checked separately, not folded into the cycle. No elapsed time
//! or physical display behavior is inferred from an accepted-NMI count.
//!
//! The test-side oracle constructs the two nametables and $e4 attribute bytes
//! from main.s, uses a manually decoded CHR matrix, and models the ordered
//! palette upload / demo writes as register events. It does not use model
//! pixel or viewport helpers to compute expected pixels. RGB values are the
//! prototype's explicitly approximate sRGB convention, not a measured NES.
//! The operator ROM replaces $2800 tile rows 16..29 and their attributes with
//! warning text. The oracle does not model those writes: demo scroll is at
//! most 127, so the visible second-nametable rows are 0..126, strictly above
//! the modified pixel row 128. This invariant is checked for every sample.
//!
//! The source guard checks selected asset bytes and warning-address anchors,
//! not NMI/program logic, tile placement, attributes, unlisted PPU writes, or
//! the compiled ROM. It is not full ROM drift detection. Re-audit these
//! assumptions whenever the NES assembly changes; the hand-coded oracle is
//! not automatically authoritative.
//!
//! The diagnostic decodes readback bytes as sRGB and uses Rec.709 linear
//! luminance. For each transition it counts pixels rising or falling by at
//! least 0.1, then counts directional transitions whose changed area is at
//! least 25% of this 256x240 image. These arbitrary thresholds are diagnostic
//! choices, not a standards implementation. Every cyclic window contains 60
//! transitions, NOT necessarily one second. A directional transition count is
//! not a flash count; this is neither certification nor a physical/display
//! safety assessment, and it does not analyze red flashes, viewing area,
//! display transfer functions, missed frames, or other platform behavior.

#[path = "../src/scenes/nes_diamond_scroll.rs"]
mod nes_diamond_scroll;
#[path = "../src/scenes/nes_diamond_scroll_gpu.rs"]
mod nes_diamond_scroll_gpu;

use nes_diamond_scroll::{DiamondScroll, Viewport};
use nes_diamond_scroll_gpu::DiamondGpu;

const WIDTH: u32 = 256;
const HEIGHT: u32 = 240;
const PIXELS: usize = (WIDTH * HEIGHT) as usize;

// Independently decoded from the two CHR bitplanes in main.s; all tile
// numbers loaded by LoadNameTable point to this same diamond.
const TILE: [[u8; 8]; 8] = [
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 1, 1, 0, 0, 0],
    [0, 0, 1, 3, 3, 1, 0, 0],
    [0, 1, 3, 3, 3, 3, 1, 0],
    [1, 3, 3, 3, 3, 3, 3, 1],
    [0, 1, 3, 3, 3, 3, 1, 0],
    [0, 0, 1, 3, 3, 1, 0, 0],
    [0, 0, 0, 1, 1, 0, 0, 0],
];
const INITIAL_PALETTE: [u8; 16] = [
    0x0f, 0x11, 0x21, 0x31, 0x0f, 0x05, 0x15, 0x25, 0x0f, 0x09, 0x19, 0x29, 0x0f, 0x0c, 0x1c, 0x2c,
];

fn rgb(index: u8) -> [u8; 3] {
    // Independent literal encoding of the prototype's documented mapping.
    match index {
        0x0f => [0, 0, 0],
        0x11 => [0, 120, 248],
        0x12 => [0, 88, 248],
        0x13 => [104, 68, 252],
        0x14 => [216, 0, 204],
        0x15 => [228, 0, 88],
        0x21 => [60, 188, 252],
        0x31 => [164, 228, 252],
        0x05 => [172, 0, 124],
        0x25 => [248, 88, 152],
        0x09 => [0, 88, 0],
        0x19 => [0, 184, 0],
        0x29 => [88, 216, 84],
        0x0c => [0, 64, 88],
        0x1c => [0, 232, 216],
        0x2c => [56, 248, 248],
        _ => panic!("unexpected palette index {index:02x}"),
    }
}

fn phase_at_exit(frame: u8) -> u8 {
    match frame {
        0..=31 => 0x11,
        32..=63 => 0x12,
        64..=95 => 0x13,
        96..=127 => 0x14,
        128..=159 => 0x15,
        160..=191 => 0x14,
        192..=223 => 0x13,
        224..=255 => 0x12,
    }
}

struct Oracle {
    tiles: [[u8; 960]; 2],
    attributes: [[u8; 64]; 2],
    palette: [u8; 16],
    scroll_table: [u8; 256],
    frame: u8,
    scroll: u8,
    visible: bool,
    queued_palette: Option<[u8; 16]>,
}

impl Oracle {
    fn new() -> Self {
        // NameLoopA writes three 256-byte pages of x & 3; NameLoopB writes
        // 192 bytes of (x >> 1) & 3 after x has wrapped back to zero.
        let mut table = [0u8; 960];
        for (i, tile) in table.iter_mut().enumerate() {
            let x = if i < 768 { i % 256 } else { i - 768 };
            *tile = if i < 768 {
                x as u8 & 3
            } else {
                (x as u8 / 2) & 3
            };
        }
        // LSR of the 8-bit frame register: consecutive pairs 0,0,1,1,...
        // Keep this table separate from the model's arithmetic.
        let mut scroll_table = [0u8; 256];
        for value in 0..128 {
            scroll_table[2 * value] = value as u8;
            scroll_table[2 * value + 1] = value as u8;
        }
        Self {
            tiles: [table; 2],
            attributes: [[0xe4; 64]; 2],
            palette: INITIAL_PALETTE,
            scroll_table,
            frame: 0,
            scroll: 0,
            visible: true,
            queued_palette: None,
        }
    }

    fn leave_demo(&mut self) {
        self.visible = false;
        self.queued_palette = None;
    }

    // SceneExit executes during logic after the PPU writes in this NMI.
    fn queue_reentry(&mut self) {
        assert!(!self.visible && self.queued_palette.is_none());
        let mut palette = INITIAL_PALETTE;
        palette[1] = phase_at_exit(self.frame);
        self.queued_palette = Some(palette);
    }

    fn nmi(&mut self, accepted: bool) {
        if !accepted {
            return;
        }
        self.frame = self.frame.wrapping_add(1);
        // NMI uploads the pending scene-exit palette before NmiDemo.
        if let Some(palette) = self.queued_palette.take() {
            self.palette = palette;
            self.visible = true;
        }
        if self.visible {
            // These are the only Frame values that write $3f01 in NmiDemo.
            if let Some(foreground) = match self.frame {
                0 => Some(0x11),
                32 => Some(0x12),
                64 => Some(0x13),
                96 => Some(0x14),
                128 => Some(0x15),
                160 => Some(0x14),
                192 => Some(0x13),
                224 => Some(0x12),
                _ => None,
            } {
                self.palette[1] = foreground;
            }
            self.scroll = self.scroll_table[usize::from(self.frame)];
        }
    }

    fn pixel_index(&self, x: usize, y: usize) -> Option<u8> {
        assert!(x < WIDTH as usize && y < HEIGHT as usize);
        if !self.visible {
            return None;
        }
        let mut row = y + usize::from(self.scroll);
        let bank = if row >= 240 {
            row -= 240;
            // The operator build's warning writes start at pixel row 128 of
            // $2800, including attribute changes for the same lower rows.
            assert!(row <= 126, "demo reached warning rows");
            1
        } else {
            0
        };
        let tx = x / 8;
        let ty = row / 8;
        let tile = self.tiles[bank][ty * 32 + tx];
        assert!(tile < 4); // all loaded tile numbers use the same CHR
        let entry = TILE[row % 8][x % 8] as usize;
        let attr = self.attributes[bank][(ty / 4) * 8 + tx / 4];
        let shift = (tx % 4 >= 2) as usize * 2 + (ty % 4 >= 2) as usize * 4;
        let palette_number = usize::from((attr >> shift) & 3);
        Some(if entry == 0 {
            self.palette[0]
        } else {
            self.palette[palette_number * 4 + entry]
        })
    }

    fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        // Black while outside the demo is a prototype adaptation, not a PPU
        // emulation of the field/warning or the transition itself.
        let [r, g, b] = self.pixel_index(x, y).map(rgb).unwrap_or([0, 0, 0]);
        [r, g, b, 255]
    }
}

#[test]
fn cpu_full_cycle_matches_indexed_oracle() {
    let mut scene = DiamondScroll::default();
    let mut oracle = Oracle::new();
    let mut highest_second_row = 0;
    for step in 0..=256 {
        assert_eq!(scene.frame(), oracle.frame, "step {step}");
        assert_eq!(scene.scroll_y(), oracle.scroll, "step {step}");
        assert_eq!(scene.foreground(), oracle.palette[1], "step {step}");
        for y in 0..HEIGHT as usize {
            let source_y = y + usize::from(oracle.scroll);
            if source_y >= 240 {
                highest_second_row = highest_second_row.max(source_y - 240);
            }
            for x in 0..WIDTH as usize {
                assert_eq!(
                    scene.pixel(x as u32, y as u32),
                    oracle.pixel_index(x, y),
                    "step {step} ({x},{y})"
                );
            }
        }
        if step < 256 {
            scene.nmi(true);
            oracle.nmi(true);
        }
    }
    assert_eq!(scene.frame(), 0, "256th accepted NMI wraps");
    assert_eq!(highest_second_row, 126);
    assert!(
        highest_second_row < 128,
        "operator warning begins at row 128"
    );
}

// This narrowly scoped source check is not a ca65 parser or ROM execution.
// Ignore comments/case/whitespace; other syntax changes require re-audit.
fn source_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| {
            line.split(';')
                .next()
                .unwrap()
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

fn source_section<'a>(lines: &'a [String], start: &str, end: &str) -> &'a [String] {
    let find_unique = |label: &str| {
        let label = source_lines(label).pop().expect("nonempty source anchor");
        let matches: Vec<_> = lines
            .iter()
            .enumerate()
            .filter_map(|(i, line)| (line == &label).then_some(i))
            .collect();
        assert_eq!(matches.len(), 1, "source anchor {label:?}");
        matches[0]
    };
    let start = find_unique(start);
    let end = find_unique(end);
    assert!(start < end, "source anchors out of order");
    &lines[start + 1..end]
}

fn literal_bytes(lines: &[String]) -> Vec<u8> {
    lines
        .iter()
        .flat_map(|line| {
            line.strip_prefix(".byte")
                .expect("expected .byte directive")
                .split(',')
                .map(|value| {
                    u8::from_str_radix(value.strip_prefix('$').expect("expected hex byte"), 16)
                        .expect("valid byte")
                })
        })
        .collect()
}

#[test]
fn rom_source_guard_for_the_oracle() {
    let source = include_str!("../../../cores/nes/homebrew/src/main.s");
    let lines = source_lines(source);

    // The .repeat and its two planes are checked separately, and the actual
    // source bytes are decoded against the manually expressed tile fixture.
    let chr = source_section(&lines, ".segment \"CHR\"", "blank 14");
    assert_eq!(chr.len(), 4, "unexpected CHR diamond block");
    assert_eq!(chr[0], ".repeat32");
    assert_eq!(chr[3], ".endrepeat");
    let planes = literal_bytes(&chr[1..3]);
    assert_eq!(planes.len(), 16);
    for (y, row) in TILE.iter().enumerate() {
        for (x, &entry) in row.iter().enumerate() {
            let mask = 0x80 >> x;
            let actual = u8::from(planes[y] & mask != 0) + 2 * u8::from(planes[y + 8] & mask != 0);
            assert_eq!(entry, actual, "CHR ({x},{y})");
        }
    }
    let phases = literal_bytes(source_section(&lines, "ForegroundColors:", "Palettes:"));
    assert_eq!(phases.len(), 8);
    for (i, phase) in phases.into_iter().enumerate() {
        for frame in i * 32..(i + 1) * 32 {
            assert_eq!(phase_at_exit(frame as u8), phase);
        }
    }
    let palette = literal_bytes(source_section(&lines, "Palettes:", "UpTab:"));
    assert_eq!(palette.len(), 32);
    assert_eq!(&palette[..16], &INITIAL_PALETTE);

    // Narrow anchors for the operator warning's text and attribute start
    // addresses. They do not prove the rest of the writer's behavior.
    let warning_start = source_section(&lines, "LoadWarningText:", "WarnRow:");
    assert!(
        warning_start
            .iter()
            .take(5)
            .map(String::as_str)
            .eq(["bit$2002", "lda#$2a", "sta$2006", "lda#$00", "sta$2006"])
    );
    let attribute_start = source_section(&lines, "WarnRowNext:", "WarnAttr:");
    assert!(attribute_start.windows(4).any(|w| {
        w.iter()
            .map(String::as_str)
            .eq(["lda#$2b", "sta$2006", "lda#$e0", "sta$2006"])
    }));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Areas {
    rising: usize,
    falling: usize,
}

fn linear_lut() -> [f64; 256] {
    std::array::from_fn(|i| {
        let value = i as f64 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    })
}

fn luminance(pixel: &[u8], lut: &[f64; 256]) -> f64 {
    0.2126 * lut[usize::from(pixel[0])]
        + 0.7152 * lut[usize::from(pixel[1])]
        + 0.0722 * lut[usize::from(pixel[2])]
}

fn directional_areas(before: &[u8], after: &[u8], lut: &[f64; 256]) -> Areas {
    assert_eq!(before.len(), after.len());
    assert!(before.len().is_multiple_of(4));
    let mut areas = Areas::default();
    for (a, b) in before.chunks_exact(4).zip(after.chunks_exact(4)) {
        let delta = luminance(b, lut) - luminance(a, lut);
        if delta >= 0.1 {
            areas.rising += 1;
        } else if delta <= -0.1 {
            areas.falling += 1;
        }
    }
    areas
}

#[derive(Debug, PartialEq, Eq)]
struct WindowMax {
    rise_transitions: usize,
    fall_transitions: usize,
    rise_pixel_steps: usize,
    fall_pixel_steps: usize,
}

fn cyclic_window_max(areas: &[Areas], pixels: usize, window: usize) -> WindowMax {
    assert!(!areas.is_empty() && pixels > 0 && window > 0 && window <= areas.len());
    let mut max = WindowMax {
        rise_transitions: 0,
        fall_transitions: 0,
        rise_pixel_steps: 0,
        fall_pixel_steps: 0,
    };
    for start in 0..areas.len() {
        let mut rise_count = 0;
        let mut fall_count = 0;
        let mut rise_area = 0;
        let mut fall_area = 0;
        for offset in 0..window {
            let area = areas[(start + offset) % areas.len()];
            rise_count += (area.rising * 4 >= pixels) as usize;
            fall_count += (area.falling * 4 >= pixels) as usize;
            rise_area += area.rising;
            fall_area += area.falling;
        }
        max.rise_transitions = max.rise_transitions.max(rise_count);
        max.fall_transitions = max.fall_transitions.max(fall_count);
        max.rise_pixel_steps = max.rise_pixel_steps.max(rise_area);
        max.fall_pixel_steps = max.fall_pixel_steps.max(fall_area);
    }
    max
}

#[test]
fn directional_diagnostic_and_cyclic_windows() {
    let before = [
        0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 100, 100, 100, 255,
    ];
    let after = [
        255, 255, 255, 255, 0, 0, 0, 255, 20, 20, 20, 255, 100, 100, 100, 0,
    ];
    assert_eq!(
        directional_areas(&before, &after, &linear_lut()),
        Areas {
            rising: 1,
            falling: 1
        }
    );
    let edges = [
        Areas {
            rising: 1,
            falling: 0,
        },
        Areas {
            rising: 0,
            falling: 1,
        },
        Areas {
            rising: 1,
            falling: 1,
        },
        Areas {
            rising: 0,
            falling: 0,
        },
    ];
    assert_eq!(
        cyclic_window_max(&edges, 4, 3),
        WindowMax {
            rise_transitions: 2,
            fall_transitions: 2,
            rise_pixel_steps: 2,
            fall_pixel_steps: 2,
        }
    );
    let wraps = [
        Areas {
            rising: 1,
            falling: 0,
        },
        Areas {
            rising: 0,
            falling: 0,
        },
        Areas {
            rising: 0,
            falling: 0,
        },
        Areas {
            rising: 1,
            falling: 0,
        },
    ];
    assert_eq!(cyclic_window_max(&wraps, 4, 2).rise_transitions, 2);
}

fn read_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &DiamondGpu,
    readback: &wgpu::Buffer,
    scene: &DiamondScroll,
) -> Vec<u8> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("diamond full-cycle encoder"),
    });
    renderer.encode(queue, &mut encoder, scene);
    encoder.copy_texture_to_buffer(
        renderer.target().as_image_copy(),
        wgpu::ImageCopyBuffer {
            buffer: readback,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
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

fn check_oracle(bytes: &[u8], scene: &DiamondScroll, oracle: &Oracle, label: &str) {
    assert_eq!(bytes.len(), PIXELS * 4);
    assert_eq!(scene.frame(), oracle.frame, "{label}");
    if oracle.visible {
        assert_eq!(scene.scroll_y(), oracle.scroll, "{label}");
        assert_eq!(scene.foreground(), oracle.palette[1], "{label}");
    }
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let actual = &bytes[(y * WIDTH as usize + x) * 4..][..4];
            let expected = oracle.pixel(x, y);
            assert_eq!(actual[3], 255, "{label} ({x},{y}) alpha");
            for channel in 0..3 {
                assert!(
                    actual[channel].abs_diff(expected[channel]) <= 1,
                    "{label} ({x},{y}) channel={channel}: actual={actual:?} expected={expected:?}"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter and the serialized GPU slot"]
fn full_cycle_gpu_readback_and_directional_diagnostic() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("headless GPU adapter required");
    println!("selected GPU adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("diamond full-cycle device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
        },
        None,
    ))
    .expect("GPU device");
    let renderer = DiamondGpu::new(&device, WIDTH, HEIGHT, wgpu::TextureFormat::Rgba8UnormSrgb)
        .expect("offscreen renderer");
    // Explicit native size: no letterbox pixels affect the diagnostic.
    assert_eq!(
        Viewport::fit(WIDTH, HEIGHT).unwrap().source_pixel(0, 0),
        Some((0, 0))
    );
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("diamond full-cycle readback"),
        size: (PIXELS * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut scene = DiamondScroll::default();
    let mut oracle = Oracle::new();
    // Fixed startup anchors for entry 1 and 3 in each $e4 quadrant.
    assert_eq!(oracle.pixel(2, 2), [0, 120, 248, 255]);
    assert_eq!(oracle.pixel(3, 2), [164, 228, 252, 255]);
    assert_eq!(oracle.pixel(18, 2), [172, 0, 124, 255]);
    assert_eq!(oracle.pixel(3, 18), [88, 216, 84, 255]);
    assert_eq!(oracle.pixel(19, 18), [56, 248, 248, 255]);
    let initial = read_frame(&device, &queue, &renderer, &readback, &scene);
    check_oracle(&initial, &scene, &oracle, "startup");
    scene.nmi(false);
    oracle.nmi(false);
    let busy = read_frame(&device, &queue, &renderer, &readback, &scene);
    check_oracle(&busy, &scene, &oracle, "busy startup");
    assert_eq!(busy, initial, "a rejected NMI does not change the image");

    println!(
        "Diagnostic ONLY: sRGB readback decoded to linear Rec.709 luminance; \
         threshold |delta| >= 0.1; directional area >= 25% of {PIXELS} native pixels. \
         60 accepted-NMI transitions per window, not a time interval. \
         Directional transitions are NOT flash counts, a standards assessment, \
         certification, or physical/display safety evidence."
    );
    let lut = linear_lut();
    let mut previous = initial.clone();
    let mut edges = Vec::with_capacity(256);
    for step in 1..=256 {
        scene.nmi(true);
        oracle.nmi(true);
        let current = read_frame(&device, &queue, &renderer, &readback, &scene);
        check_oracle(&current, &scene, &oracle, &format!("accepted NMI {step}"));
        let area = directional_areas(&previous, &current, &lut);
        println!(
            "edge {}->{}: rise={} ({:.6}), fall={} ({:.6}) of {PIXELS} pixels",
            (step - 1) % 256,
            step % 256,
            area.rising,
            area.rising as f64 / PIXELS as f64,
            area.falling,
            area.falling as f64 / PIXELS as f64,
        );
        edges.push(area);
        previous = current;
    }
    assert_eq!(scene.frame(), 0, "8-bit Frame wraps");
    assert_eq!(previous, initial, "wrap returns exactly to startup bytes");
    let max = cyclic_window_max(&edges, PIXELS, 60);
    println!(
        "Cyclic 60-transition maxima: qualifying rise={}, fall={}; \
         rise pixel-steps={}, fall pixel-steps={} (maxima may occur in \
         different windows; pixel-steps sum areas and can count a pixel repeatedly).",
        max.rise_transitions, max.fall_transitions, max.rise_pixel_steps, max.fall_pixel_steps
    );

    // SceneExit queues the palette selected at its current Frame; the next
    // accepted NMI increments Frame, uploads that palette, then can overwrite
    // foreground on a phase boundary. The prototype draws black while away.
    for (exit_frame, next_frame, next_scroll, next_foreground) in
        [(31, 32, 16, 0x12), (45, 46, 23, 0x12), (255, 0, 0, 0x11)]
    {
        let mut scene = DiamondScroll::default();
        let mut oracle = Oracle::new();
        scene.leave_demo();
        oracle.leave_demo();
        for _ in 0..exit_frame {
            scene.nmi(true);
            oracle.nmi(true);
        }
        scene.queue_reentry();
        oracle.queue_reentry();
        scene.nmi(false);
        oracle.nmi(false);
        let pending = read_frame(&device, &queue, &renderer, &readback, &scene);
        check_oracle(&pending, &scene, &oracle, "reentry pending after busy NMI");
        scene.nmi(true);
        oracle.nmi(true);
        let restored = read_frame(&device, &queue, &renderer, &readback, &scene);
        check_oracle(&restored, &scene, &oracle, "reentry restored");
        assert_eq!(
            (scene.frame(), scene.scroll_y(), scene.foreground()),
            (next_frame, next_scroll, next_foreground)
        );
    }
}
