use super::*;
use crate::scenes::static_noise::font::glyph;

const WIDE: f32 = 16.0 / 9.0;

fn at(frame: u16) -> ControlFrame {
    ControlFrame::procedural(u64::from(frame))
}

/// Frame `local` of section `index`.
fn frame_in(index: u16, local: u16) -> ControlFrame {
    at(index * SECTION_FRAMES + local)
}

fn build_frame(profile: &RenderProfile, frame: ControlFrame, aspect: f32) -> Vec<Instance> {
    let mut instances = Vec::new();
    build(profile, frame, 0.0, aspect, &mut instances);
    instances
}

#[test]
fn timeline_reaches_every_object_background_morph_and_overlay() {
    let sections: Vec<Section> = (0..30).map(|index| section(frame_in(index, 100))).collect();
    let first = |count: usize| sections[..count].iter();
    for kind in Kind::ALL {
        // Every object meets every background, and both overlays early.
        for background in Background::ALL {
            assert!(first(30).any(|s| (s.object, s.background) == (kind, background)));
        }
        for overlay in [Overlay::Camcorder, Overlay::Infomercial] {
            assert!(first(12).any(|s| (s.object, s.overlay) == (kind, overlay)));
        }
    }
    for morph in [Morph::Cycle, Morph::Sweep, Morph::BeatJump] {
        assert!(first(6).any(|s| s.morph == morph));
    }
}

#[test]
fn beats_and_drops_do_not_move_the_schedule() {
    let shifted = ControlFrame {
        beat_count: 999,
        beat_phase: 3,
        flags: 0x07,
        ..at(1300)
    };
    assert_eq!(section(shifted), section(at(1300)));
    assert_eq!(section(at(1300)).index, 2);
    assert_eq!(section(at(1300)).local, 1300 - 2 * SECTION_FRAMES);
}

#[test]
fn instances_stay_within_both_profile_budgets() {
    let frames = (0..12)
        .flat_map(|index| [0, 30, 48, 200, 490].map(|local| frame_in(index, local)))
        .flat_map(|frame| {
            [
                frame,
                ControlFrame {
                    energy: 255,
                    flags: 0x07,
                    ..frame
                },
            ]
        });
    let frames: Vec<ControlFrame> = frames.collect();
    for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
        for aspect in [WIDE, 1.0, 9.0 / 16.0, 21.0 / 9.0] {
            for &frame in &frames {
                let instances = build_frame(&profile, frame, aspect);
                assert!(instances.len() <= profile.instance_count, "{}", frame.frame);
                assert!(instances.iter().all(|instance| {
                    let [x, y, z] = instance.center;
                    [x, y, z, instance.rotation]
                        .into_iter()
                        .chain(instance.size)
                        .chain(instance.color)
                        .chain(instance.pattern)
                        .all(f32::is_finite)
                }));
            }
        }
    }
}

#[test]
fn appended_build_respects_the_remaining_budget() {
    let profile = RenderProfile::pi5();
    let mut instances =
        vec![<Instance as bytemuck::Zeroable>::zeroed(); profile.instance_count - 100];
    build(&profile, frame_in(5, 200), 0.0, WIDE, &mut instances);
    assert!(instances.len() <= profile.instance_count);
    let mut full = vec![<Instance as bytemuck::Zeroable>::zeroed(); profile.instance_count];
    build(&profile, frame_in(5, 200), 0.0, WIDE, &mut full);
    assert_eq!(full.len(), profile.instance_count);
}

