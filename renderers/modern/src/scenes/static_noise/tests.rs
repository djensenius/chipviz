use super::*;

const WIDE: f32 = 16.0 / 9.0;

fn at(frame: u16) -> ControlFrame {
    ControlFrame::procedural(u64::from(frame))
}

/// Frame `local` of section `index`.
fn frame_in(index: u16, local: u16) -> ControlFrame {
    at(index * SECTION_FRAMES + local)
}

fn cells(profile: &RenderProfile, frame: ControlFrame, source: &dyn TextSource) -> Vec<Cell> {
    let section = section(frame);
    let grid = Grid::new(profile, WIDE);
    let noise = Noise::new(frame);
    target(section, grid, source)
        .into_iter()
        .enumerate()
        .map(|(index, target)| cell(index, target, None, section, noise))
        .collect()
}

fn build_frame(profile: &RenderProfile, frame: ControlFrame, aspect: f32) -> Vec<Instance> {
    let mut instances = Vec::new();
    build(
        profile,
        frame,
        0.0,
        aspect,
        &SceneState::default(),
        &mut instances,
    );
    instances
}

struct Custom(&'static str);

impl TextSource for Custom {
    fn len(&self) -> usize {
        1
    }

    fn phrase(&self, _index: usize) -> &str {
        self.0
    }
}

#[test]
fn sections_rotate_content_style_camera_and_background() {
    let expected = [
        (Content::Text(0), Style::Glyphs, Move::Flat, false),
        (Content::Constant(0), Style::Glyphs, Move::ZoomOut, false),
        (Content::Knot, Style::Glyphs, Move::Orbit, false),
        (Content::Noise, Style::Glyphs, Move::Flat, false),
        (Content::Text(1), Style::Pixels, Move::ZoomOut, true),
        (Content::Constant(1), Style::Pixels, Move::Orbit, false),
    ];
    for (index, (content, style, camera, white)) in expected.into_iter().enumerate() {
        let section = section(frame_in(index as u16, 77));
        assert_eq!(section.index, index as u16);
        assert_eq!(section.local, 77);
        assert_eq!(
            (
                section.content,
                section.style,
                section.camera,
                section.white
            ),
            (content, style, camera, white),
            "section {index}"
        );
    }
    // The beat fields and the drop flag do not move the schedule.
    let shifted = ControlFrame {
        beat_count: 999,
        beat_phase: 3,
        flags: 0x04,
        ..at(700)
    };
    assert_eq!(section(shifted), section(at(700)));
}

#[test]
fn grid_and_instances_stay_within_the_instance_budget() {
    for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
        for aspect in [0.001, 0.3, 1.0, WIDE, 2.4, 4.0, 50.0] {
            let grid = Grid::new(&profile, aspect);
            assert!(grid.rows > 10 && grid.cols > 10, "{aspect}: {grid:?}");
            assert!(RESERVED + grid.len() <= profile.instance_count);
            // White background, pixel strips and a full grid at once.
            let frame = ControlFrame {
                energy: 255,
                ..frame_in(4, 10)
            };
            assert!(section(frame).white && section(frame).style == Style::Pixels);
            assert!(build_frame(&profile, frame, aspect).len() <= profile.instance_count);
        }
        // Builders append: existing instances shrink what Static may add.
        let mut instances =
            vec![build_frame(&profile, at(40), WIDE)[0]; profile.instance_count - 5];
        build(
            &profile,
            at(40),
            0.0,
            WIDE,
            &SceneState::default(),
            &mut instances,
        );
        assert_eq!(instances.len(), profile.instance_count);
    }
    // The m1 budget draws smaller glyphs than the Pi 5 budget.
    assert!(
        Grid::new(&RenderProfile::m1(), WIDE).rows > Grid::new(&RenderProfile::pi5(), WIDE).rows
    );
}

