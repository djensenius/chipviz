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
    let noise = Noise::new(frame, f32::from(frame.energy) / 255.0);
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
            ..Noise::new(at(65_535), f32::from(energy as u8) / 255.0)
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

/// Frames sampled by `observed_fingerprint`: glyph text (300 resolved after
/// a flip, 332 mid-flip under a reframe), constant, knot (1430 reframed),
/// noise, then pixel text, constant (on white at 2400 and 2700), knot and
/// noise.
const OBSERVED_FRAMES: [u16; 10] = [0, 300, 332, 900, 1430, 1800, 2400, 2700, 3300, 3900];

/// FNV-1a over Static's instances on `OBSERVED_FRAMES` of a replay from
/// frame 0 through one observed state per profile, as the renderer and
/// `chipviz-capture` run it: the beat-synced path, which `fingerprint`
/// (built before any frame is observed) does not reach.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn observed_fingerprint(input: fn(u16) -> ControlFrame) -> u64 {
    let mut fingerprint: u64 = 0xcbf2_9ce4_8422_2325;
    for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
        let mut state = SceneState::default();
        for index in 0..=OBSERVED_FRAMES[OBSERVED_FRAMES.len() - 1] {
            let frame = input(index);
            state.observe(&frame);
            if !OBSERVED_FRAMES.contains(&index) {
                continue;
            }
            let mut instances = Vec::new();
            build(&profile, frame, 0.0, WIDE, &state, &mut instances);
            for byte in bytemuck::cast_slice::<Instance, u8>(&instances) {
                fingerprint ^= u64::from(*byte);
                fingerprint = fingerprint.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    fingerprint
}

/// The sampled frames reach what the beat drives: a flip under way, a
/// reframe, and every section type.
#[test]
fn observed_frames_cover_flips_reframes_and_sections() {
    let mut state = SceneState::default();
    let (mut flipping, mut reframed, mut indices) = (false, false, Vec::new());
    for index in 0..=OBSERVED_FRAMES[OBSERVED_FRAMES.len() - 1] {
        let frame = at(index);
        state.observe(&frame);
        if OBSERVED_FRAMES.contains(&index) {
            let clock = state.static_noise.clock(frame);
            flipping |= section_at(clock).before_flip().is_some();
            reframed |= clock.framing != REST;
            indices.push(clock.index);
        }
    }
    assert!(flipping && reframed);
    assert_eq!(indices, [0, 0, 0, 1, 2, 3, 4, 5, 6, 7]);
}

// The frame-timed golden above covers the schedule before any frame is
// observed (tests and the first frame); these cover the beat-synced replay,
// with procedural beats (every 16 frames) and live-like ones (every 30).
// Re-recorded for #116: the replay follows the slew-limited energy, so its
// noise density and tint differ; the frame-timed golden, at the frame's own
// energy, is unchanged.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn observed_instances_match_recorded_golden() {
    assert_eq!(observed_fingerprint(at), 0xc026_3bfb_04f8_e00c);
    let live = |index: u16| ControlFrame {
        beat_count: index / 30,
        ..at(index)
    };
    assert_eq!(observed_fingerprint(live), 0xc12c_1145_4c1e_39c4);
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
    rgb.iter().copied().map(luminance).collect()
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
    for (y, row) in image.chunks_exact(RASTER_W).enumerate() {
        let sums = &mut sums[y * TILES_Y / RASTER_H * TILES_X..][..TILES_X];
        for (sum, part) in sums.iter_mut().zip(row.chunks_exact(RASTER_W / TILES_X)) {
            *sum += part.iter().sum::<f32>();
        }
    }
    let area = (RASTER_W * RASTER_H / (TILES_X * TILES_Y)) as f32;
    sums.map(|sum| sum / area)
}

/// Rec. 709 relative luminance of linear RGB, as WCAG uses.
fn luminance([r, g, b]: [f32; 3]) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Mean luminance of a rainbow noise pixel's hue (or its inverse), which
/// `gpu_reference::pixel` draws uniformly at random.
/// `[plain, inverted]`, for `tile_estimate`.
fn rainbow_luminance() -> [f32; 2] {
    const SAMPLES: usize = 4096;
    [false, true].map(|inverted| {
        (0..SAMPLES)
            .map(|sample| {
                let rgb = hue((sample as f32 + 0.5) / SAMPLES as f32);
                luminance(if inverted {
                    rgb.map(|channel| 1.0 - channel)
                } else {
                    rgb
                })
            })
            .sum::<f32>()
            / SAMPLES as f32
    })
}

