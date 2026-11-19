//! Lines: thin lines travelling across a mirrored plane that folds up into a
//! hallway.
//!
//! The scene projects its own 3D camera on the CPU and emits `ScreenSpace`
//! quads (one per visible line, sorted far to near) because the shared quad
//! shader can only draw quads facing the z axis, and the hallway walls face x.
//!
//! The plane is two mirrored halves (`x >= 0` and its reflection) running the
//! same line animation. Every 16 beats a section starts: odd sections are a
//! hallway (each half folds up about its outer edge into a wall, so the walls
//! mirror each other) and even sections are the flat plane. A section change
//! morphs smoothly over its first 4 beats, except that every fourth section
//! (`section % 4 == 3`) and any frame with the drop flag cut straight to the
//! section's shape. Everything derives from the control frame and elapsed
//! time, so captures are reproducible.

use super::{CameraMode, SceneEntry, hash, note_emphasis, palette};
use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scene::Instance;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

pub const ENTRY: SceneEntry = SceneEntry {
    name: "Lines",
    camera: CameraMode::ScreenSpace,
    build,
};

const FLAG_DROP: u8 = 0x04;
const SECTION_BEATS: u16 = 16;
const MORPH_BEATS: f32 = 4.0;
/// Width of one plane half, which is also the hallway wall height.
const HALF_WIDTH: f32 = 5.0;
/// Half the plane's length along z.
const HALF_LENGTH: f32 = 20.0;
const NEAR: f32 = 0.1;
/// 1 / tan(fov / 2) for a 60 degree vertical field of view.
const FOCAL: f32 = 1.732_050_8;
/// Minimum on-screen thickness in NDC y units, so far lines stay visible.
const MIN_THICKNESS: f32 = 0.0018;

/// Line slots per mirrored half: an eighth of the instance budget, so the two
/// halves together use at most a quarter and leave visible gaps.
pub(crate) fn line_slots(profile: &RenderProfile) -> usize {
    profile.instance_count / 8
}

/// Lines alive this frame: energy scales density from 35% to 100% of slots.
pub(crate) fn active_lines(profile: &RenderProfile, frame: ControlFrame) -> usize {
    let energy = f32::from(frame.energy) / 255.0;
    (line_slots(profile) as f32 * (0.35 + 0.65 * energy)) as usize
}

