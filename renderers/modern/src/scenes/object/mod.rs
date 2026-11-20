//! Object: an object in space, sold like a late-night 1980s/90s infomercial
//! and taped on an early VHS camcorder (issue #17, phases A and B).
//!
//! One procedural object at a time swings on a turntable (or rocks, for the
//! flat CD and the sun's face) in front of a cheesy background. The eleven
//! objects are original code-built models (`models.rs`): a CRT television,
//! a VHS tape, a CD, the sun, a tree and a 1980s home computer, then (added
//! in phase B, appended so the first six sections are unchanged) a red
//! phone box, a tricycle, a milk carton, a jukebox and a tin of beans. No
//! assets are downloaded. Each model is a cloud of surface splats that the
//! scene rotates, lights (diffuse, specular and a colour-cycling rim light),
//! projects on the CPU and draws far to near as flat `ScreenSpace` quads,
//! as Lines and Static do. The splat spacing follows the instance budget,
//! so the Pi 5 draws the same models with fewer, larger splats.
//!
//! Timing follows the monotonic `frame` counter, so the scene is stateless
//! and captures are reproducible. Every 512 frames (about 8.5 s at 60 fps)
//! a section starts: section `n` shows object `n % 11` on background `n % 5`
//! (starfield, lava goop, vaporwave grid, rainbow sweep, chroma-key colour
//! cycle), so all 55 pairs appear every 55 sections. The colour morph
//! (hue cycle, rainbow sweep, hue jump per beat) follows `n % 3` and the
//! overlay (camcorder or infomercial) `n % 2`; 11 is coprime with both, so
//! each object meets each of them.
//! The object spins in over the first 48 frames with a gleam, and spins out
//! over the last 32.
//!
//! Audio: each beat punches the object's zoom (decaying over `beat_phase`);
//! the spectrum drives the TV's colour bars, the computer's bar chart and
//! the jukebox grille, mids sway the tree, bass stretches the sun's rays
//! and the goop, treble slides the CD's sheen, flutters the tricycle's
//! streamers and twinkles the stars, beats light the phone box and hop the
//! beans, and energy adds sparkles and speeds the tape reels and the
//! tricycle's wheels. The drop flag (and every eighth beat) fires a
//! lens flare and extra sparkles; the drop is a one-frame trigger, so on
//! its own the flare lasts one frame.
//!
//! The camcorder overlay draws scanlines, a rolling tracking bar of pixel
//! noise (Static's `pixel_noise`), a "REC" light, a date and a running
//! clock in Static's 5x8 font, and now and then tears the object into
//! horizontally jittered bands. The infomercial overlay draws a caption
//! banner and a spinning "NEW!" starburst.
//!
//! Flash safety: no effect inverts or flips the whole screen's brightness.
//! The chroma-key background cuts between colours of equal luminance at
//! most once every 32 frames (under 2 per second at 60 fps), section cuts
//! come every 8.5 s, and beat effects only touch the object, sparkles and
//! the flare, never the full frame.

mod backdrop;
mod models;
#[cfg(test)]
mod tests;

use super::{CameraMode, SceneBuild, SceneEntry, hash, sin_cos};
use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scene::Instance;
use backdrop::Stage;
use models::{Kind, Look, Mesh, Splat, Vec3, hue};
use std::f32::consts::{PI, TAU};
use std::sync::Mutex;

pub const ENTRY: SceneEntry = SceneEntry {
    name: "Object",
    camera: CameraMode::ScreenSpace,
    build: SceneBuild::Stateless(build),
};