/// `tiles(&rasterise(instances))` in expectation, without decoding glyph
/// bits or hashing noise per pixel, so the flash test can afford every
/// frame of all eight section types. Each quad blends in draw order over
/// only the pixels of the `RASTER_W` x `RASTER_H` grid it overlaps, each
/// weighted by the share of that pixel it covers, so disjoint quads never
/// dim each other beyond a shared edge pixel. A glyph blends by its lit
/// bits (out of 40), a noise strip by the chance a pixel is drawn
/// (1 - blank) and its mean alpha (`unit` is uniform, so half the colour's),
/// with rainbow pixels at the mean hue luminance. The pixel noise hashes
/// every output pixel independently and each tile holds 3600 of them, so
/// its tile mean sits within about 0.01 of this expectation;
/// `tile_estimate_matches_the_rasteriser` checks that against `rasterise`.
fn tile_estimate(instances: &[Instance], rainbow: [f32; 2]) -> [f32; TILES_X * TILES_Y] {
    const TILE_W: usize = RASTER_W / TILES_X;
    let mut pixels = vec![0.0f32; RASTER_W * RASTER_H];
    // Tile sums, kept up to date as pixels blend.
    let mut sums = [0.0f32; TILES_X * TILES_Y];
    // Share of pixel `index` inside `low..high`.
    let overlap = |low: f32, high: f32, index: usize| {
        (high.min(index as f32 + 1.0) - low.max(index as f32)).max(0.0)
    };
    let span = |low: f32, high: f32, size: usize| {
        low.floor().clamp(0.0, size as f32) as usize..high.ceil().clamp(0.0, size as f32) as usize
    };
    for instance in instances {
        let [cx, cy, _] = instance.center;
        let [w, h] = instance.size.map(f32::abs);
        let [low, high] = instance.pattern;
        let rgb = [instance.color[0], instance.color[1], instance.color[2]];
        let (alpha, light) = if low < -0.5 {
            let flags = -low - 1.0;
            let inverted = flags >= 4.0;
            let code = flags - if inverted { 4.0 } else { 0.0 };
            let blank = code - if code >= 2.0 { 2.0 } else { 0.0 };
            let light = if code >= 2.0 {
                rainbow[usize::from(inverted)]
            } else {
                luminance(rgb)
            };
            ((1.0 - blank) * 0.5 * instance.color[3], light)
        } else if low > 0.5 || high > 0.5 {
            let lit = (low as u64 | (high as u64) << 20).count_ones();
            (lit as f32 / 40.0 * instance.color[3], luminance(rgb))
        } else {
            (instance.color[3], luminance(rgb))
        };
        // The quad in pixels, rows counting down from the top.
        let left = (cx - w * 0.5 + 1.0) * 0.5 * RASTER_W as f32;
        let right = (cx + w * 0.5 + 1.0) * 0.5 * RASTER_W as f32;
        let top = (-cy - h * 0.5 + 1.0) * 0.5 * RASTER_H as f32;
        let bottom = (-cy + h * 0.5 + 1.0) * 0.5 * RASTER_H as f32;
        let columns = span(left, right, RASTER_W);
        if columns.is_empty() {
            continue;
        }
        // Partly covered edge columns; the ones between are covered whole.
        let (first, last) = (columns.start, columns.end - 1);
        let edges = [overlap(left, right, first), overlap(left, right, last)];
        for y in span(top, bottom, RASTER_H) {
            let weight = overlap(top, bottom, y) * alpha;
            let row = &mut pixels[y * RASTER_W..][..RASTER_W];
            let sums = &mut sums[y * TILES_Y / RASTER_H * TILES_X..][..TILES_X];
            for (x, edge) in [(first, edges[0]), (last, edges[1])] {
                let change = edge.min(right - left) * weight * (light - row[x]);
                row[x] += change;
                sums[x / TILE_W] += change;
                if first == last {
                    break;
                }
            }
            // Whole pixels, a tile at a time.
            let mut x = first + 1;
            while x < last {
                let end = last.min((x / TILE_W + 1) * TILE_W);
                let mut change = 0.0;
                for pixel in &mut row[x..end] {
                    let step = weight * (light - *pixel);
                    *pixel += step;
                    change += step;
                }
                sums[x / TILE_W] += change;
                x = end;
            }
        }
    }
    let area = (RASTER_W * RASTER_H / (TILES_X * TILES_Y)) as f32;
    sums.map(|sum| sum / area)
}