#[test]
fn every_object_is_drawn_and_pi5_uses_fewer_splats() {
    for index in 0..6 {
        let frame = frame_in(index, 200);
        let section = section(frame);
        let pose = pose(section, frame);
        let stage = stage(frame, section, WIDE);
        let drawn = [RenderProfile::m1(), RenderProfile::pi5()].map(|profile| {
            let (_, budget) = background(&profile, section, &stage, profile.instance_count);
            let splats = draw_object(section, frame, pose, WIDE, budget).len();
            assert!(splats <= budget, "{:?} {}", section.object, profile.name);
            splats
        });
        let [m1, pi5] = drawn;
        assert!(pi5 > 500, "{:?} pi5 {pi5}", section.object);
        assert!(m1 > 2 * pi5, "{:?} m1 {m1} pi5 {pi5}", section.object);
    }
}

#[test]
fn splat_counts_do_not_depend_on_the_look() {
    let loud = Look {
        t: 333.0,
        spectrum: [1.0; 8],
        energy: 1.0,
        yaw: 2.0,
        ..Look::default()
    };
    for kind in Kind::ALL {
        let step = step_for(kind, 4_000);
        let [mut quiet_mesh, mut loud_mesh] = [Mesh::new(step), Mesh::new(step)];
        kind.build(&mut quiet_mesh, &Look::default());
        kind.build(&mut loud_mesh, &loud);
        assert_eq!(quiet_mesh.splats.len(), loud_mesh.splats.len(), "{kind:?}");
        assert!(loud_mesh.splats.len() <= 4_000);
    }
}

#[test]
fn overlay_survives_the_final_truncate() {
    for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
        for index in 0..12 {
            // Drop flag and a downbeat: the most sparkles and the flare.
            let frame = ControlFrame {
                flags: 0x05,
                beat_count: 8,
                beat_phase: 0,
                ..frame_in(index, 200)
            };
            let section = section(frame);
            let stage = stage(frame, section, WIDE);
            let mut overlay = Vec::new();
            match section.overlay {
                Overlay::Camcorder => backdrop::camcorder(&stage, &mut overlay),
                Overlay::Infomercial => backdrop::infomercial(&stage, section.object, &mut overlay),
            }
            let built = build_frame(&profile, frame, WIDE);
            assert!(
                bytemuck::cast_slice::<Instance, u8>(&built)
                    .ends_with(bytemuck::cast_slice(&overlay)),
                "{} section {index}",
                profile.name
            );
        }
    }
}

#[test]
fn hue_turns_keep_luminance() {
    for color in [
        [0.9, 0.85, 0.1],
        [0.05, 0.1, 0.9],
        [0.8, 0.74, 0.6],
        [1.0, 0.0, 0.0],
    ] {
        for step in 0..12 {
            let turned = turn_hue(color, step as f32 * TAU / 12.0);
            let (before, after) = (backdrop::luminance(color), backdrop::luminance(turned));
            assert!((before - after).abs() < 1e-3, "{color:?} {step}: {after}");
            assert!(turned.iter().all(|channel| (0.0..=1.0).contains(channel)));
        }
    }
}

#[test]
fn build_is_deterministic() {
    for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
        for frame in [0, 48, 777, 2_000, 3_001] {
            let a = build_frame(&profile, at(frame), WIDE);
            let b = build_frame(&profile, at(frame), WIDE);
            assert_eq!(
                bytemuck::cast_slice::<Instance, u8>(&a),
                bytemuck::cast_slice::<Instance, u8>(&b)
            );
        }
    }
}

#[test]
fn chroma_key_changes_hue_not_brightness_and_slowly() {
    let first = backdrop::luminance(backdrop::chroma_key(0));
    let mut cuts = 0;
    for clock in 1..2_000u16 {
        let key = backdrop::chroma_key(clock);
        assert!(
            (backdrop::luminance(key) - first).abs() < 0.03,
            "clock {clock}"
        );
        if key != backdrop::chroma_key(clock - 1) {
            cuts += 1;
        }
    }
    // At most two cuts a second at 60 fps.
    const { assert!(backdrop::CHROMA_FRAMES >= 30) };
    assert!(cuts <= 2_000 / 30 + 1, "{cuts}");
}