/// Divides the u16 frame counter's wrap.
const SECTION_FRAMES: u16 = 512;
const INTRO_FRAMES: f32 = 48.0;
const OUTRO_FRAMES: f32 = 32.0;
/// Instances kept back for sun rays, sparkles, the flare and the overlays.
const OVERLAY_RESERVE: usize = 320;
/// The background may use `instance_count / BACKGROUND_SHARE` instances.
const BACKGROUND_SHARE: usize = 16;
/// Splats generated per drawable instance; about half face away.
const OVERSAMPLE: f32 = 1.8;
/// 1 / tan(fov / 2) for a 60 degree vertical field of view.
const FOCAL: f32 = 1.732_050_8;
const DISTANCE: f32 = 3.7;
const NEAR: f32 = 0.1;
const SCENE_MIX: f32 = 5.0;
/// Splats overlap their neighbours by this factor to close seams.
const OVERLAP: f32 = 1.12;
const LIGHT: Vec3 = [-0.45, 0.65, 0.62];

/// Shader shapes for `pattern = [0, -(1 + shape)]`; see `shape_alpha` in
/// `shader.wgsl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Disc,
    Glow,
    Sparkle,
    Scanlines,
}

impl Shape {
    fn pattern(self) -> [f32; 2] {
        [0.0, -1.0 - self as u8 as f32]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Background {
    Stars,
    Goop,
    Vaporwave,
    Rainbow,
    ChromaKey,
}

impl Background {
    const ALL: [Self; 5] = [
        Self::Stars,
        Self::Goop,
        Self::Vaporwave,
        Self::Rainbow,
        Self::ChromaKey,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Morph {
    /// The whole palette swings around the colour wheel.
    Cycle,
    /// A rainbow sweeps across the object.
    Sweep,
    /// The palette jumps a quarter turn on every beat.
    BeatJump,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Overlay {
    Camcorder,
    Infomercial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Section {
    index: u16,
    local: u16,
    object: Kind,
    background: Background,
    morph: Morph,
    overlay: Overlay,
}

fn section(frame: ControlFrame) -> Section {
    let index = frame.frame / SECTION_FRAMES;
    Section {
        index,
        local: frame.frame % SECTION_FRAMES,
        object: Kind::ALL[usize::from(index) % Kind::ALL.len()],
        background: Background::ALL[usize::from(index % 5)],
        morph: [Morph::Cycle, Morph::Sweep, Morph::BeatJump][usize::from(index % 3)],
        overlay: [Overlay::Camcorder, Overlay::Infomercial][usize::from(index % 2)],
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 1 at a beat onset, easing to 0 across the beat.
fn punch(frame: ControlFrame) -> f32 {
    (1.0 - f32::from(frame.beat_phase) / 255.0).powi(4)
}

/// Lens-flare strength: the drop flag, every eighth beat, and the reveal
/// at the end of the intro.
fn flare(frame: ControlFrame, section: Section) -> f32 {
    let drop = f32::from(u8::from(frame.flags & 0x04 != 0));
    let fade = 1.0 - f32::from(frame.beat_phase) / 255.0;
    let downbeat = fade * fade * f32::from(u8::from(frame.beat_count.is_multiple_of(8)));
    let reveal = 1.0 - (f32::from(section.local) - INTRO_FRAMES).abs() / 24.0;
    f32::max(drop, downbeat).max(reveal.clamp(0.0, 1.0))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pose {
    yaw: f32,
    pitch: f32,
    scale: f32,
    /// NDC y of the object's centre.
    lift: f32,
}

fn pose(section: Section, frame: ControlFrame) -> Pose {
    let t = f32::from(section.local);
    let intro = smoothstep(t / INTRO_FRAMES);
    let outro = smoothstep((f32::from(SECTION_FRAMES) - t) / OUTRO_FRAMES);
    let turn = if section.object.rocks() {
        0.65 * (t * 0.02).sin()
    } else {
        // A wide turntable swing (+-63 degrees) around a three-quarter front
        // view; the intro and outro spins show the back.
        -0.45 + 1.1 * (TAU * (t - INTRO_FRAMES) / 300.0).sin()
    };
    let spin = (1.0 - intro) * 2.0 * TAU - (1.0 - outro) * 1.5 * TAU;
    Pose {
        yaw: turn + spin,
        pitch: 0.26 + 0.06 * (t * 0.013).sin(),
        scale: section.object.size() * intro.min(outro) * (1.0 + 0.12 * punch(frame)),
        lift: 0.1 + 0.03 * (t * 0.045).sin(),
    }
}

/// The coarsest grid step whose splat count fits `target`. Counts depend
/// only on the model and the step (`Look` only paints and poses), so the
/// search runs once per object and target, with a fixed reference look,
/// and is cached.
fn step_for(kind: Kind, target: usize) -> f32 {
    static CACHE: Mutex<Vec<((Kind, usize), f32)>> = Mutex::new(Vec::new());
    let key = (kind, target);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(&(_, step)) = cache.iter().find(|(entry, _)| *entry == key) {
        return step;
    }
    let look = Look::default();
    let count = |step: f32| {
        let mut mesh = Mesh::new(step);
        kind.build(&mut mesh, &look);
        mesh.splats.len()
    };
    let probe_step = 0.05;
    let ratio = count(probe_step).max(1) as f32 / target.max(1) as f32;
    let mut step = (probe_step * ratio.sqrt()).max(1e-3);
    while step <= 1.0 && count(step) > target {
        step *= 1.05;
    }
    if cache.len() >= 64 {
        cache.clear();
    }
    cache.push((key, step));
    step
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: Vec3) -> Vec3 {
    let length = dot(v, v).sqrt().max(1e-6);
    v.map(|axis| axis / length)
}

/// Rotates `color` around the grey axis, then restores its Rec. 709
/// luminance so hue changes (including beat jumps) never flash: scaled
/// down when brighter, or lifted towards white (exactly, as luminance is
/// linear) when clamping leaves it darker.
fn turn_hue(color: Vec3, angle: f32) -> Vec3 {
    let (s, c) = sin_cos(angle);
    let k = 1.0 / 3.0_f32.sqrt();
    let along = (color[0] + color[1] + color[2]) * k * k * (1.0 - c);
    let cross = [
        k * (color[2] - color[1]),
        k * (color[0] - color[2]),
        k * (color[1] - color[0]),
    ];
    let turned: Vec3 =
        std::array::from_fn(|i| (color[i] * c + cross[i] * s + along).clamp(0.0, 1.0));
    let target = backdrop::luminance(color);
    let now = backdrop::luminance(turned);
    if now >= target {
        let gain = if now > 1e-6 { target / now } else { 0.0 };
        return turned.map(|channel| channel * gain);
    }
    let headroom = backdrop::luminance(turned.map(|channel| 1.0 - channel));
    let lift = if headroom > 1e-6 {
        ((target - now) / headroom).min(1.0)
    } else {
        0.0
    };
    turned.map(|channel| channel + (1.0 - channel) * lift)
}

fn morph(section: Section, frame: ControlFrame, splat: &Splat) -> Vec3 {
    let t = f32::from(section.local);
    let color = splat.surface.color;
    if splat.surface.glow > 0.5 {
        // Screens and lamps keep their own colours so the object still reads.
        return color;
    }
    match section.morph {
        // Swings a third of the way round the wheel and back, so the
        // object passes through its true colours every four seconds.
        Morph::Cycle => turn_hue(color, TAU / 3.0 * (TAU * t / 240.0).sin()),
        Morph::Sweep => {
            let luma = 0.3 * color[0] + 0.59 * color[1] + 0.11 * color[2];
            let band = hue(splat.pos[1] * 0.6 + splat.pos[0] * 0.25 - t / 90.0);
            let strength = (0.3 + 1.3 * luma).min(1.0);
            std::array::from_fn(|i| color[i] * 0.35 + band[i] * strength * 0.65)
        }
        Morph::BeatJump => turn_hue(color, f32::from(frame.beat_count % 4) * 0.25 * TAU),
    }
}

/// A projected splat: depth for sorting and the instance to draw.
#[derive(Clone, Copy, Debug)]
struct Drawn {
    depth: f32,
    instance: Instance,
}

fn quad(
    center: [f32; 2],
    size: [f32; 2],
    color: [f32; 4],
    rotation: f32,
    pattern: [f32; 2],
) -> Instance {
    Instance {
        center: [center[0], center[1], 0.0],
        size,
        color,
        rotation,
        scene_mix: SCENE_MIX,
        pattern,
    }
}

/// Horizontal tearing of the camcorder overlay: an NDC x offset per band of
/// screen rows, or `None` outside a glitch.
fn tear(section: Section, frame: ControlFrame) -> Option<impl Fn(f32) -> f32> {
    if section.overlay != Overlay::Camcorder {
        return None;
    }
    let window = u32::from(frame.frame / 20);
    if hash(window ^ 0x5bd1_e995) > 0.3 {
        return None;
    }
    let tick = u32::from(frame.frame / 2);
    Some(move |y: f32| {
        let band = ((y + 1.0) * 24.0).max(0.0) as u32;
        (hash(band.wrapping_mul(0x9e37_79b9) ^ tick) - 0.5) * 0.06
    })
}

/// Rotates, lights and projects the object, far to near, keeping at most
/// `budget` splats (the farthest go first).
fn draw_object(
    section: Section,
    frame: ControlFrame,
    pose: Pose,
    aspect: f32,
    budget: usize,
) -> Vec<Drawn> {
    if pose.scale < 1e-3 || budget == 0 {
        return Vec::new();
    }
    let t = f32::from(section.local);
    let spectrum = frame.spectrum.map(|band| f32::from(band) / 255.0);
    let look = Look {
        t,
        spectrum,
        mid: f32::from(frame.mid) / 255.0,
        treble: f32::from(frame.treble) / 255.0,
        energy: f32::from(frame.energy) / 255.0,
        beat: punch(frame),
        yaw: pose.yaw,
    };
    let target = (budget as f32 * OVERSAMPLE) as usize;
    let mut mesh = Mesh::new(step_for(section.object, target));
    section.object.build(&mut mesh, &look);
    let (sy, cy) = sin_cos(pose.yaw);
    let (sp, cp) = sin_cos(pose.pitch);
    let rotate = |v: Vec3| {
        let (x, z) = (v[0] * cy + v[2] * sy, v[2] * cy - v[0] * sy);
        [x, v[1] * cp - z * sp, v[1] * sp + z * cp]
    };
    let light = normalize(LIGHT);
    let rim = hue(t / 200.0 + 0.8);
    let tear = tear(section, frame);
    let mut drawn = Vec::with_capacity(mesh.splats.len());
    for splat in &mesh.splats {
        let p = rotate(splat.pos.map(|axis| axis * pose.scale));
        let n = rotate(splat.normal);
        let depth = DISTANCE - p[2];
        let to_eye = [-p[0], -p[1], depth];
        if depth < NEAR || dot(n, to_eye) <= 0.0 {
            continue;
        }
        let jacobian = |o: Vec3| {
            let o = rotate(o.map(|axis| axis * pose.scale));
            [
                FOCAL / aspect * (o[0] + p[0] * o[2] / depth) / depth,
                FOCAL * (o[1] + p[1] * o[2] / depth) / depth,
            ]
        };
        let (a, b) = (jacobian(splat.du), jacobian(splat.dv));
        let size = [
            (a[0].abs() + b[0].abs()) * OVERLAP,
            (a[1].abs() + b[1].abs()) * OVERLAP,
        ];
        let y = FOCAL * p[1] / depth + pose.lift;
        let x = FOCAL * p[0] / (aspect * depth) + tear.as_ref().map_or(0.0, |tear| tear(y));
        let view = normalize(to_eye);
        let albedo = morph(section, frame, splat);
        let surface = splat.surface;
        let diffuse = dot(n, light).max(0.0);
        let half = normalize(std::array::from_fn(|i| light[i] + view[i]));
        let spec = dot(n, half).max(0.0).powi(24) * surface.shine * 0.85;
        let fresnel = (1.0 - dot(n, view).max(0.0)).powi(3) * 0.55;
        let color: [f32; 3] = std::array::from_fn(|i| {
            let shaded = albedo[i] * (0.3 + 0.8 * diffuse) + rim[i] * fresnel;
            (albedo[i] * surface.glow + shaded * (1.0 - surface.glow) + spec).clamp(0.0, 1.0)
        });
        drawn.push(Drawn {
            depth,
            instance: quad([x, y], size, backdrop::rgba(color, 1.0), 0.0, [0.0; 2]),
        });
    }
    drawn.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    let excess = drawn.len().saturating_sub(budget);
    drawn.drain(..excess);
    drawn
}

/// Twinkling infomercial sparkles on the nearer half of the object.
fn sparkles(stage: &Stage, object: &[Drawn], flare: f32, out: &mut Vec<Instance>) {
    if object.is_empty() {
        return;
    }
    let count = 2 + (stage.energy * 4.0 + flare * 8.0) as usize;
    let window = stage.t as u32 / 18;
    let phase = (stage.t % 18.0) / 18.0;
    let near = object.len() / 2;
    for index in 0..count as u32 {
        let seed = index.wrapping_mul(0x85eb_ca6b) ^ window.wrapping_mul(0x27d4_eb2f);
        let pick = ((hash(seed) * near as f32) as usize).min(near.saturating_sub(1));
        let center = object[object.len() - 1 - pick].instance.center;
        let size = (0.05 + 0.08 * hash(seed ^ 1)) * (PI * phase).sin() * (1.0 + flare);
        let color = backdrop::rgba(hue(hash(seed ^ 2)).map(|c| 0.8 + 0.2 * c), 1.0);
        let center = [center[0], center[1]];
        out.push(backdrop::shape(
            center,
            size,
            stage.aspect,
            color,
            Shape::Sparkle,
        ));
    }
}

fn stage(frame: ControlFrame, section: Section, aspect: f32) -> Stage {
    Stage {
        aspect,
        clock: f32::from(frame.frame),
        t: f32::from(section.local),
        energy: f32::from(frame.energy) / 255.0,
        bass: f32::from(frame.bass) / 255.0,
        treble: f32::from(frame.treble) / 255.0,
        punch: punch(frame),
    }
}

/// Draws the background into a new list, returning it with the number of
/// splats left for the object out of `budget`.
fn background(
    profile: &RenderProfile,
    section: Section,
    stage: &Stage,
    budget: usize,
) -> (Vec<Instance>, usize) {
    let mut drawn = Vec::with_capacity(budget);
    let background_budget = (profile.instance_count / BACKGROUND_SHARE).min(budget);
    backdrop::background(section.background, stage, background_budget, &mut drawn);
    let object_budget = budget.saturating_sub(drawn.len() + OVERLAY_RESERVE);
    (drawn, object_budget)
}

fn build(
    profile: &RenderProfile,
    frame: ControlFrame,
    _elapsed: f32,
    aspect: f32,
    instances: &mut Vec<Instance>,
) {
    let aspect = aspect.max(0.01);
    let budget = profile.instance_count.saturating_sub(instances.len());
    let section = section(frame);
    let pose = pose(section, frame);
    let flare = flare(frame, section);
    let stage = stage(frame, section, aspect);
    let (mut drawn, object_budget) = background(profile, section, &stage, budget);
    let object = draw_object(section, frame, pose, aspect, object_budget);
    if section.object == Kind::Sun && pose.scale > 1e-3 {
        let radius = FOCAL * 0.78 * pose.scale / DISTANCE;
        backdrop::sun_rays(&stage, [0.0, pose.lift], radius, &mut drawn);
    }
    drawn.extend(object.iter().map(|splat| splat.instance));
    sparkles(&stage, &object, flare, &mut drawn);
    backdrop::flare(&stage, flare, &mut drawn);
    match section.overlay {
        Overlay::Camcorder => backdrop::camcorder(&stage, &mut drawn),
        Overlay::Infomercial => backdrop::infomercial(&stage, section.object, &mut drawn),
    }
    drawn.truncate(budget);
    instances.extend(drawn);
}