/// Frames per sample in glyph sections in the flash tests: building one
/// costs a few milliseconds in debug, so they sample at 30 Hz; pixel
/// sections, about five times cheaper, sample every frame. Every
/// beat-driven change eases or staggers over at least 15 frames, and WCAG's
/// three flashes a second are six reversals, well inside 30 samples. The
/// blind spot: in glyph sections a pattern that alternates every frame, or
/// a one-frame blip, can fall between samples; the beat drives neither, and
/// the 60 Hz noise reshuffles are tile-averaged (see `tile_estimate`).
const GLYPH_STRIDE: usize = 2;
/// Frames in one second.
const SECOND: usize = 60;

/// A tile luminance series: per sample, its frame, its tiles' estimated
/// luminance (see `tile_estimate`) and its section index.
#[derive(Default)]
struct Series {
    frames: Vec<usize>,
    tiles: Vec<[f32; TILES_X * TILES_Y]>,
    sections: Vec<u16>,
}

impl Series {
    /// Samples `start..end`.
    fn slice(&self, range: std::ops::Range<usize>) -> Series {
        Series {
            frames: self.frames[range.clone()].to_vec(),
            tiles: self.tiles[range.clone()].to_vec(),
            sections: self.sections[range].to_vec(),
        }
    }

    /// Samples in section `index`.
    fn section(&self, index: u16) -> Series {
        let first = self.sections.iter().position(|&section| section == index);
        let last = self.sections.iter().rposition(|&section| section == index);
        self.slice(first.unwrap()..last.unwrap() + 1)
    }
}

/// `frames` rendered through a state stepped on every frame, untimed (each
/// frame 1/60 s) or, with `fps`, on a clock at that rate as the renderer
/// runs it: the frames of `sections` and the first 60 frames of the next
/// (or until `frames` ends), sampled every frame in pixel sections and
/// every `GLYPH_STRIDE`th in glyph sections.
fn tile_series(
    frames: impl Iterator<Item = ControlFrame>,
    fps: Option<f64>,
    sections: std::ops::Range<u16>,
) -> Series {
    let until = sections.end;
    let profile = RenderProfile::pi5();
    let rainbow = rainbow_luminance();
    let mut state = SceneState::default();
    let mut series = Series::default();
    let mut instances = Vec::new();
    let mut reached = None;
    for (index, frame) in frames.enumerate() {
        match fps {
            Some(fps) => state.observe_at(&frame, index as f64 / fps),
            None => state.observe(&frame),
        }
        let clock = state.static_noise.clock(frame);
        if clock.index == until {
            let reached = *reached.get_or_insert(index);
            if index >= reached + SECOND {
                break;
            }
        }
        let pixels = section_at(clock).style == Style::Pixels;
        if clock.index < sections.start || !pixels && index % GLYPH_STRIDE != 0 {
            continue;
        }
        instances.clear();
        build(&profile, frame, 0.0, WIDE, &state, &mut instances);
        series.frames.push(index);
        series.tiles.push(tile_estimate(&instances, rainbow));
        series.sections.push(clock.index);
    }
    series
}

/// WCAG 2.3.1 general flashes, as the colour field's `wcag_flashes` counts
/// them, for one luminance series (`values`, sampled on `frames`, `second`
/// frames to a second): opposing changes of at least 0.1 with the darker
/// state below 0.8, paired, most in any one-second window.
fn most_flashes(frames: &[usize], values: &[f32], second: usize) -> usize {
    let mut opposing: Vec<usize> = Vec::new();
    let mut last = None;
    // Changes accumulate between reversals, so a slow ramp counts too.
    let (mut low, mut high) = (values[0], values[0]);
    for (&frame, &value) in frames.iter().zip(values) {
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
                .take_while(|&&frame| frame < opposing[start] + second)
                .count()
                / 2
        })
        .max()
        .unwrap_or(0)
}

