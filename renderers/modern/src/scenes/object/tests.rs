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
    for kind in Kind::ALL {
        assert!(
            sections.iter().take(6).any(|s| s.object == kind),
            "{kind:?}"
        );
    }
    for background in Background::ALL {
        assert!(sections.iter().take(5).any(|s| s.background == background));
    }
    for morph in [Morph::Cycle, Morph::Sweep, Morph::BeatJump] {
        assert!(sections.iter().take(6).any(|s| s.morph == morph));
    }
    for overlay in [Overlay::Camcorder, Overlay::Infomercial] {
        assert!(sections.iter().take(2).any(|s| s.overlay == overlay));
    }
    // Every object meets every background within 30 sections.
    for kind in Kind::ALL {
        for background in Background::ALL {
            assert!(
                sections
                    .iter()
                    .any(|s| s.object == kind && s.background == background),
                "{kind:?} on {background:?}"
            );
        }
    }
    // Each object is shown under both overlays within the first 12 sections.
    for kind in Kind::ALL {
        for overlay in [Overlay::Camcorder, Overlay::Infomercial] {
            assert!(
                sections
                    .iter()
                    .take(12)
                    .any(|s| s.object == kind && s.overlay == overlay),
                "{kind:?} under {overlay:?}"
            );
        }
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
    for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
        for aspect in [WIDE, 1.0, 9.0 / 16.0, 21.0 / 9.0] {
            for index in 0..12 {
                for local in [0, 30, 48, 200, 490] {
                    for frame in [
                        frame_in(index, local),
                        ControlFrame {
                            energy: 255,
                            flags: 0x07,
                            ..frame_in(index, local)
                        },
                    ] {
                        let instances = build_frame(&profile, frame, aspect);
                        assert!(
                            instances.len() <= profile.instance_count,
                            "{} section {index} local {local}: {}",
                            profile.name,
                            instances.len()
                        );
                        assert!(instances.iter().all(|instance| {
                            instance.center.iter().all(|value| value.is_finite())
                                && instance.size.iter().all(|value| value.is_finite())
                                && instance.color.iter().all(|value| value.is_finite())
                                && instance.rotation.is_finite()
                        }));
                    }
                }
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
        let m1 = draw_object(section, frame, pose, WIDE, 12_000).len();
        let pi5 = draw_object(section, frame, pose, WIDE, 2_500).len();
        assert!(pi5 > 500, "{:?} pi5 {pi5}", section.object);
        assert!(m1 > 2 * pi5, "{:?} m1 {m1} pi5 {pi5}", section.object);
        assert!(m1 <= 12_000 && pi5 <= 2_500);
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
fn flare_fires_on_drop_and_reveal_only() {
    let quiet = ControlFrame {
        beat_count: 3,
        beat_phase: 100,
        flags: 0,
        ..frame_in(0, 300)
    };
    assert_eq!(flare(quiet, section(quiet)), 0.0);
    let drop = ControlFrame {
        flags: 0x04,
        ..quiet
    };
    assert_eq!(flare(drop, section(drop)), 1.0);
    let reveal = ControlFrame {
        beat_count: 3,
        beat_phase: 100,
        flags: 0,
        ..frame_in(0, INTRO_FRAMES as u16)
    };
    assert_eq!(flare(reveal, section(reveal)), 1.0);
    let downbeat = ControlFrame {
        beat_count: 8,
        beat_phase: 0,
        ..quiet
    };
    assert_eq!(flare(downbeat, section(downbeat)), 1.0);
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
    assert_eq!(fingerprint(), 0xcfae_60e8_a2aa_3035);
}
