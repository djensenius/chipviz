//! Lines: thin lines travelling across a mirrored plane that folds up into a
//! hallway.
//!
//! The scene projects its own 3D camera on the CPU and emits `ScreenSpace`
//! quads (one per visible line, sorted far to near) because the shared quad
//! shader can only draw quads facing the z axis, and the hallway walls face x.
//!
//! The plane is two mirrored halves (`x >= 0` and its reflection) running the
//! same line animation. Odd sections are a hallway (each half folds up about
//! its outer edge into a wall, so the walls mirror each other) and even
//! sections are the flat plane. [`LinesState`] keeps the section between
//! frames:
//!
//! - A section starts when `beat_count` crosses a multiple of 16 (a 16-beat
//!   phrase, four bars of 4/4). Only small forward steps of the counter
//!   count; a backward or large jump (a restart, a loop or a source handover)
//!   resyncs without a section change. `beat_phase` is not used: procedural
//!   input and `live_bridge` wrap it before the count ticks.
//! - The fold eases from wherever the shape was to the section's shape over
//!   one bar (four beats at the section's `bpm`, 60 to 240 control frames),
//!   counted in observed frames, so it never moves backwards within a
//!   section. Every fourth section (`section % 4 == 3`) cuts straight to the
//!   hallway.
//! - Sources whose beat counter stalls for 4 s fall back to a new section
//!   every 512 frames (about 8.5 s).
//! - With [`SceneConfig::lines_drop_cuts`](super::SceneConfig) (`--lines-drop-cuts`),
//!   a rising drop flag (0x04) cuts straight to the next section. The cut
//!   refolds the same lines rather than flashing the screen, and any section
//!   change, cut or not, waits for one bar after the last one.
//!
//! Timings in seconds assume 60 control frames per second; the fold's real
//! speed follows the sender's frame rate. Everything else derives from the
//! control frame and elapsed time, and the state steps only on observed
//! frames, so captures replayed from frame 0 are reproducible.

use super::{CameraMode, SceneBuild, SceneEntry, SceneState, hash, note_emphasis, palette};
use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scene::Instance;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

pub const ENTRY: SceneEntry = SceneEntry {
    name: "Lines",
    camera: CameraMode::ScreenSpace,
    build: SceneBuild::Stateful(build),
};

/// A section starts when `beat_count` crosses a multiple of this.
const SECTION_BEATS: u16 = 16;
/// A forward `beat_count` step larger than this is a resync, not beats.
const MAX_BEAT_STEP: u16 = 4;
/// A forward `frame` step larger than this counts as one frame.
const MAX_FRAME_STEP: u16 = 8;
/// Frames without a beat before sections free-run.
const BEAT_STALL_FRAMES: u32 = 240;
/// Free-running section length.
const FREE_SECTION_FRAMES: u32 = 512;
/// Control frames per second assumed when converting `bpm` to frames.
const FRAME_RATE: f32 = 60.0;
const BAR_BEATS: f32 = 4.0;
/// Used when `bpm` is 0 (unknown).
const DEFAULT_BPM: f32 = 120.0;
const MIN_MORPH_FRAMES: f32 = 60.0;
const MAX_MORPH_FRAMES: f32 = 240.0;
const DROP_FLAG: u8 = 0x04;
/// Largest per-frame morph change: the smoothstep's peak slope.
#[cfg(test)]
const MAX_MORPH_STEP: f32 = 1.5 / MIN_MORPH_FRAMES;
/// The orbit swings up to this far (radians) either side of the hallway
/// entrance, so easing it to the entrance never wraps around the target.
const SWING: f32 = 2.6;
const SWING_RATE: f32 = 0.12;
/// Fraction of line slots over which lines fade in at the density cutoff.
const FADE_SHARE: f32 = 0.1;
/// Shortest projected segment (NDC) worth drawing.
const MIN_SEGMENT: f32 = 1e-4;
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

/// Fractional number of lines alive: energy scales density from 35% to
/// 100% of slots. Lines near the cutoff fade instead of popping.
pub(crate) fn active_lines(profile: &RenderProfile, frame: ControlFrame) -> f32 {
    let energy = f32::from(frame.energy) / 255.0;
    line_slots(profile) as f32 * (0.35 + 0.65 * energy)
}