#[test]
fn blank_chance_follows_energy_within_bounds() {
    for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
        for energy in [0u8, 64, 128, 255] {
            let expected = MAX_BLANK - (MAX_BLANK - MIN_BLANK) * f32::from(energy) / 255.0;
            for local in [0, 50, 333] {
                // Section 3 is pure glyph noise.
                let frame = ControlFrame {
                    energy,
                    ..frame_in(3, local)
                };
                let cells = cells(&profile, frame, &Embedded);
                assert!(
                    cells
                        .iter()
                        .all(|cell| matches!(cell.role, Role::Blank | Role::Noise))
                );
                let blank = cells.iter().filter(|cell| cell.role == Role::Blank).count();
                let share = blank as f32 / cells.len() as f32;
                assert!((share - expected).abs() < 0.03, "energy {energy}: {share}");
                assert!((MIN_BLANK - 0.03..=MAX_BLANK + 0.03).contains(&share));
            }
        }
    }
}

#[test]
fn energy_speeds_up_the_noise() {
    let profile = RenderProfile::pi5();
    let changed = |energy: u8| {
        let frame = |local: u16| ControlFrame {
            energy,
            beat_count: 0,
            ..frame_in(3, local)
        };
        let before = cells(&profile, frame(100), &Embedded);
        let after = cells(&profile, frame(101), &Embedded);
        before.iter().zip(&after).filter(|(a, b)| a != b).count()
    };
    assert!(
        changed(255) > 2 * changed(0),
        "{} vs {}",
        changed(255),
        changed(0)
    );
    // Silence still changes some cells every frame.
    assert!(changed(0) > 0);
}

#[test]
fn morph_lands_on_the_exact_text_at_the_hold() {
    let profile = RenderProfile::pi5();
    let grid = Grid::new(&profile, WIDE);
    let phrase: Vec<char> = text::PHRASES[0].1.chars().collect();
    let frame = frame_in(0, HOLD_START);
    let targets = target(section(frame), grid, &Embedded);

    // Drawn large: each letter's 5x8 block holds that letter where its glyph
    // is lit and nothing elsewhere.
    let width = phrase.len() * 6 - 1;
    let (left, top) = ((grid.cols - width) / 2, (grid.rows - 8) / 2);
    for (position, &character) in phrase.iter().enumerate() {
        let bits = glyph(character).unwrap();
        for bit in 0..40 {
            let index = (top + bit / 5) * grid.cols + left + position * 6 + bit % 5;
            let lit = bits >> bit & 1 == 1;
            assert_eq!(
                targets[index],
                lit.then_some(character),
                "{character} bit {bit}"
            );
        }
    }
    let lit = targets.iter().filter(|target| target.is_some()).count();
    let expected: u32 = phrase.iter().map(|&c| glyph(c).unwrap().count_ones()).sum();
    assert_eq!(lit, expected as usize);
    // The denser m1 grid wraps it onto two lines, each font pixel 3x3 cells.
    let m1 = Grid::new(&RenderProfile::m1(), WIDE);
    let large = target(section(frame), m1, &Embedded);
    assert_eq!(large.iter().flatten().count(), 9 * lit);

    // Every cell has landed: targets show their glyph, the rest are blank.
    let cells = cells(&profile, frame, &Embedded);
    for (cell, target) in cells.iter().zip(&targets) {
        match target {
            Some(character) => {
                assert_eq!(
                    (cell.role, cell.glyph),
                    (Role::Target, glyph(*character).unwrap())
                )
            }
            None => assert_eq!(cell.role, Role::Blank),
        }
    }
    // The flat camera draws exactly those glyphs, in order.
    let drawn: Vec<_> = build_frame(&profile, frame, WIDE)
        .iter()
        .map(|instance| instance.pattern)
        .collect();
    let wanted: Vec<_> = targets
        .iter()
        .flatten()
        .map(|&character| glyph_pattern(glyph(character).unwrap()))
        .collect();
    assert_eq!(drawn, wanted);
}