/// Largest change of any tile's mean luminance between consecutive samples
/// and between any two samples at most one second apart (a rise and fall
/// within the second counts, not just its endpoints), never across a
/// section start (`sections` changes), and the most WCAG flashes in any
/// tile.
fn luminance_report(series: &Series) -> (f32, f32, usize) {
    let Series {
        frames,
        tiles,
        sections,
    } = series;
    let starts: Vec<bool> = (0..sections.len())
        .map(|sample| sample > 0 && sections[sample] != sections[sample - 1])
        .collect();
    let mut per_sample = 0.0f32;
    let mut per_second = 0.0f32;
    // The window: samples `earliest..=sample`, at most one second apart and
    // in one section. Per tile, the window's samples in increasing (`lows`)
    // and decreasing (`highs`) order of value, so each front is the
    // window's minimum or maximum.
    let mut earliest = 0;
    let mut lows = vec![std::collections::VecDeque::new(); TILES_X * TILES_Y];
    let mut highs = lows.clone();
    for sample in 0..tiles.len() {
        if starts[sample] {
            earliest = sample;
        } else if sample > 0 {
            for (now, then) in tiles[sample].iter().zip(&tiles[sample - 1]) {
                per_sample = per_sample.max((now - then).abs());
            }
        }
        while frames[earliest] + SECOND < frames[sample] {
            earliest += 1;
        }
        for (tile, (lows, highs)) in lows.iter_mut().zip(&mut highs).enumerate() {
            let value = tiles[sample][tile];
            while lows
                .back()
                .is_some_and(|&low: &usize| tiles[low][tile] >= value)
            {
                lows.pop_back();
            }
            while highs
                .back()
                .is_some_and(|&high: &usize| tiles[high][tile] <= value)
            {
                highs.pop_back();
            }
            lows.push_back(sample);
            highs.push_back(sample);
            while lows.front().is_some_and(|&low| low < earliest) {
                lows.pop_front();
            }
            while highs.front().is_some_and(|&high| high < earliest) {
                highs.pop_front();
            }
            per_second = per_second.max(tiles[highs[0]][tile] - tiles[lows[0]][tile]);
        }
    }
    (per_sample, per_second, worst_flashes(series, SECOND))
}

/// The most WCAG flashes in 1 s in any tile of `series`, with `second`
/// frames to a second.
fn worst_flashes(series: &Series, second: usize) -> usize {
    (0..TILES_X * TILES_Y)
        .map(|tile| {
            let values: Vec<f32> = series.tiles.iter().map(|tiles| tiles[tile]).collect();
            most_flashes(&series.frames, &values, second)
        })
        .max()
        .unwrap_or(0)
}

/// Steady energy and palette, so the only large-area changes are the
/// schedule's: Static's noise density follows energy by design, and the
/// energy tests below bound that on its own.
fn steady(frame: ControlFrame) -> ControlFrame {
    ControlFrame {
        energy: 160,
        palette: 0,
        ..frame
    }
}

/// A tile that rises to 0.4 and falls back within every second has a 1 s
/// change of 0.4, over the flash test's 0.35, though samples exactly one
/// second apart never differ; across a section start it does not count.
#[test]
fn one_second_change_counts_a_rise_and_fall_within_the_second() {
    let pulse = |frame: usize| 0.4 * (1.0 - (frame as f32 % 60.0 - 30.0).abs() / 30.0);
    let frames: Vec<usize> = (0..=180).collect();
    let series = Series {
        tiles: frames
            .iter()
            .map(|&frame| {
                let mut tiles = [0.0; TILES_X * TILES_Y];
                tiles[5] = pulse(frame);
                tiles
            })
            .collect(),
        sections: vec![0; frames.len()],
        frames: frames.clone(),
    };
    let endpoints = (SECOND..frames.len())
        .map(|frame| (pulse(frame) - pulse(frame - SECOND)).abs())
        .fold(0.0f32, f32::max);
    assert_eq!(endpoints, 0.0);
    let (_, second, _) = luminance_report(&series);
    assert!((second - 0.4).abs() < 1e-6 && second >= 0.35, "{second}");
    // A new section every 20 frames: each holds only a third of a pulse.
    let cut = Series {
        sections: frames.iter().map(|&frame| (frame / 20) as u16).collect(),
        ..series
    };
    let (_, second, _) = luminance_report(&cut);
    assert!(second < 0.3, "{second}");
}