/// 0 = flat plane, 1 = full hallway.
pub(crate) fn hallway_morph(frame: ControlFrame) -> f32 {
    let section = frame.beat_count / SECTION_BEATS;
    let target = if section % 2 == 1 { 1.0 } else { 0.0 };
    if frame.flags & FLAG_DROP != 0 || section % 4 == 3 {
        return target;
    }
    let beats = f32::from(frame.beat_count % SECTION_BEATS) + f32::from(frame.beat_phase) / 256.0;
    let t = (beats / MORPH_BEATS).clamp(0.0, 1.0);
    let eased = t * t * (3.0 - 2.0 * t);
    (1.0 - target) + (2.0 * target - 1.0) * eased
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LineParams {
    /// Centre on one plane half: `u` in `0..HALF_WIDTH` (0 is the mirror
    /// seam), `v` in `-HALF_LENGTH..HALF_LENGTH`.
    pub center: [f32; 2],
    /// Unit travel direction; lines are drawn along it.
    pub direction: [f32; 2],
    pub length: f32,
    pub thickness: f32,
    pub brightness: f32,
    pub band: usize,
}

/// Line `index`'s parameters. Each line belongs to spectrum band
/// `index % 8`: low bands are slow, long and thick, high bands fast, short
/// and thin. A band's level stretches, thickens and brightens its lines and
/// the beat stretches them further; travel speed is fixed per line so motion
/// stays continuous while the audio changes.
pub(crate) fn line_params(index: usize, frame: ControlFrame, elapsed: f32) -> LineParams {
    let seed = index as u32 * 4;
    let band = index % frame.spectrum.len();
    let level = f32::from(frame.spectrum[band]) / 255.0;
    let high = band as f32 / (frame.spectrum.len() - 1) as f32;
    let angle = (hash(seed) * 8.0).floor() * FRAC_PI_4;
    let direction = [angle.cos(), angle.sin()];
    let speed = (0.4 + high * 2.6) * (0.6 + hash(seed + 1) * 0.8);
    let travel = speed * elapsed;
    let wrap = |start: f32, step: f32, span: f32| (start + step / span).rem_euclid(1.0) * span;
    let u = wrap(hash(seed + 2), direction[0] * travel, HALF_WIDTH);
    let v = wrap(hash(seed + 3), direction[1] * travel, 2.0 * HALF_LENGTH) - HALF_LENGTH;
    let beat = 1.0 - f32::from(frame.beat_phase) / 255.0;
    let length =
        (1.6 - high * 1.2) * (0.5 + hash(seed + 1) * 0.5) * (0.6 + level + beat * beat * 0.4);
    let thickness = (0.07 - high * 0.045) * (0.7 + level * 0.6);
    let brightness = 0.25 + level * 0.65 + note_emphasis(frame) * 0.1;
    LineParams {
        center: [u, v],
        direction,
        length,
        thickness,
        brightness,
        band,
    }
}

/// Maps a plane-half point to world space for half `side` (+1 or -1).
/// Morph 0 lays it flat at `x = side * u`; morph 1 folds it up about
/// `x = side * HALF_WIDTH` into a wall, mirrored across `x = 0`.
pub(crate) fn fold(u: f32, v: f32, side: f32, morph: f32) -> [f32; 3] {
    let (sin, cos) = (morph * FRAC_PI_2).sin_cos();
    [
        side * (HALF_WIDTH + (u - HALF_WIDTH) * cos),
        (HALF_WIDTH - u) * sin,
        v,
    ]
}

struct View {
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
    aspect: f32,
}

impl View {
    /// Orbits above the plane, moving into the hallway as it forms.
    fn new(frame: ControlFrame, elapsed: f32, morph: f32, aspect: f32) -> Self {
        let energy = f32::from(frame.energy) / 255.0;
        let angle = elapsed * 0.15;
        let orbit = [angle.cos() * 16.0, 9.0 + energy * 2.0, angle.sin() * 16.0];
        let sway = (elapsed * 0.4).sin();
        let hall = [sway * 1.2, HALF_WIDTH * 0.45, -HALF_LENGTH - 2.0];
        let hall_target = [sway * -0.6, HALF_WIDTH * 0.5, HALF_LENGTH];
        let ease = morph * morph * (3.0 - 2.0 * morph);
        let eye = lerp3(orbit, hall, ease);
        let target = lerp3([0.0; 3], hall_target, ease);
        let forward = normalize(sub(target, eye));
        let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
        Self {
            eye,
            right,
            up: cross(right, forward),
            forward,
            aspect,
        }
    }

    /// (x, y, depth) in camera space.
    fn camera(&self, point: [f32; 3]) -> [f32; 3] {
        let offset = sub(point, self.eye);
        [
            dot(offset, self.right),
            dot(offset, self.up),
            dot(offset, self.forward),
        ]
    }

    fn project(&self, point: [f32; 3]) -> [f32; 2] {
        [
            point[0] * FOCAL / (self.aspect * point[2]),
            point[1] * FOCAL / point[2],
        ]
    }

    /// Screen-space quad for a world segment, clipped to the near plane.
    /// Returns (depth, instance) or `None` when behind the camera or off screen.
    fn segment(
        &self,
        a: [f32; 3],
        b: [f32; 3],
        thickness: f32,
        color: [f32; 4],
    ) -> Option<(f32, Instance)> {
        let (mut a, mut b) = (self.camera(a), self.camera(b));
        if a[2] < NEAR && b[2] < NEAR {
            return None;
        }
        if a[2] < NEAR || b[2] < NEAR {
            let t = (NEAR - a[2]) / (b[2] - a[2]);
            let clipped = lerp3(a, b, t);
            if a[2] < NEAR {
                a = clipped;
            } else {
                b = clipped;
            }
        }
        let (pa, pb) = (self.project(a), self.project(b));
        if pa[0].max(pb[0]) < -1.1
            || pa[0].min(pb[0]) > 1.1
            || pa[1].max(pb[1]) < -1.1
            || pa[1].min(pb[1]) > 1.1
        {
            return None;
        }
        let depth = (a[2] + b[2]) * 0.5;
        let delta = [pb[0] - pa[0], pb[1] - pa[1]];
        let length = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt().max(1e-6);
        // Keep the on-screen width even though NDC x and y scale differently.
        let normal = [-delta[1] / length * self.aspect, delta[0] / length];
        let stretch = (normal[0] * normal[0] + normal[1] * normal[1]).sqrt();
        let width = (thickness * FOCAL / depth).max(MIN_THICKNESS) / stretch;
        let fog = (1.0 - depth / 70.0).clamp(0.15, 1.0);
        Some((
            depth,
            Instance {
                center: [(pa[0] + pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5, 0.0],
                size: [length, width],
                color: [color[0] * fog, color[1] * fog, color[2] * fog, color[3]],
                rotation: delta[1].atan2(delta[0]),
                scene_mix: 3.0,
                _padding: [0.0; 2],
            },
        ))
    }
}

fn build(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    aspect: f32,
    instances: &mut Vec<Instance>,
) {
    let morph = hallway_morph(frame);
    let view = View::new(frame, elapsed, morph, aspect.max(0.01));
    let energy = f32::from(frame.energy) / 255.0;
    let mut drawn = Vec::with_capacity(2 * active_lines(profile, frame));
    for index in 0..active_lines(profile, frame) {
        let line = line_params(index, frame, elapsed);
        let half = [
            line.direction[0] * line.length * 0.5,
            line.direction[1] * line.length * 0.5,
        ];
        let (a, b) = clip_to_half(
            [line.center[0] - half[0], line.center[1] - half[1]],
            [line.center[0] + half[0], line.center[1] + half[1]],
        );
        let color = palette(
            frame.palette.wrapping_add(line.band as u8 / 2),
            line.brightness,
            energy,
        );
        for side in [1.0, -1.0] {
            let start = fold(a[0], a[1], side, morph);
            let end = fold(b[0], b[1], side, morph);
            drawn.extend(view.segment(start, end, line.thickness, color));
        }
    }
    // Painter's order: far lines first; stable sort keeps ties deterministic.
    drawn.sort_by(|left, right| right.0.total_cmp(&left.0));
    let room = profile.instance_count.saturating_sub(instances.len());
    instances.extend(drawn.into_iter().take(room).map(|(_, instance)| instance));
}

/// Clips a segment's `u` to the half's `0..HALF_WIDTH` so lines never cross
/// the mirror seam or the fold edge.
fn clip_to_half(a: [f32; 2], b: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    let clamp = |point: [f32; 2], other: [f32; 2]| {
        let target = point[0].clamp(0.0, HALF_WIDTH);
        if target == point[0] || other[0] == point[0] {
            return [target, point[1]];
        }
        let t = (target - point[0]) / (other[0] - point[0]);
        [target, point[1] + (other[1] - point[1]) * t]
    };
    (clamp(a, b), clamp(b, a))
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] + (b[axis] - a[axis]) * t)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(value: [f32; 3]) -> [f32; 3] {
    let length = dot(value, value).sqrt().max(1e-6);
    value.map(|component| component / length)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(beat_count: u16, beat_phase: u8, flags: u8) -> ControlFrame {
        ControlFrame {
            beat_count,
            beat_phase,
            flags,
            ..ControlFrame::procedural(0)
        }
    }

    #[test]
    fn hallway_alternates_by_section_with_smooth_and_cut_transitions() {
        assert_eq!(hallway_morph(at(5, 0, 0)), 0.0);
        assert_eq!(hallway_morph(at(16, 0, 0)), 0.0);
        let mid = hallway_morph(at(18, 0, 0));
        assert!(mid > 0.4 && mid < 0.6, "{mid}");
        assert_eq!(hallway_morph(at(20, 0, 0)), 1.0);
        assert_eq!(hallway_morph(at(32, 0, 0)), 1.0);
        assert_eq!(hallway_morph(at(36, 0, 0)), 0.0);
        // Every fourth section cuts; a drop always cuts to the section shape.
        assert_eq!(hallway_morph(at(48, 0, 0)), 1.0);
        assert_eq!(hallway_morph(at(16, 0, FLAG_DROP)), 1.0);
        assert_eq!(hallway_morph(at(32, 0, FLAG_DROP)), 0.0);
    }

    #[test]
    fn smooth_morph_is_monotonic_within_ramp() {
        let mut previous = 0.0;
        for beat in 16..20 {
            for phase in (0..=255).step_by(15) {
                let morph = hallway_morph(at(beat, phase, 0));
                assert!(morph >= previous, "beat {beat} phase {phase}");
                previous = morph;
            }
        }
    }

    #[test]
    fn line_params_follow_band_and_stay_on_the_half() {
        let frame = ControlFrame {
            spectrum: [255, 0, 0, 0, 0, 0, 0, 0],
            ..ControlFrame::procedural(40)
        };
        for index in 0..512 {
            let line = line_params(index, frame, 37.5);
            assert_eq!(line.band, index % 8);
            assert!((0.0..=HALF_WIDTH).contains(&line.center[0]));
            assert!((-HALF_LENGTH..=HALF_LENGTH).contains(&line.center[1]));
            let norm = line.direction[0].hypot(line.direction[1]);
            assert!((norm - 1.0).abs() < 1e-5);
            assert!(line.length > 0.0 && line.thickness > 0.0);
        }
        // A loud band draws longer, thicker, brighter lines than a quiet one.
        let quiet = ControlFrame {
            spectrum: [0; 8],
            ..frame
        };
        let (loud, soft) = (line_params(8, frame, 1.0), line_params(8, quiet, 1.0));
        assert!(loud.length > soft.length && loud.thickness > soft.thickness);
        assert!(loud.brightness > soft.brightness);
        assert_eq!(line_params(8, frame, 1.0), loud);
    }

    #[test]
    fn fold_lays_flat_then_forms_mirrored_walls() {
        for (u, v) in [(0.0, -3.0), (2.5, 7.0), (HALF_WIDTH, 0.0)] {
            assert_eq!(fold(u, v, 1.0, 0.0), [u, 0.0, v]);
            assert_eq!(fold(u, v, -1.0, 0.0), [-u, 0.0, v]);
            let right = fold(u, v, 1.0, 1.0);
            let left = fold(u, v, -1.0, 1.0);
            assert!((right[0] - HALF_WIDTH).abs() < 1e-5);
            assert!((right[1] - (HALF_WIDTH - u)).abs() < 1e-5);
            assert_eq!(left, [-right[0], right[1], right[2]]);
        }
    }

    #[test]
    fn instances_respect_profile_budget_and_are_deterministic() {
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            for index in [0, 120, 288, 400, 800] {
                let frame = ControlFrame::procedural(index);
                let elapsed = index as f32 / 60.0;
                let mut first = Vec::new();
                build(&profile, frame, elapsed, 16.0 / 9.0, &mut first);
                let mut second = Vec::new();
                build(&profile, frame, elapsed, 16.0 / 9.0, &mut second);
                assert!(!first.is_empty() && first.len() <= 2 * line_slots(&profile));
                assert!(first.len() <= profile.instance_count);
                assert_eq!(
                    bytemuck::cast_slice::<Instance, u8>(&first),
                    bytemuck::cast_slice::<Instance, u8>(&second)
                );
            }
        }
        assert!(line_slots(&RenderProfile::m1()) > line_slots(&RenderProfile::pi5()));
        let quiet = ControlFrame {
            energy: 0,
            ..ControlFrame::procedural(0)
        };
        let loud = ControlFrame {
            energy: 255,
            ..quiet
        };
        let profile = RenderProfile::m1();
        assert!(active_lines(&profile, quiet) < active_lines(&profile, loud));
        assert_eq!(active_lines(&profile, loud), line_slots(&profile));
    }
}