#[test]
fn long_text_falls_back_to_one_cell_per_character() {
    let profile = RenderProfile::pi5();
    let grid = Grid::new(&profile, WIDE);
    let phrase = "ΓΕΙΑ ΣΟΥ ΚΟΣΜΕ ПРИВЕТ МИР HOLA MUNDO OLÁ MUNDO BRUIT BLANC XIN CHÀO";
    let targets = target(section(frame_in(0, HOLD_START)), grid, &Custom(phrase));
    let read: String = targets
        .chunks(grid.cols)
        .map(|row| {
            let line: String = row.iter().map(|cell| cell.unwrap_or(' ')).collect();
            line.trim().to_string()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(read, phrase);
}

#[test]
fn transitions_cycle_glyphs_before_landing_and_after_leaving() {
    let profile = RenderProfile::pi5();
    let roles = |local: u16| {
        let cells = cells(&profile, frame_in(0, local), &Embedded);
        let count = |role| cells.iter().filter(|cell| cell.role == role).count();
        (count(Role::Target), count(Role::Cycling))
    };
    let (target, cycling) = roles(RESOLVE_START - 1);
    assert_eq!(target, 0);
    assert!(cycling > 0);
    let (target, cycling) = roles(RESOLVE_START + SPREAD / 2);
    assert!(
        target > 0 && cycling > 0,
        "mid-transition {target} {cycling}"
    );
    assert_eq!(roles(DISSOLVE_START - 1).1, 0);
    assert_eq!(roles(DISSOLVE_START + SPREAD).0, 0);
    // Cycling glyphs change from frame to frame.
    let glyphs = |local| cells(&profile, frame_in(0, local), &Embedded);
    let (a, b) = (glyphs(RESOLVE_START - 4), glyphs(RESOLVE_START - 2));
    let differs = a
        .iter()
        .zip(&b)
        .any(|(a, b)| a.role == Role::Cycling && b.role == Role::Cycling && a.glyph != b.glyph);
    assert!(differs);
}

#[test]
fn constants_and_knot_resolve_into_numbers() {
    let profile = RenderProfile::pi5();
    let grid = Grid::new(&profile, WIDE);
    let constant = target(section(frame_in(1, HOLD_START)), grid, &Embedded);
    let shown: std::collections::BTreeSet<char> = constant.iter().flatten().copied().collect();
    let expected: std::collections::BTreeSet<char> = "π=3.14159265358979".chars().collect();
    assert_eq!(shown, expected);

    let early = target(section(frame_in(2, HOLD_START)), grid, &Embedded);
    let late = target(section(frame_in(2, DISSOLVE_START - 1)), grid, &Embedded);
    let digits = |cells: &[Option<char>]| {
        cells[..grid.len() - grid.cols]
            .iter()
            .filter(|cell| cell.is_some_and(|c| c.is_ascii_digit()))
            .count()
    };
    assert!(digits(&late) > digits(&early) && digits(&early) > 0);
    let label: String = late[grid.len() - grid.cols..]
        .iter()
        .map(|cell| cell.unwrap_or(' '))
        .collect();
    assert!(label.trim().starts_with('X') && label.contains(" Y") && label.contains(" Z"));
}

#[test]
fn text_source_seam_accepts_any_text() {
    let profile = RenderProfile::pi5();
    let grid = Grid::new(&profile, WIDE);
    let targets = target(section(frame_in(0, HOLD_START)), grid, &Custom("Á 🙂"));
    let emoji = targets.iter().filter(|cell| **cell == Some('🙂')).count();
    // The font has no emoji: a stable glyph-like bitmap stands in for it.
    let bits = glyph_or_noise('🙂');
    assert_eq!(emoji, 9 * bits.count_ones() as usize, "drawn at scale 3");
    assert!(bits != 0 && bits == glyph_or_noise('🙂'));
    let empty = Custom("");
    assert!(
        target(section(at(HOLD_START)), grid, &empty)
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn pixel_sections_draw_per_pixel_noise_that_thins_for_content() {
    let profile = RenderProfile::m1();
    let strips = |local: u16| -> Vec<Instance> {
        build_frame(&profile, frame_in(12, local), WIDE)
            .into_iter()
            .filter(|instance| instance.pattern[0] < -0.5)
            .collect()
    };
    let noisy = strips(10);
    assert_eq!(noisy.len(), NOISE_STRIPS);
    let held = strips(HOLD_START + 20);
    let blank = |instance: &Instance| {
        -instance.pattern[0] - 1.0 - 2.0 * f32::from(instance.pattern[0] < -2.999)
    };
    assert!(blank(&held[0]) > blank(&noisy[0]) + 0.1);
    // Strips tile the plane edge to edge.
    for pair in noisy.windows(2) {
        let edge =
            |instance: &Instance, side: f32| instance.center[0] + side * instance.size[0] * 0.5;
        assert!((edge(&pair[0], 1.0) - edge(&pair[1], -1.0)).abs() < 1e-5);
    }
}

/// A white section complements every colour, including the hues rainbow
/// pixel noise generates in the shader.
#[test]
fn white_rainbow_pixel_noise_is_inverted() {
    let profile = RenderProfile::m1();
    // Section 4 is white pixel noise and frame 2400 has palette 3; section 13
    // (frame 6760) is black pixel noise with palette 3.
    let strip = |frame: u16| {
        let frame = at(frame);
        assert_eq!(frame.palette % 4, 3);
        let instances = build_frame(&profile, frame, WIDE);
        *instances
            .iter()
            .find(|instance| instance.pattern[0] < -0.5)
            .expect("noise strip")
    };
    let (white, black) = (strip(2400), strip(6760));
    assert!(section(at(2400)).white && !section(at(6760)).white);
    let decode = |instance: &Instance| {
        let flags = -instance.pattern[0] - 1.0;
        (flags >= 4.0, flags - 4.0 * f32::from(flags >= 4.0) >= 2.0)
    };
    assert_eq!(decode(&white), (true, true));
    assert_eq!(decode(&black), (false, true));
    // Same pixel and pattern without the flag: the hue is complemented and
    // the level unchanged.
    let plain = [white.pattern[0] + 4.0, white.pattern[1]];
    let mut lit = 0;
    for (x, y) in (0..64).flat_map(|x| (0..64).map(move |y| (x, y))) {
        let flipped = gpu_reference::pixel(x, y, white.color, white.pattern);
        let normal = gpu_reference::pixel(x, y, white.color, plain);
        assert_eq!(flipped.is_some(), normal.is_some());
        if let (Some(flipped), Some(normal)) = (flipped, normal) {
            lit += 1;
            for channel in 0..3 {
                assert!((normal[channel] + flipped[channel] - 1.0).abs() < 1e-6);
            }
            assert_eq!(normal[3], flipped[3]);
        }
    }
    assert!(lit > 0);
}

/// Perspective moves the projected centre off the midpoint of the projected
/// edges, so strips placed at their centres would overlap and blend twice.
#[test]
fn pixel_strips_tile_exactly_while_the_camera_moves() {
    let profile = RenderProfile::m1();
    // Zoom-out (section 4, 13) and orbit (section 5, 14) pixel sections,
    // including frame 2702 (section 5) that overlapped ~1.2 px at 1920 wide.
    let frames = [2102, 2300, 2500, 2702, 2900, 3000, 6900, 7400];
    let mut checked = 0;
    for frame in frames.map(at) {
        let section = section(frame);
        assert_eq!(section.style, Style::Pixels, "frame {}", frame.frame);
        assert_ne!(camera_pose(section), (0.0, 0.0, FOCAL), "camera at rest");
        for aspect in [WIDE, 1.0, 0.5] {
            let strips: Vec<Instance> = build_frame(&profile, frame, aspect)
                .into_iter()
                .filter(|instance| instance.pattern[0] < -0.5)
                .collect();
            assert!(strips.len() > 1, "frame {} aspect {aspect}", frame.frame);
            for pair in strips.windows(2) {
                let edge = |instance: &Instance, side: f32| {
                    instance.center[0] + side * instance.size[0] * 0.5
                };
                // Pixels at 1920 wide (NDC spans 2).
                let gap = (edge(&pair[0], 1.0) - edge(&pair[1], -1.0)).abs() * 960.0;
                assert!(
                    gap < 1e-3,
                    "frame {} aspect {aspect}: {gap} px",
                    frame.frame
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 100);
}

#[test]
fn noise_pattern_decodes_like_the_shader() {
    for (energy, beat, rainbow, inverted, blank) in [
        (0, 0, false, false, 0.2),
        (255, 7, true, false, 0.985),
        (90, 5, false, true, 0.0),
        (255, 7, true, true, 0.985),
        (40, 2, true, true, 0.0),
    ] {
        let noise = Noise {
            beat,
            ..Noise::new(ControlFrame {
                energy,
                ..at(65_535)
            })
        };
        let [mode, packed] = noise_pattern(blank, rainbow, inverted, noise);
        // Mirror of `pixel_noise` in shader.wgsl.
        let flags = -mode - 1.0;
        let is_inverted = flags >= 4.0;
        let code = flags - if is_inverted { 4.0 } else { 0.0 };
        let is_rainbow = code >= 2.0;
        let decoded_blank = code - if is_rainbow { 2.0 } else { 0.0 };
        let packed = (packed + 0.5) as u32;
        assert_eq!(is_inverted, inverted);
        assert_eq!(is_rainbow, rainbow);
        assert!((decoded_blank - blank).abs() < 1e-6);
        assert_eq!((packed & 15) + 1, noise.longest);
        assert_eq!(packed >> 4, 65_535 | beat << 16);
    }
    let bits = glyph('Ж').unwrap();
    let [low, high] = glyph_pattern(bits);
    assert_eq!(u64::from(low as u32) | u64::from(high as u32) << 20, bits);
}

#[test]
fn camera_moves_stay_finite_and_orbit_behind_the_plane() {
    let profile = RenderProfile::pi5();
    let mut mirrored = 0;
    for index in 0..6 {
        for local in (0..SECTION_FRAMES).step_by(23) {
            for aspect in [0.3, 1.0, WIDE, 4.0] {
                let frame = frame_in(index, local);
                let (yaw, pitch, distance) = camera_pose(section(frame));
                assert!(yaw.is_finite() && pitch.is_finite() && distance >= FOCAL);
                for instance in build_frame(&profile, frame, aspect) {
                    let values = instance
                        .center
                        .iter()
                        .chain(&instance.size)
                        .chain(&instance.color);
                    for value in values {
                        assert!(value.is_finite() && value.abs() < 1e4, "{instance:?}");
                    }
                    mirrored += usize::from(instance.size[0] < 0.0);
                }
            }
        }
    }
    assert!(mirrored > 0, "the orbit never showed the back of the plane");
    // At rest the camera is exactly head on, so the plane fills the height.
    for local in [0, MOVE_START, MOVE_END, SECTION_FRAMES - 1] {
        for index in 0..3 {
            let pose = camera_pose(section(frame_in(index, local)));
            assert_eq!(pose, (0.0, 0.0, FOCAL), "section {index} frame {local}");
        }
    }
    let view = View::new((0.0, 0.0, FOCAL), WIDE);
    let [top, bottom] = [1.0, -1.0].map(|y| view.project([0.0, y, 0.0]).unwrap()[1]);
    assert!((top - 1.0).abs() < 1e-5 && (bottom + 1.0).abs() < 1e-5);
}

#[test]
fn instances_are_deterministic() {
    for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
        for frame in [0, 200, 300, 900, 1400, 2300, 2700, 6500] {
            let first = build_frame(&profile, at(frame), WIDE);
            let second = build_frame(&profile, at(frame), WIDE);
            assert!(!first.is_empty());
            assert_eq!(
                bytemuck::cast_slice::<Instance, u8>(&first),
                bytemuck::cast_slice::<Instance, u8>(&second)
            );
        }
    }
}

/// FNV-1a over Static's instances for a spread of frames and both profiles.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn fingerprint() -> u64 {
    let mut fingerprint: u64 = 0xcbf2_9ce4_8422_2325;
    for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
        for frame in [0, 200, 300, 900, 1400, 2300, 2700, 6500] {
            for byte in
                bytemuck::cast_slice::<Instance, u8>(&build_frame(&profile, at(frame), WIDE))
            {
                fingerprint ^= u64::from(*byte);
                fingerprint = fingerprint.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    fingerprint
}

// Gated like the registry goldens: platform libm sin/cos may differ in the
// last ulp elsewhere.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn instances_match_recorded_golden() {
    assert_eq!(fingerprint(), 0xe30c_b1b8_9d4f_14dd);
}

/// CPU stand-in for the GPU pass at `RASTER_W` x `RASTER_H`, like Lines'
/// `rasterise`: point-samples each quad at pixel centres (glyph bitmaps as
/// `glyph_lit` decodes them, noise strips through `gpu_reference::pixel`),
/// alpha-blending its linear colour over black in draw order. Returns each
/// pixel's relative luminance (Rec. 709 weights on linear light, as WCAG
/// uses). Static never rotates its quads.
const RASTER_W: usize = 320;
const RASTER_H: usize = 180;

fn rasterise(instances: &[Instance]) -> Vec<f32> {
    let mut rgb = vec![[0.0f32; 3]; RASTER_W * RASTER_H];
    for instance in instances {
        let [cx, cy, _] = instance.center;
        let [w, h] = instance.size;
        let to_pixel = |ndc: f32, size: usize| ((ndc + 1.0) * 0.5 * size as f32).floor();
        let x_range = to_pixel(cx - w.abs() * 0.5, RASTER_W).max(0.0) as usize
            ..(to_pixel(cx + w.abs() * 0.5, RASTER_W) + 1.0).clamp(0.0, RASTER_W as f32) as usize;
        // Rows count down from the top, as in the shader.
        let y_range = to_pixel(-cy - h.abs() * 0.5, RASTER_H).max(0.0) as usize
            ..(to_pixel(-cy + h.abs() * 0.5, RASTER_H) + 1.0).clamp(0.0, RASTER_H as f32) as usize;
        let [low, high] = instance.pattern;
        let bits = (low > 0.0 || high > 0.0).then_some(low as u64 | (high as u64) << 20);
        for y in y_range {
            let ny = 1.0 - (y as f32 + 0.5) / RASTER_H as f32 * 2.0;
            for x in x_range.clone() {
                let nx = (x as f32 + 0.5) / RASTER_W as f32 * 2.0 - 1.0;
                let local = [(nx - cx) / w, (ny - cy) / h];
                if local[0].abs() > 0.5 || local[1].abs() > 0.5 {
                    continue;
                }
                let color = if low < -0.5 {
                    match gpu_reference::pixel(x as u32, y as u32, instance.color, instance.pattern)
                    {
                        Some(color) => color,
                        None => continue,
                    }
                } else if let Some(bits) = bits {
                    let column = ((local[0] + 0.5) * 5.0).floor().clamp(0.0, 4.0) as u64;
                    let row = ((0.5 - local[1]) * 8.0).floor().clamp(0.0, 7.0) as u64;
                    if bits >> (row * 5 + column) & 1 == 0 {
                        continue;
                    }
                    instance.color
                } else {
                    instance.color
                };
                let pixel = &mut rgb[y * RASTER_W + x];
                for (value, source) in pixel.iter_mut().zip(color) {
                    *value = source * color[3] + *value * (1.0 - color[3]);
                }
            }
        }
    }
    rgb.iter()
        .map(|[r, g, b]| 0.2126 * r + 0.7152 * g + 0.0722 * b)
        .collect()
}

/// Mean luminance of each tile of a `TILES_X` x `TILES_Y` grid: large areas
/// (each about 6 % of the screen, under WCAG's 25 % of the central 10
/// degrees once smaller tiles are averaged in) whose joint brightening or
/// darkening would read as a flash, rather than single glyph pixels, which
/// the noise flips every frame by design.
const TILES_X: usize = 4;
const TILES_Y: usize = 4;

fn tiles(image: &[f32]) -> [f32; TILES_X * TILES_Y] {
    let mut sums = [0.0f32; TILES_X * TILES_Y];
    for (index, value) in image.iter().enumerate() {
        let (x, y) = (index % RASTER_W, index / RASTER_W);
        sums[y * TILES_Y / RASTER_H * TILES_X + x * TILES_X / RASTER_W] += value;
    }
    let area = (RASTER_W * RASTER_H / (TILES_X * TILES_Y)) as f32;
    sums.map(|sum| sum / area)
}

/// Per tile, the luminance series of `frames` rendered through a state
/// stepped on every frame, and whether each frame starts a section.
fn tile_series(frames: &[ControlFrame]) -> (Vec<[f32; TILES_X * TILES_Y]>, Vec<bool>) {
    let profile = RenderProfile::pi5();
    let mut state = SceneState::default();
    let mut series = Vec::new();
    let mut starts = Vec::new();
    let mut last_index = None;
    for frame in frames {
        state.observe(frame);
        let mut instances = Vec::new();
        build(&profile, *frame, 0.0, WIDE, &state, &mut instances);
        series.push(tiles(&rasterise(&instances)));
        let index = state.static_noise.clock(*frame).index;
        starts.push(last_index.is_some_and(|last| last != index));
        last_index = Some(index);
    }
    (series, starts)
}

/// WCAG 2.3.1 general flashes, as the colour field's `wcag_flashes` counts
/// them, for one luminance series at 60 frames per second: opposing changes
/// of at least 0.1 with the darker state below 0.8, paired, most in any
/// 60-frame window.
fn most_flashes(series: &[f32]) -> usize {
    let mut opposing: Vec<usize> = Vec::new();
    let mut last = None;
    // Changes accumulate between reversals, so a slow ramp counts too.
    let (mut low, mut high) = (series[0], series[0]);
    for (frame, &value) in series.iter().enumerate() {
        low = low.min(value);
        high = high.max(value);
        let up = if value - low >= 0.1 && low < 0.8 {
            Some(true)
        } else if high - value >= 0.1 && value < 0.8 {
            Some(false)
        } else {
            None
        };
        if let Some(up) = up {
            if last != Some(up) {
                opposing.push(frame);
                last = Some(up);
            }
            (low, high) = (value, value);
        }
    }
    (0..opposing.len())
        .map(|start| {
            opposing[start..]
                .iter()
                .take_while(|&&frame| frame < opposing[start] + 60)
                .count()
                / 2
        })
        .max()
        .unwrap_or(0)
}

/// Largest change of any tile's mean luminance between consecutive frames
/// and across any 60-frame (one second) span, skipping frames where a
/// section starts (`starts`), and the most WCAG flashes in any tile.
fn luminance_report(series: &[[f32; TILES_X * TILES_Y]], starts: &[bool]) -> (f32, f32, usize) {
    let mut per_frame = 0.0f32;
    let mut per_second = 0.0f32;
    let most = |most: f32, now: &[f32], then: &[f32]| {
        now.iter()
            .zip(then)
            .fold(most, |most, (now, then)| most.max((now - then).abs()))
    };
    for frame in 1..series.len() {
        if !starts[frame] {
            per_frame = most(per_frame, &series[frame], &series[frame - 1]);
        }
        if frame >= 60 && !starts[frame - 59..=frame].contains(&true) {
            per_second = most(per_second, &series[frame], &series[frame - 60]);
        }
    }
    let flashes = (0..TILES_X * TILES_Y)
        .map(|tile| most_flashes(&series.iter().map(|tiles| tiles[tile]).collect::<Vec<_>>()))
        .max()
        .unwrap_or(0);
    (per_frame, per_second, flashes)
}

/// Steady energy and palette, so the only large-area changes are the
/// schedule's: Static's noise density follows energy by design, and
/// procedural energy's 15-frame sawtooth swings pixel noise far more than
/// anything the beat does (see the flash test).
fn steady(frame: ControlFrame) -> ControlFrame {
    ControlFrame {
        energy: 160,
        palette: 0,
        ..frame
    }
}

/// Flash policy for the beat-synced changes, measured with `rasterise`
/// (`pi5` profile, 320x180, 4x4 tiles of mean luminance, steady energy) for
/// three beat inputs, with the frame-timed schedule on the same frames in
/// brackets. The table was measured once over 1400 frames; to stay quick
/// the test checks the beat-synced bounds over the first 700 and 800.
///
/// | input, frames | worst tile step per frame | worst change over 1 s | flashes in 1 s |
/// | --- | --- | --- | --- |
/// | procedural (beat every 16), 0..1400 | 0.076 (timed 0.076) | 0.162 (0.159) | 1 (0) |
/// | procedural, 1900..3300 | 0.050 (0.042) | 0.303 (0.295) | 1 (1) |
/// | live (beat every 30), 0..1400 | 0.076 (0.086) | 0.147 (0.169) | 1 (0) |
/// | live, 1900..3300 | 0.045 (0.043) | 0.304 (0.298) | 1 (1) |
/// | racing (beat every frame, 255 bpm), 0..1400 | 0.099 (0.099) | 0.176 (0.171) | 1 (0) |
/// | racing, 1900..3300 | 0.064 (0.046) | 0.302 (0.300) | 1 (1) |
///
/// Steps skip the frames where a section starts (the white background cuts
/// in or out there, as before: sections stay at least 512 frames apart, see
/// `beat.rs`); flashes count them. The worst per-frame step (racing beats
/// reshuffling the noise every frame) and the worst 1 s change (content
/// resolving and dissolving) match the frame-timed schedule's. The one flash
/// the beat adds is a resolve darkening a tile and a split-flap flip to a
/// larger phrase brightening it again within a second: one pair, under
/// WCAG 2.3.1's three. Reframes ease over at least 15 frames.
///
/// Supporting evidence only, not a substitute for PEAT or a Harding check.
/// With procedural energy (not steady) both schedules measure 4 to 5
/// flashes in 1 s in a pixel-noise tile: energy drives noise density, and
/// that pre-dates beat sync.
#[test]
fn beat_changes_keep_luminance_flashes_bounded() {
    type Input = (&'static str, fn(u64) -> ControlFrame);
    let inputs: [Input; 3] = [
        ("procedural", |index| {
            steady(ControlFrame::procedural(index))
        }),
        ("live", |index| ControlFrame {
            beat_count: (index / 30) as u16,
            ..steady(ControlFrame::procedural(index))
        }),
        ("racing", |index| ControlFrame {
            bpm: 255,
            beat_count: index as u16,
            ..steady(ControlFrame::procedural(index))
        }),
    ];
    for (name, input) in inputs {
        // Text with flips and reframes; then the cut into and out of a
        // white pixel-noise section.
        for range in [0..700, 1900..2700] {
            let frames: Vec<_> = range.clone().map(input).collect();
            let (series, starts) = tile_series(&frames);
            let (step, second, flashes) = luminance_report(&series, &starts);
            let case = format!("{name} {range:?}");
            assert!(step < 0.1, "{case}: step {step}");
            assert!(second < 0.35, "{case}: {second} in 1 s");
            assert!(flashes <= 1, "{case}: {flashes} flashes");
        }
    }
}

#[test]
fn beat_synced_instances_are_deterministic() {
    let profile = RenderProfile::m1();
    let render = || {
        let mut state = SceneState::default();
        let mut bytes = Vec::new();
        for index in 0..1400 {
            let frame = ControlFrame {
                beat_count: (index / 30) as u16,
                ..ControlFrame::procedural(index)
            };
            state.observe(&frame);
            if index % 50 == 0 {
                let mut instances = Vec::new();
                build(&profile, frame, 0.0, WIDE, &state, &mut instances);
                bytes.extend_from_slice(bytemuck::cast_slice::<Instance, u8>(&instances));
            }
        }
        bytes
    };
    assert_eq!(render(), render());
}