/// Flash policy for the beat-synced changes, measured with `tile_estimate`
/// (`pi5` profile, 4x4 tiles of mean luminance, steady energy; sampled
/// every frame in pixel sections and every `GLYPH_STRIDE` frames in glyph
/// sections) over all eight section types (index 0..8: text, constant,
/// knot and noise, in glyphs then pixels) and the cut into the ninth, for
/// three beat inputs. Each cell is the worst over the four glyph or four
/// pixel sections, with the frame-timed schedule on the same frames in
/// brackets:
///
/// | input | sections | worst tile step per sample | worst swing within 1 s | flashes in 1 s |
/// | --- | --- | --- | --- | --- |
/// | procedural (beat every 16) | glyphs | 0.050 (timed 0.041) | 0.140 (0.140) | 1 (0) |
/// | procedural | pixels | 0.026 (0.013) | 0.288 (0.295) | 1 (1) |
/// | live (beat every 30) | glyphs | 0.046 (0.042) | 0.136 (0.143) | 1 (0) |
/// | live | pixels | 0.026 (0.013) | 0.296 (0.295) | 1 (1) |
/// | racing (beat every frame, 255 bpm) | glyphs | 0.064 (0.052) | 0.141 (0.136) | 1 (0) |
/// | racing | pixels | 0.055 (0.013) | 0.295 (0.295) | 1 (1) |
///
/// A step is over two frames in glyph sections and one in pixel sections.
/// Steps skip the samples where a section starts (the white background
/// cuts in or out there, as before: sections stay at least 512 frames
/// apart, see `beat.rs`); flashes count them. The worst 1 s change (content
/// resolving and dissolving, largest in the pixel sections) matches the
/// frame-timed schedule's. Racing beats reshuffling the noise and reframes
/// moving it raise the step: in pixel sections a reframe's 15-frame ease
/// moves up to 0.1 in two frames, which is why they are sampled every
/// frame (0.055). The one flash the beat adds in glyph sections is a
/// resolve darkening a tile and a split-flap flip to a larger phrase or a
/// reframe brightening it again within a second: one pair, under WCAG
/// 2.3.1's three. (A one-off release run sampling every frame in all
/// sections measured glyph steps up to 0.068 and 1 s changes up to 0.154,
/// with the same flash counts.)
///
/// Supporting evidence only, not a substitute for PEAT or a Harding check.
/// Energy drives noise density, slew-limited (#116):
/// `procedural_energy_keeps_noise_flashes_bounded` and the
/// `square_wave_energy_keeps_pixel_noise_flashes_bounded` and
/// `square_wave_energy_keeps_glyph_noise_flashes_bounded` tests (with the
/// pixel test at 120 and 240 fps) bound that.
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
    // Every section type (index 0..8) and the first second of the ninth;
    // live sections start every 600 frames. One thread per input and half
    // (glyph sections, then pixel sections, each with the cut after it).
    let runs: Vec<_> = std::thread::scope(|scope| {
        let runs: Vec<_> = inputs
            .iter()
            .flat_map(|&(name, input)| {
                [0..4, 4..8].map(|half| {
                    scope.spawn(move || {
                        let series = tile_series((0..).map(input), None, half.clone());
                        (name, half, series)
                    })
                })
            })
            .collect();
        runs.into_iter().map(|run| run.join().unwrap()).collect()
    });
    for (name, half, series) in runs {
        assert!(series.sections.contains(&half.end), "{name} {half:?}");
        for section in half {
            let (step, second, flashes) = luminance_report(&series.section(section));
            let case = format!("{name} section {section}");
            assert!(step < 0.1, "{case}: step {step}");
            assert!(second < 0.35, "{case}: {second} in 1 s");
            assert!(flashes <= 1, "{case}: {flashes} flashes");
        }
        // Across the section cuts too.
        let (_, _, flashes) = luminance_report(&series);
        assert!(flashes <= 1, "{name}: {flashes} flashes");
    }
}