/// Brightness factor for line `index`: 1 well below the cutoff, falling to 0
/// at it over `FADE_SHARE` of the slots.
pub(crate) fn line_fade(profile: &RenderProfile, active: f32, index: usize) -> f32 {
    let ramp = (line_slots(profile) as f32 * FADE_SHARE).max(1.0);
    ((active - index as f32) / ramp).clamp(0.0, 1.0)
}

/// One bar in control frames at `bpm`: how long a fold takes, and the
/// shortest gap between section changes.
fn bar_frames(bpm: u8) -> f32 {
    let bpm = if bpm == 0 {
        DEFAULT_BPM
    } else {
        f32::from(bpm)
    };
    (BAR_BEATS * 60.0 * FRAME_RATE / bpm).clamp(MIN_MORPH_FRAMES, MAX_MORPH_FRAMES)
}

/// The fields [`LinesState::observe`] compares between frames.
#[derive(Clone, Copy, Debug)]
struct Seen {
    frame: u16,
    beat_count: u16,
    flags: u8,
}

/// Lines' section and fold, kept between frames (see the module docs).
#[derive(Clone, Debug)]
pub struct LinesState {
    drop_cuts: bool,
    last: Option<Seen>,
    /// Odd sections are the hallway.
    section: u32,
    /// The section started with a cut: it shows its shape at once.
    cut: bool,
    /// Morph when the section started; the fold eases from it.
    from: f32,
    /// Frames observed since the section started.
    frames: u32,
    /// Fold length and minimum section length, from `bpm` at its start.
    bar: f32,
    /// Frames since `beat_count` last advanced.
    stalled: u32,
}

impl Default for LinesState {
    fn default() -> Self {
        Self::new(false)
    }
}

impl LinesState {
    /// Starts flat, as if the plane had been showing for a while.
    pub fn new(drop_cuts: bool) -> Self {
        Self {
            drop_cuts,
            last: None,
            section: 0,
            cut: false,
            from: 0.0,
            frames: 0,
            bar: bar_frames(0),
            stalled: 0,
        }
    }

    /// Steps on one new frame. A frame whose counter repeats the last one is
    /// ignored, like the colour field's step.
    pub fn observe(&mut self, frame: &ControlFrame) {
        let seen = Seen {
            frame: frame.frame,
            beat_count: frame.beat_count,
            flags: frame.flags,
        };
        let Some(last) = self.last else {
            self.last = Some(seen);
            return;
        };
        let step = frame.frame.wrapping_sub(last.frame);
        if step == 0 {
            return;
        }
        self.last = Some(seen);
        let frames = u32::from(if step <= MAX_FRAME_STEP { step } else { 1 });
        self.frames = self.frames.saturating_add(frames);
        let beats = frame.beat_count.wrapping_sub(last.beat_count);
        let advanced = (1..=MAX_BEAT_STEP).contains(&beats);
        self.stalled = if advanced {
            0
        } else {
            self.stalled.saturating_add(frames)
        };
        if (self.frames as f32) < self.bar {
            return;
        }
        let phrase = advanced && frame.beat_count % SECTION_BEATS < beats;
        let free_run = self.stalled >= BEAT_STALL_FRAMES && self.frames >= FREE_SECTION_FRAMES;
        let drop = self.drop_cuts && frame.flags & !last.flags & DROP_FLAG != 0;
        if phrase || free_run || drop {
            self.start_section(frame.bpm, drop);
        }
    }

    fn start_section(&mut self, bpm: u8, cut: bool) {
        self.from = self.morph();
        self.section = self.section.wrapping_add(1);
        self.cut = cut || self.section % 4 == 3;
        self.frames = 0;
        self.bar = bar_frames(bpm);
    }

    /// The current section; odd sections are the hallway.
    #[cfg(test)]
    pub fn section(&self) -> u32 {
        self.section
    }

