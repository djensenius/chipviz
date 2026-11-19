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
        .map(|(index, target)| cell(index, target, section, noise))
        .collect()
}

fn build_frame(profile: &RenderProfile, frame: ControlFrame, aspect: f32) -> Vec<Instance> {
    let mut instances = Vec::new();
    build(profile, frame, 0.0, aspect, &mut instances);
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
        build(&profile, at(40), 0.0, WIDE, &mut instances);
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

#[test]
fn noise_pattern_decodes_like_the_shader() {
    for (energy, beat, rainbow, blank) in [
        (0, 0, false, 0.2),
        (255, 7, true, 0.985),
        (90, 5, false, 0.0),
    ] {
        let noise = Noise {
            beat,
            ..Noise::new(ControlFrame {
                energy,
                ..at(65_535)
            })
        };
        let [mode, packed] = noise_pattern(blank, rainbow, noise);
        // Mirror of `pixel_noise` in shader.wgsl.
        let code = -mode - 1.0;
        let is_rainbow = code >= 2.0;
        let decoded_blank = code - if is_rainbow { 2.0 } else { 0.0 };
        let packed = (packed + 0.5) as u32;
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
    assert_eq!(fingerprint(), 0x6557_4ce2_d8d1_838c);
}