/// #116: Static's noise density and tint follow the energy, slew-limited
/// (`ENERGY_SLEW`). With procedural energy, a sawtooth over 15 frames, and
/// its palettes, replayed through an observed state as the renderer runs
/// it, the worst tile flashes at most 3 times in 1 s over all eight
/// section types, sampled as in the flash test (measured: 1; before the
/// limit, 4). The frame-timed path (built before any frame is observed, so
/// only a state's first frame) follows the raw energy, and would measure 4
/// if rendered as a sequence.
#[test]
fn procedural_energy_keeps_noise_flashes_bounded() {
    // One thread per rate and half, as in the flash test.
    let flashes = std::thread::scope(|scope| {
        [60.0, 120.0, 240.0]
            .map(|fps| {
                [0..4, 4..8].map(|half| {
                    scope.spawn(move || {
                        let frames = (0..4096).map(ControlFrame::procedural);
                        let series = tile_series(frames, Some(fps), half);
                        worst_flashes(&series, fps as usize)
                    })
                })
            })
            .map(|halves| halves.map(|run| run.join().unwrap()))
    });
    assert!(
        flashes.iter().flatten().all(|&flashes| flashes <= 3),
        "{flashes:?}"
    );
}

/// An energy square wave as the renderer would see it: `fps` observations
/// a second (the frame counter advancing on each, as procedural and mic
/// input do on every redraw), energy jumping between `low` and 255 every
/// half `period` observations, and `palette` held or (`None`) procedural.
#[derive(Clone, Copy)]
struct Wave {
    fps: f64,
    period: usize,
    low: u8,
    palette: Option<u8>,
}

/// Worst flash pairs in 1 s in any tile for `wave` over 2.5 s from each of
/// `starts`, sampled every `stride` observations.
fn square_wave_flashes(wave: Wave, starts: &[u16], stride: usize) -> usize {
    let profile = RenderProfile::pi5();
    let rainbow = rainbow_luminance();
    let mut worst = 0;
    for &start in starts {
        let mut state = SceneState::default();
        let mut series = Series::default();
        let mut instances = Vec::new();
        for index in 0..(2.5 * wave.fps) as usize {
            let high = index * 2 / wave.period % 2 == 1;
            let procedural = ControlFrame::procedural(u64::from(start) + index as u64);
            let frame = ControlFrame {
                energy: if high { 255 } else { wave.low },
                palette: wave.palette.unwrap_or(procedural.palette),
                ..procedural
            };
            state.observe_at(&frame, index as f64 / wave.fps);
            if index % stride != 0 {
                continue;
            }
            instances.clear();
            build(&profile, frame, 0.0, WIDE, &state, &mut instances);
            series.frames.push(index);
            series.tiles.push(tile_estimate(&instances, rainbow));
            series.sections.push(0);
        }
        worst = worst.max(worst_flashes(&series, wave.fps as usize));
    }
    worst
}