#[test]
fn overlay_text_uses_only_font_characters() {
    let mut lines: Vec<&str> = vec!["REC", "SP", "JAN. 1 1989", "12:34:56 PM", "NEW!"];
    lines.extend(backdrop::TAGLINES);
    lines.extend(Kind::ALL.map(Kind::caption));
    for line in lines {
        for character in line.chars() {
            assert!(glyph(character).is_some(), "{character:?} in {line:?}");
        }
    }
}

#[test]
fn overlay_text_stays_inside_the_frame() {
    for aspect in [WIDE, 9.0 / 16.0] {
        for index in 0..12 {
            for local in [0, 128, 256, 384] {
                let frame = ControlFrame {
                    beat_phase: 0,
                    ..frame_in(index, local)
                };
                let stage = stage(frame, section(frame), aspect);
                let mut overlay = Vec::new();
                backdrop::camcorder(&stage, &mut overlay);
                backdrop::infomercial(&stage, section(frame).object, &mut overlay);
                // Glyphs are the only quads with a non-negative, non-zero pattern.
                let glyphs = overlay.iter().filter(|instance| {
                    instance.pattern.iter().all(|&bits| bits >= 0.0) && instance.pattern != [0.0; 2]
                });
                let limit = 1.0 - backdrop::TEXT_MARGIN + 1e-4;
                for glyph in glyphs {
                    let [left, right] =
                        [-0.5, 0.5].map(|side| glyph.center[0] + side * glyph.size[0]);
                    assert!(
                        left >= -limit && right <= limit,
                        "{aspect} {index}: {left} {right}"
                    );
                }
            }
        }
    }
}

#[test]
fn zoom_punch_lands_on_the_beat() {
    let on_beat = ControlFrame {
        beat_phase: 0,
        ..frame_in(0, 200)
    };
    let off_beat = ControlFrame {
        beat_phase: 200,
        ..on_beat
    };
    let section = section(on_beat);
    assert!(pose(section, on_beat).scale > pose(section, off_beat).scale * 1.08);
}

#[test]
fn intro_and_outro_shrink_the_object_to_nothing() {
    for index in 0..6 {
        let first = frame_in(index, 0);
        assert_eq!(pose(section(first), first).scale, 0.0);
        let last = frame_in(index, SECTION_FRAMES - 1);
        assert!(pose(section(last), last).scale < 0.01);
    }
}

#[test]
fn flare_fires_on_drop_downbeat_and_reveal() {
    let quiet = ControlFrame {
        beat_count: 3,
        beat_phase: 100,
        flags: 0,
        ..frame_in(0, 300)
    };
    let strength = |frame: ControlFrame| flare(frame, section(frame));
    assert_eq!(strength(quiet), 0.0);
    assert_eq!(
        strength(ControlFrame {
            flags: 0x04,
            ..quiet
        }),
        1.0
    );
    assert_eq!(
        strength(ControlFrame {
            beat_count: 8,
            beat_phase: 0,
            ..quiet
        }),
        1.0
    );
    let reveal = frame_in(0, INTRO_FRAMES as u16);
    assert_eq!(
        strength(ControlFrame {
            frame: reveal.frame,
            ..quiet
        }),
        1.0
    );
}

#[test]
fn camcorder_tearing_only_in_camcorder_sections() {
    let mut torn = [false; 2];
    for frame in 0..(SECTION_FRAMES * 2) {
        let frame = at(frame);
        let section = section(frame);
        if tear(section, frame).is_some() {
            torn[section.overlay as usize] = true;
        }
    }
    assert_eq!(torn, [true, false]);
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn fingerprint() -> u64 {
    let mut fingerprint: u64 = 0xcbf2_9ce4_8422_2325;
    for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
        for frame in [0, 48, 200, 700, 1_300, 1_800, 2_250, 2_800, 6_000] {
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
    assert_eq!(fingerprint(), 0xbfff_534e_f332_2181);
}