    /// 0 = flat plane, 1 = full hallway. Within a section it only moves
    /// toward the section's shape.
    pub fn morph(&self) -> f32 {
        let target = if self.section % 2 == 1 { 1.0 } else { 0.0 };
        if self.cut {
            return target;
        }
        let t = (self.frames as f32 / self.bar).clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        self.from + (target - self.from) * eased
    }
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

/// Camera eye and target. `swing` (within `±SWING`) is the orbit's angle
/// from the hallway entrance at `-z`; the morph eases that angle, not the
/// eye position, to the entrance, so the eye never passes over the target.
fn camera_pose(swing: f32, sway: f32, energy: f32, morph: f32) -> ([f32; 3], [f32; 3]) {
    let angle = -FRAC_PI_2 + swing * (1.0 - morph);
    let radius = 16.0 + (HALF_LENGTH - 14.0) * morph;
    let height = 9.0 + energy * 2.0 + (HALF_WIDTH * 0.45 - 9.0 - energy * 2.0) * morph;
    let eye = [
        angle.cos() * radius + sway * 1.2 * morph,
        height,
        angle.sin() * radius,
    ];
    let target = [sway * -0.6, HALF_WIDTH * 0.5, HALF_LENGTH].map(|axis| axis * morph);
    (eye, target)
}

impl View {
    /// Swings around the plane, moving into the hallway as it forms.
    fn new(frame: ControlFrame, elapsed: f32, morph: f32, aspect: f32) -> Self {
        let energy = f32::from(frame.energy) / 255.0;
        let swing = SWING * (elapsed * SWING_RATE).sin();
        let (eye, target) = camera_pose(swing, (elapsed * 0.4).sin(), energy, morph);
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
        let length = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
        // Also rejects NaN: a degenerate segment has no direction for its width.
        if length.is_nan() || length < MIN_SEGMENT {
            return None;
        }
        let direction = [delta[0] / length, delta[1] / length];
        let target = (thickness * FOCAL / depth).max(MIN_THICKNESS);
        let width = ndc_width(direction, target, self.aspect);
        let fog = (1.0 - depth / 70.0).clamp(0.15, 1.0);
        Some((
            depth,
            Instance {
                center: [(pa[0] + pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5, 0.0],
                size: [length, width],
                color: [color[0] * fog, color[1] * fog, color[2] * fog, color[3]],
                rotation: delta[1].atan2(delta[0]),
                scene_mix: 3.0,
                pattern: [0.0; 2],
            },
        ))
    }
}

fn build(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    aspect: f32,
    state: &SceneState,
    instances: &mut Vec<Instance>,
) {
    build_with_morph(
        profile,
        frame,
        elapsed,
        aspect,
        state.lines.morph(),
        instances,
    );
}

fn build_with_morph(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    aspect: f32,
    morph: f32,
    instances: &mut Vec<Instance>,
) {
    let view = View::new(frame, elapsed, morph, aspect.max(0.01));
    let energy = f32::from(frame.energy) / 255.0;
    let active = active_lines(profile, frame);
    let count = (active.ceil() as usize).min(line_slots(profile));
    let mut drawn = Vec::with_capacity(2 * count);
    for index in 0..count {
        let fade = line_fade(profile, active, index);
        if fade <= 0.0 {
            continue;
        }
        let line = line_params(index, frame, elapsed);
        let half = [
            line.direction[0] * line.length * 0.5,
            line.direction[1] * line.length * 0.5,
        ];
        let (a, b) = clip_to_half(
            [line.center[0] - half[0], line.center[1] - half[1]],
            [line.center[0] + half[0], line.center[1] + half[1]],
        );
        // Fade in alpha (the pipeline alpha-blends) so a fading line stays
        // translucent instead of drawing a dark stroke over brighter ones.
        let mut color = palette(
            frame.palette.wrapping_add(line.band as u8 / 2),
            line.brightness,
            energy,
        );
        color[3] = fade;
        for side in [1.0, -1.0] {
            let start = fold(a[0], a[1], side, morph);
            let end = fold(b[0], b[1], side, morph);
            drawn.extend(view.segment(start, end, line.thickness, color));
        }
    }
    // Keep the nearest lines if over budget, then draw far to near (painter's
    // order); the stable sort keeps ties deterministic.
    drawn.sort_by(|left, right| left.0.total_cmp(&right.0));
    drawn.truncate(profile.instance_count.saturating_sub(instances.len()));
    instances.extend(drawn.into_iter().rev().map(|(_, instance)| instance));
}

/// NDC width for a screen-space quad along unit NDC `direction` whose long
/// edges are `thickness` apart on screen (NDC y units) at any angle.
///
/// The shader rotates the quad in NDC, then the viewport scales x by
/// `aspect`, turning the rectangle into a parallelogram. Its long edges stay
/// parallel to the line, but its width edge leans off the perpendicular, so
/// the gap between the long edges is `aspect * width / |S t|`, where
/// `S = diag(aspect, 1)` and `t` is `direction`. Solving for `width` makes
/// that gap exact; only the end caps stay slanted, which the quad's centre,
/// size and rotation cannot express without a shear.
pub(crate) fn ndc_width(direction: [f32; 2], thickness: f32, aspect: f32) -> f32 {
    let tangent = (direction[0] * aspect).hypot(direction[1]);
    thickness * tangent / aspect
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
    use crate::capture::parse_cvz;

    const FIXTURES: [&[u8]; 3] = [
        include_bytes!("../../../../host/fixtures/musical/groove.cvz"),
        include_bytes!("../../../../host/fixtures/musical/scale.cvz"),
        include_bytes!("../../../../host/fixtures/chipsynth/groove.cvz"),
    ];

    /// `host/bridge/live_bridge.py`'s `frame_from_bands` timing: phase 4 per
    /// frame (wrapping every 64 frames), a beat every 30 frames, 120 bpm.
    fn live(index: u64) -> ControlFrame {
        ControlFrame {
            bpm: 120,
            beat_phase: ((index * 4) & 0xFF) as u8,
            beat_count: (index / 30) as u16,
            ..ControlFrame::procedural(index)
        }
    }

    /// Each fixture played start to end `loops` times by a player that keeps
    /// counting frames and beats across loops.
    fn fixture_playback(bytes: &[u8], loops: u16) -> Vec<ControlFrame> {
        let frames = parse_cvz(bytes).unwrap();
        let beats = frames.last().unwrap().beat_count + 1;
        let length = frames.len() as u16;
        (0..loops)
            .flat_map(|round| {
                frames.iter().map(move |frame| ControlFrame {
                    frame: frame.frame.wrapping_add(round * length),
                    beat_count: frame.beat_count.wrapping_add(round * beats),
                    ..*frame
                })
            })
            .collect()
    }

    /// Steps a state through `frames`, checking that the fold only moves
    /// toward the section's shape, by at most the eased step, and that every
    /// section starts on a 16-beat boundary at least a bar after the last.
    /// Returns the number of sections started.
    fn assert_monotonic(state: &mut LinesState, frames: &[ControlFrame], source: &str) -> u32 {
        let start = state.section();
        let mut since_change = u32::MAX;
        for (index, frame) in frames.iter().enumerate() {
            let (section, before, bar) = (state.section(), state.morph(), state.bar);
            state.observe(frame);
            let after = state.morph();
            since_change = since_change.saturating_add(1);
            if state.section() != section {
                assert_eq!(state.section(), section.wrapping_add(1), "{source} {index}");
                assert_eq!(frame.beat_count % SECTION_BEATS, 0, "{source} {index}");
                assert!(since_change as f32 >= bar, "{source} {index}");
                since_change = 0;
                if state.cut {
                    continue;
                }
                // A new section starts from where the fold was.
                assert!((after - before).abs() <= MAX_MORPH_STEP + 1e-6);
                continue;
            }
            let step = after - before;
            let toward = if section % 2 == 1 { step } else { -step };
            assert!(toward >= -1e-6, "{source} frame {index}: step {step}");
            assert!(
                step.abs() <= MAX_MORPH_STEP + 1e-6,
                "{source} frame {index}: step {step}"
            );
        }
        state.section().wrapping_sub(start)
    }

    #[test]
    fn fold_is_monotonic_and_phrase_aligned_under_procedural_input() {
        // Includes the u16 frame-counter wrap at index 65536.
        for range in [0..8192u64, 60_000..70_000] {
            let frames: Vec<_> = range.map(ControlFrame::procedural).collect();
            let sections = assert_monotonic(&mut LinesState::default(), &frames, "procedural");
            assert!(sections >= 15, "{sections} sections");
        }
    }

    #[test]
    fn fold_is_monotonic_and_phrase_aligned_under_live_bridge_input() {
        let frames: Vec<_> = (0..8192).map(live).collect();
        let sections = assert_monotonic(&mut LinesState::default(), &frames, "live_bridge");
        // A phrase is 16 beats of 30 frames.
        assert_eq!(sections, 8192 / (16 * 30));
    }

    #[test]
    fn fold_is_monotonic_and_phrase_aligned_under_cvz_playback() {
        for bytes in FIXTURES {
            // Played once, and looped with continuing counters.
            let once = parse_cvz(bytes).unwrap();
            assert_monotonic(&mut LinesState::default(), &once, "cvz once");
            let looped = fixture_playback(bytes, 40);
            let sections = assert_monotonic(&mut LinesState::default(), &looped, "cvz looped");
            assert!(sections >= 4, "{sections} sections");
            // Restarting the file (counters jump back) is a resync, not a
            // section change, and never reverses a fold in progress.
            let mut state = LinesState::default();
            let restarted: Vec<_> = (0..3).flat_map(|_| looped.iter().copied()).collect();
            assert_monotonic(&mut state, &restarted, "cvz restarted");
        }
    }

    #[test]
    fn source_handover_mid_fold_keeps_folding_forward() {
        let mut state = LinesState::default();
        let procedural: Vec<_> = (0..300).map(ControlFrame::procedural).collect();
        assert_monotonic(&mut state, &procedural, "procedural");
        assert_eq!(state.section(), 1);
        let mid = state.morph();
        assert!(0.0 < mid && mid < 1.0, "{mid}");
        let fixture = fixture_playback(FIXTURES[0], 2);
        assert_monotonic(&mut state, &fixture, "handover to cvz");
        let live: Vec<_> = (1000..1400).map(live).collect();
        assert_monotonic(&mut state, &live, "handover to live_bridge");
    }

    #[test]
    fn sections_alternate_with_one_bar_folds_and_every_fourth_cuts() {
        let mut state = LinesState::default();
        let mut morph = Vec::new();
        for index in 0..1200 {
            state.observe(&ControlFrame::procedural(index));
            morph.push(state.morph());
        }
        // Procedural input: 120 bpm (one bar = 120 frames), a beat every 16
        // frames, so a 16-beat phrase starts every 256 frames.
        assert_eq!(morph[255], 0.0);
        assert!((morph[256 + 60] - 0.5).abs() < 1e-6, "{}", morph[316]);
        assert_eq!(morph[256 + 120], 1.0);
        assert_eq!(morph[511], 1.0);
        assert_eq!(morph[512 + 120], 0.0);
        // Section 3 cuts straight to the hallway.
        assert_eq!((morph[767], morph[768]), (0.0, 1.0));
        assert_eq!(morph[1024 + 120], 0.0);
        // The beat phase and drop flag (cuts off) do not move the fold.
        let mut plain = LinesState::default();
        let mut noisy = LinesState::default();
        for index in 0..600 {
            let frame = ControlFrame::procedural(index);
            plain.observe(&frame);
            noisy.observe(&ControlFrame {
                beat_phase: (index * 77) as u8,
                flags: frame.flags | if index % 7 == 0 { DROP_FLAG } else { 0 },
                ..frame
            });
            assert_eq!(plain.morph(), noisy.morph());
        }
    }

    #[test]
    fn fold_length_follows_bpm() {
        assert_eq!(bar_frames(120), 120.0);
        assert_eq!(bar_frames(0), 120.0);
        assert_eq!(bar_frames(240), MIN_MORPH_FRAMES);
        assert_eq!(bar_frames(30), 240.0);
        assert_eq!(bar_frames(10), MAX_MORPH_FRAMES);
    }

    #[test]
    fn stalled_beats_free_run_sections() {
        let mut state = LinesState::default();
        for index in 0..2048 {
            state.observe(&ControlFrame {
                beat_count: 7,
                ..ControlFrame::procedural(index)
            });
        }
        assert_eq!(state.section(), 3);
    }

    /// Procedural frames with the drop flag on `drops`.
    fn with_drops(drops: &[u64], range: std::ops::Range<u64>) -> Vec<ControlFrame> {
        range
            .map(|index| {
                let frame = ControlFrame::procedural(index);
                let drop = if drops.contains(&index) { DROP_FLAG } else { 0 };
                ControlFrame {
                    flags: frame.flags | drop,
                    ..frame
                }
            })
            .collect()
    }

    #[test]
    fn drop_cuts_are_opt_in_and_at_most_one_per_bar() {
        let frames = with_drops(&[150, 160, 200, 290, 415, 416], 0..800);
        let mut off = LinesState::new(false);
        let mut on = LinesState::new(true);
        let mut sections = Vec::new();
        for frame in &frames {
            off.observe(frame);
            on.observe(frame);
            sections.push((on.section(), on.morph()));
        }
        assert_eq!(off.section(), 3);
        // Frame 150 cuts to the hallway at once; 160 and 200 fall within
        // its bar, and so does the phrase at 256. The drop at 290 cuts back
        // to the plane and 415 cuts again (416 is not a rising edge); the
        // phrase at 512 is again within a bar, and 768 folds as usual.
        assert_eq!(sections[149], (0, 0.0));
        assert_eq!(sections[150], (1, 1.0));
        assert_eq!(sections[289].0, 1);
        assert_eq!(sections[290], (2, 0.0));
        assert_eq!(sections[415], (3, 1.0));
        assert_eq!(sections[512].0, 3);
        assert_eq!(sections[768], (4, 1.0));
        assert!(sections[769].1 < 1.0);
        let mut last_change = None;
        for index in 1..sections.len() {
            if sections[index].0 != sections[index - 1].0 {
                assert!(last_change.is_none_or(|last| index - last >= 120));
                last_change = Some(index);
            }
        }
        // A held drop flag does not repeat the cut.
        let held = with_drops(&(150..600).collect::<Vec<_>>(), 0..600);
        let mut state = LinesState::new(true);
        held.iter().for_each(|frame| state.observe(frame));
        assert_eq!(state.section(), 2);
    }

    #[test]
    fn repeated_frame_counters_do_not_step() {
        let mut state = LinesState::default();
        for index in 0..300 {
            state.observe(&ControlFrame::procedural(index));
        }
        let morph = state.morph();
        for _ in 0..50 {
            state.observe(&ControlFrame::procedural(299));
        }
        assert_eq!(state.morph(), morph);
    }

    #[test]
    fn camera_heading_changes_less_than_ten_degrees_per_frame() {
        let swing_step = SWING * SWING_RATE / 60.0;
        let forward = |swing: f32, sway: f32, energy: f32, morph: f32| {
            let (eye, target) = camera_pose(swing, sway, energy, morph);
            normalize(sub(target, eye))
        };
        let mut worst: f32 = 0.0;
        for energy in [0.0, 1.0] {
            for swing_index in 0..=64 {
                let swing = -SWING + swing_index as f32 * SWING / 32.0;
                for sway in [-1.0, 0.0, 1.0] {
                    let mut morph = 0.0;
                    while morph < 1.0 {
                        let next = (morph + MAX_MORPH_STEP).min(1.0);
                        let next_swing = (swing + swing_step).min(SWING);
                        let a = forward(swing, sway, energy, morph);
                        let b = forward(next_swing, sway + 0.4 / 60.0, energy, next);
                        worst = worst.max(dot(a, b).clamp(-1.0, 1.0).acos());
                        morph = next;
                    }
                }
            }
        }
        assert!(
            worst.to_degrees() < 10.0,
            "worst {} degrees",
            worst.to_degrees()
        );
    }

    #[test]
    fn instances_stay_finite_and_bounded() {
        let profile = RenderProfile::pi5();
        for elapsed in [0.0, 0.37, 13.1, 600.0, 5000.0] {
            for morph in [0.0, 0.25, 0.5, 0.75, 1.0] {
                for aspect in [0.5, 1.0, 16.0 / 9.0, 3.5] {
                    let frame = ControlFrame::procedural((elapsed * 60.0) as u64);
                    let mut instances = Vec::new();
                    build_with_morph(&profile, frame, elapsed, aspect, morph, &mut instances);
                    for instance in &instances {
                        let values = instance
                            .center
                            .iter()
                            .chain(&instance.size)
                            .chain(&instance.color);
                        for value in values.chain([&instance.rotation]) {
                            assert!(value.is_finite(), "{instance:?}");
                            assert!(value.abs() < 1e4, "{instance:?}");
                        }
                        assert!(instance.size[0] >= MIN_SEGMENT && instance.size[1] > 0.0);
                    }
                }
            }
        }
    }

    #[test]
    fn line_thickness_is_perpendicular_on_screen_at_any_angle_and_aspect() {
        let thickness = 0.01;
        for aspect in [1.0, 16.0 / 9.0, 3.5] {
            for degrees in [0.0f32, 30.0, 45.0, 60.0, 90.0] {
                // A line at `degrees` on screen, expressed in NDC.
                let (sin, cos) = degrees.to_radians().sin_cos();
                let delta = [cos / aspect, sin];
                let length = delta[0].hypot(delta[1]);
                let direction = [delta[0] / length, delta[1] / length];
                let width = ndc_width(direction, thickness, aspect);
                // Replay the shader: rotate the quad's edges in NDC, then
                // scale x by aspect into screen units of NDC y.
                let rotation = delta[1].atan2(delta[0]);
                let (rs, rc) = rotation.sin_cos();
                let long = [rc * aspect, rs];
                let side = [-rs * width * aspect, rc * width];
                let gap = (long[0] * side[1] - long[1] * side[0]).abs() / long[0].hypot(long[1]);
                let error = (gap / thickness - 1.0).abs();
                assert!(error < 0.02, "aspect {aspect} angle {degrees}: gap {gap}");
            }
        }
    }

    #[test]
    fn zero_length_segments_are_skipped() {
        let view = View::new(ControlFrame::procedural(0), 1.0, 0.0, 16.0 / 9.0);
        let point = [1.0, 0.0, 2.0];
        assert!(view.segment(point, point, 0.05, [1.0; 4]).is_none());
        // A segment along the ray from the eye also projects to one point.
        let toward = add_scaled(point, sub(point, view.eye), 0.5);
        assert!(view.segment(point, toward, 0.05, [1.0; 4]).is_none());
        let across = add_scaled(point, view.right, 0.5);
        assert!(view.segment(point, across, 0.05, [1.0; 4]).is_some());
    }

    fn add_scaled(point: [f32; 3], direction: [f32; 3], scale: f32) -> [f32; 3] {
        std::array::from_fn(|axis| point[axis] + direction[axis] * scale)
    }

    #[test]
    fn overflow_keeps_the_nearest_lines() {
        let profile = RenderProfile::pi5();
        let frame = ControlFrame::procedural(400);
        let mut full = Vec::new();
        build_with_morph(&profile, frame, 6.0, 1.5, 1.0, &mut full);
        let filler = profile.instance_count - 10;
        let mut tight = vec![full[0]; filler];
        build_with_morph(&profile, frame, 6.0, 1.5, 1.0, &mut tight);
        let near = &full[full.len() - 10..];
        assert_eq!(
            bytemuck::cast_slice::<Instance, u8>(&tight[filler..]),
            bytemuck::cast_slice::<Instance, u8>(near)
        );
    }

    #[test]
    fn lines_fade_at_the_density_cutoff() {
        let profile = RenderProfile::m1();
        let active = 1000.5;
        assert_eq!(line_fade(&profile, active, 0), 1.0);
        assert_eq!(line_fade(&profile, active, 1001), 0.0);
        let fades: Vec<_> = (700..1001)
            .map(|index| line_fade(&profile, active, index))
            .collect();
        assert!(fades.windows(2).all(|pair| pair[1] <= pair[0]));
        // One energy step moves any line's fade by only a small amount.
        let frame = ControlFrame::procedural(0);
        let louder = ControlFrame {
            energy: frame.energy + 1,
            ..frame
        };
        let (low, high) = (
            active_lines(&profile, frame),
            active_lines(&profile, louder),
        );
        for index in 0..line_slots(&profile) {
            let change = line_fade(&profile, high, index) - line_fade(&profile, low, index);
            assert!((0.0..0.05).contains(&change), "line {index}: {change}");
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
            let mut state = SceneState::default();
            let mut observed = 0;
            for index in [0, 120, 288, 400, 800] {
                let frame = ControlFrame::procedural(index);
                while observed <= index {
                    state.observe(&ControlFrame::procedural(observed));
                    observed += 1;
                }
                let elapsed = index as f32 / 60.0;
                let mut first = Vec::new();
                build(&profile, frame, elapsed, 16.0 / 9.0, &state, &mut first);
                let mut second = Vec::new();
                build(&profile, frame, elapsed, 16.0 / 9.0, &state, &mut second);
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
        assert_eq!(active_lines(&profile, loud), line_slots(&profile) as f32);
    }
}