/// `square_wave_flashes` for each of `waves`, a thread per few waves:
/// (wave, flashes).
fn square_wave_sweep(waves: &[Wave], starts: &[u16], stride: usize) -> Vec<(Wave, usize)> {
    std::thread::scope(|scope| {
        let runs: Vec<_> = waves
            .chunks(waves.len().div_ceil(12))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&wave| (wave, square_wave_flashes(wave, starts, stride)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        runs.into_iter()
            .flat_map(|run| run.join().unwrap())
            .collect()
    })
}

/// Silence to full energy at 60 fps, every period from 2 to 60 frames,
/// with the procedural palettes.
fn sixty_fps_waves() -> Vec<Wave> {
    (2..=60)
        .map(|period| Wave {
            fps: 60.0,
            period,
            low: 0,
            palette: None,
        })
        .collect()
}

fn assert_bounded(results: Vec<(Wave, usize)>) {
    for (wave, flashes) in results {
        let Wave {
            fps, period, low, ..
        } = wave;
        assert!(
            flashes <= 3,
            "{fps} fps, period {period}, {low}..255: {flashes} flashes"
        );
    }
}

/// #116, the worst case for pixel noise, which follows energy most: energy
/// jumping between silence and full every half period, for every period
/// from 2 to 60 frames, over the first 150 frames of the white pixel text
/// section (4) and black pixel noise (7), before content thins the noise,
/// through the procedural palettes 0 to 3, sampled every frame. From
/// silence to full energy a pixel tile's luminance moves by 0.30, at most
/// about 0.39 per unit of energy (palette 2), so with the slew limit it
/// moves at most 0.39 / 48 a frame: a reversal of 0.1 takes at least 13
/// frames and 1 s holds at most 5 (2 flashes). Measured worst: 2 flashes
/// in 1 s (periods 44 to 46); without the limit 30 (period 2), and over 3
/// for every period under 18 frames.
#[test]
fn square_wave_energy_keeps_pixel_noise_flashes_bounded() {
    let starts = [4 * SECTION_FRAMES, 7 * SECTION_FRAMES];
    assert_bounded(square_wave_sweep(&sixty_fps_waves(), &starts, 1));
}

/// As above for glyph noise: plain noise (section 3) and the white glyph
/// constant (9), sampled every `GLYPH_STRIDE` frames. Its tile luminance
/// moves by about 0.14 from silence to full energy, though cells rehash as
/// their change periods follow energy. Measured worst: no flashes; without
/// the limit 13 (period 4), and over 3 for periods 3 to 17 frames.
#[test]
fn square_wave_energy_keeps_glyph_noise_flashes_bounded() {
    let starts = [3 * SECTION_FRAMES, 9 * SECTION_FRAMES];
    assert_bounded(square_wave_sweep(&sixty_fps_waves(), &starts, GLYPH_STRIDE));
}

/// The pixel test above at redraw rates over 60 fps (`fps / 60`
/// observations per 1/60 s), where a per-frame limit would let the energy
/// move two or four times as fast: square waves between silence or 180
/// (the review's case, where luminance is steepest) and full energy, with
/// palette 2 (the steepest) on black pixel noise (7), at periods of 2 to 60
/// sixtieths of a second (every other sixtieth at 120 fps, every seventh at
/// 240) plus 17 observations; and the review's 180..255
/// wave at period 17 on both pixel sections (4 and 7). The limit is per
/// 1/60 s of clock, so the bound and argument are those at 60 fps.
fn high_rate_waves(fps: f64, step: usize) -> Vec<Wave> {
    let per = fps as usize / 60;
    (per * 2..=per * 60)
        .step_by(step)
        .chain([17])
        .flat_map(|period| {
            [0, 180].map(|low| Wave {
                fps,
                period,
                low,
                palette: Some(2),
            })
        })
        .collect()
}

fn review_case(fps: f64) -> Vec<(Wave, usize)> {
    let wave = Wave {
        fps,
        period: 17,
        low: 180,
        palette: Some(2),
    };
    let starts = [4 * SECTION_FRAMES, 7 * SECTION_FRAMES];
    square_wave_sweep(&[wave], &starts, 1)
}

#[test]
fn square_wave_energy_keeps_pixel_noise_flashes_bounded_at_120_fps() {
    let starts = [7 * SECTION_FRAMES];
    assert_bounded(square_wave_sweep(&high_rate_waves(120.0, 4), &starts, 1));
    assert_bounded(review_case(120.0));
}

#[test]
fn square_wave_energy_keeps_pixel_noise_flashes_bounded_at_240_fps() {
    let starts = [7 * SECTION_FRAMES];
    assert_bounded(square_wave_sweep(&high_rate_waves(240.0, 28), &starts, 1));
    assert_bounded(review_case(240.0));
}

/// A solid white quad over pixels `x` x `y` of the `RASTER_W` x
/// `RASTER_H` grid, at `alpha`.
fn solid(x: [f32; 2], y: [f32; 2], alpha: f32) -> Instance {
    let ndc = |pixels: [f32; 2], size: usize| pixels.map(|pixel| pixel / size as f32 * 2.0 - 1.0);
    let ([left, right], [top, bottom]) = (ndc(x, RASTER_W), ndc(y, RASTER_H));
    Instance {
        center: [(left + right) * 0.5, -(top + bottom) * 0.5, 0.0],
        size: [right - left, bottom - top],
        color: [1.0, 1.0, 1.0, alpha],
        ..bytemuck::Zeroable::zeroed()
    }
}

/// Quads that do not overlap must not dim each other (four quarter-tile
/// quads at 0.4 cover their tile at 0.4, not 0.3439), and overlapping ones
/// composite only where they overlap, as `rasterise` does.
#[test]
fn tile_estimate_keeps_quads_where_they_are() {
    let (width, height) = ((RASTER_W / TILES_X) as f32, (RASTER_H / TILES_Y) as f32);
    // Four disjoint quarter-tile strips, on whole pixels so `rasterise`
    // samples them exactly too.
    let quarters: Vec<_> = (0..4)
        .map(|quarter| {
            let x = quarter as f32 * width * 0.25;
            solid([x, x + width * 0.25], [0.0, height], 0.4)
        })
        .collect();
    // Half-transparent: the left half, then the right three quarters, of
    // the first tile. They overlap in its second quarter (0.75); the
    // other quarters get one (0.5): 0.5 + 0.75 + 0.5 + 0.5 over four.
    let overlapping = [
        solid([0.0, width * 0.5], [0.0, height], 0.5),
        solid([width * 0.25, width], [0.0, height], 0.5),
    ];
    for (instances, expected) in [(&quarters[..], 0.4), (&overlapping[..], 0.5625)] {
        let estimate = tile_estimate(instances, rainbow_luminance());
        let exact = tiles(&rasterise(instances));
        assert!((estimate[0] - expected).abs() < 1e-4, "{estimate:?}");
        assert!((exact[0] - expected).abs() < 1e-4, "{exact:?}");
        assert!(
            estimate[1..]
                .iter()
                .chain(&exact[1..])
                .all(|&mean| mean == 0.0)
        );
    }
}

/// `tile_estimate` against the point-sampled `rasterise` on pairs of
/// frames `GLYPH_STRIDE` apart every 128 frames across all eight
/// section types, with the procedural palettes and with random hues
/// (palette 3, the rainbow branch): both each tile's level and its change
/// between the two samples, which the flash test bounds (worst measured
/// 0.010 and 0.013).
#[test]
fn tile_estimate_matches_the_rasteriser() {
    let profile = RenderProfile::pi5();
    let rainbow = rainbow_luminance();
    let (mut level, mut change) = (0.0f32, 0.0f32);
    for palette in [None, Some(3)] {
        let mut state = SceneState::default();
        let mut last: Option<([f32; TILES_X * TILES_Y], [f32; TILES_X * TILES_Y])> = None;
        for index in 0..4096u64 {
            let procedural = ControlFrame::procedural(index);
            let frame = ControlFrame {
                palette: palette.unwrap_or(procedural.palette),
                ..procedural
            };
            state.observe(&frame);
            let offset = index % 128;
            if offset != 100 && offset != 100 + GLYPH_STRIDE as u64 {
                continue;
            }
            let mut instances = Vec::new();
            build(&profile, frame, 0.0, WIDE, &state, &mut instances);
            let exact = tiles(&rasterise(&instances));
            let estimate = tile_estimate(&instances, rainbow);
            for (exact, estimate) in exact.iter().zip(estimate) {
                level = level.max((exact - estimate).abs());
            }
            if let Some((last_exact, last_estimate)) = last.take() {
                for tile in 0..TILES_X * TILES_Y {
                    let exact: f32 = exact[tile] - last_exact[tile];
                    let estimate: f32 = estimate[tile] - last_estimate[tile];
                    change = change.max((exact - estimate).abs());
                }
            } else {
                last = Some((exact, estimate));
            }
        }
    }
    assert!(level < 0.02, "level {level}");
    assert!(change < 0.02, "change {change}");
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
