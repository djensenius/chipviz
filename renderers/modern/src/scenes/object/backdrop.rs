//! Backgrounds, overlays and flourishes for the Object scene, all drawn as
//! flat `ScreenSpace` quads in NDC (`-1..1`, `+y` up).

use super::super::hash;
use super::super::static_noise::font::glyph;
use super::models::{Kind, Vec3, hue};
use super::{Background, Shape, quad};
use crate::scene::Instance;
use std::f32::consts::TAU;

/// Per-frame inputs shared by the background and overlays.
pub(super) struct Stage {
    pub(super) aspect: f32,
    /// The monotonic control-frame counter.
    pub(super) clock: f32,
    /// Frames since the section started.
    pub(super) t: f32,
    pub(super) energy: f32,
    pub(super) bass: f32,
    pub(super) treble: f32,
    pub(super) punch: f32,
}

/// The chroma-key background holds each colour this many frames.
pub(super) const CHROMA_FRAMES: u16 = 32;
/// Linear luminance every chroma-key colour is scaled to.
const CHROMA_LUMINANCE: f32 = 0.3;
const CHROMA_KEYS: [Vec3; 4] = [
    [0.0, 1.0, 0.25],
    [0.1, 0.3, 1.0],
    [1.0, 0.0, 1.0],
    [1.0, 0.45, 0.0],
];
pub(super) const TAGLINES: [&str; 4] = ["AS SEEN ON TV!", "CALL NOW!", "ONLY 19.95", "BUT WAIT..."];
const GOLD: [f32; 4] = [1.0, 0.8, 0.15, 1.0];

pub(super) fn luminance(color: Vec3) -> f32 {
    0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2]
}

/// The chroma-key colour at control frame `clock`: hard cuts between hues
/// of (near) equal luminance, so the frame changes colour, not brightness.
pub(super) fn chroma_key(clock: u16) -> Vec3 {
    let key = CHROMA_KEYS[usize::from(clock / CHROMA_FRAMES) % CHROMA_KEYS.len()];
    let gain = CHROMA_LUMINANCE / luminance(key);
    key.map(|channel| (channel * gain).min(1.0))
}

pub(super) fn rgba(color: Vec3, alpha: f32) -> [f32; 4] {
    [color[0], color[1], color[2], alpha]
}

fn solid(center: [f32; 2], size: [f32; 2], color: [f32; 4]) -> Instance {
    quad(center, size, color, 0.0, [0.0; 2])
}

/// A round shape `diameter` NDC-y units across (round in pixels).
pub(super) fn shape(
    center: [f32; 2],
    diameter: f32,
    aspect: f32,
    color: [f32; 4],
    shape: Shape,
) -> Instance {
    quad(
        center,
        [diameter / aspect, diameter],
        color,
        0.0,
        shape.pattern(),
    )
}

/// A bar from `a` to `b` (NDC), `width` thick.
fn bar(a: [f32; 2], b: [f32; 2], width: f32, color: [f32; 4]) -> Instance {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    quad(
        [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5],
        [dx.hypot(dy), width],
        color,
        dy.atan2(dx),
        [0.0; 2],
    )
}

/// The end of a ray of `length` (NDC y units, round in pixels) at `angle`.
fn ray(center: [f32; 2], angle: f32, length: f32, aspect: f32) -> [f32; 2] {
    [
        center[0] + angle.cos() * length / aspect,
        center[1] + angle.sin() * length,
    ]
}

/// Horizontal strips from `top` to `bottom` through evenly spaced `stops`.
fn gradient(top: f32, bottom: f32, stops: &[Vec3], strips: usize, out: &mut Vec<Instance>) {
    let height = (top - bottom) / strips as f32;
    for strip in 0..strips {
        let t = (strip as f32 + 0.5) / strips as f32 * (stops.len() - 1) as f32;
        let (low, mix) = ((t as usize).min(stops.len() - 2), t.fract());
        let color =
            std::array::from_fn(|k| stops[low][k] + (stops[low + 1][k] - stops[low][k]) * mix);
        let y = top - (strip as f32 + 0.5) * height;
        out.push(solid([0.0, y], [2.0, height * 1.02], rgba(color, 1.0)));
    }
}

/// Draws `background` using at most `budget` instances.
pub(super) fn background(
    background: Background,
    stage: &Stage,
    budget: usize,
    out: &mut Vec<Instance>,
) {
    let start = out.len();
    match background {
        Background::Stars => stars(stage, budget, out),
        Background::Goop => goop(stage, budget, out),
        Background::Vaporwave => vaporwave(stage, out),
        Background::Rainbow => rainbow(stage, budget, out),
        Background::ChromaKey => {
            let key = chroma_key(stage.clock as u16);
            out.push(solid([0.0; 2], [2.0; 2], rgba(key, 1.0)));
            for index in 0..3 {
                let drift = stage.clock * 0.004 + index as f32 * 2.1;
                let center = [drift.sin() * 0.8, (drift * 1.3).cos() * 0.6];
                let color = rgba(hue(stage.clock / 600.0 + index as f32 / 3.0), 0.18);
                out.push(shape(center, 1.4, stage.aspect, color, Shape::Glow));
            }
        }
    }
    out.truncate(start + budget);
}

/// A procedural warp-speed starfield over faint nebulae.
fn stars(stage: &Stage, budget: usize, out: &mut Vec<Instance>) {
    let stops = [[0.0, 0.0, 0.03], [0.02, 0.0, 0.07], [0.04, 0.01, 0.1]];
    gradient(1.0, -1.0, &stops, 12, out);
    for index in 0..4 {
        let drift = stage.clock * 0.002 + index as f32 * 1.7;
        let center = [drift.sin() * 0.7, (drift * 0.8).cos() * 0.5];
        let color = rgba(hue(0.6 + 0.12 * index as f32), 0.22);
        out.push(shape(center, 1.8, stage.aspect, color, Shape::Glow));
    }
    for index in 0..budget.saturating_sub(16) as u32 {
        let seed = index.wrapping_mul(3);
        let angle = hash(seed + 1) * TAU;
        let speed = 0.5 + hash(seed + 2);
        let z = (hash(seed + 3) + stage.clock * 0.002 * speed).fract();
        let r = 0.03 + z * z * 1.9;
        let center = [angle.cos() * r * 1.1, angle.sin() * r * 0.9];
        let twinkle = 1.0 + 0.6 * stage.treble * hash(seed ^ stage.clock as u32);
        let size = (0.004 + z * 0.014) * twinkle;
        let tint = hash(seed + 4);
        let color = [0.8 + 0.2 * tint, 0.85, 1.0 - 0.2 * tint, (z * 5.0).min(1.0)];
        let (scale, kind) =
            [(1.0, Shape::Disc), (5.0, Shape::Sparkle)][usize::from(index % 17 == 0)];
        out.push(shape(center, size * scale, stage.aspect, color, kind));
    }
}

/// Lava-lamp goop: outlined blobs that merge as they wander, with glossy
/// highlights.
fn goop(stage: &Stage, budget: usize, out: &mut Vec<Instance>) {
    let stops = [[0.12, 0.0, 0.15], [0.25, 0.02, 0.2], [0.06, 0.0, 0.08]];
    gradient(1.0, -1.0, &stops, 12, out);
    let colors = [[1.0, 0.2, 0.6], [0.5, 1.0, 0.1], [1.0, 0.55, 0.05]];
    let count = (budget.saturating_sub(12) / 3).min(18) as u32;
    let blobs: Vec<([f32; 2], f32, Vec3)> = (0..count)
        .map(|index| {
            let seed = index.wrapping_mul(5);
            let (w1, w2) = (
                0.003 + 0.005 * hash(seed + 1),
                0.003 + 0.005 * hash(seed + 2),
            );
            let (p1, p2) = (hash(seed + 3) * TAU, hash(seed + 4) * TAU);
            let center = [
                1.05 * (stage.clock * w1 + p1).sin(),
                0.95 * (stage.clock * w2 + p2).sin(),
            ];
            let radius = (0.22 + 0.28 * hash(seed + 5)) * (1.0 + 0.15 * stage.bass);
            (center, radius, colors[index as usize % 3])
        })
        .collect();
    for &(center, radius, color) in &blobs {
        let dark = rgba(color.map(|channel| channel * 0.3), 1.0);
        out.push(shape(center, radius * 2.2, stage.aspect, dark, Shape::Disc));
    }
    for &(center, radius, color) in &blobs {
        let body = rgba(color, 1.0);
        out.push(shape(center, radius * 2.0, stage.aspect, body, Shape::Disc));
    }
    for &(center, radius, color) in &blobs {
        let spot = [
            center[0] - 0.3 * radius / stage.aspect,
            center[1] + 0.35 * radius,
        ];
        let light = rgba(color.map(|channel| 0.4 + 0.6 * channel), 0.7);
        out.push(shape(spot, radius * 0.5, stage.aspect, light, Shape::Disc));
    }
}

/// A vaporwave sunset over a scrolling neon grid floor.
fn vaporwave(stage: &Stage, out: &mut Vec<Instance>) {
    let horizon = -0.2;
    let stops = [
        [0.05, 0.0, 0.15],
        [0.45, 0.05, 0.45],
        [1.0, 0.35, 0.45],
        [1.0, 0.6, 0.3],
    ];
    gradient(1.0, horizon, &stops, 16, out);
    let center = [0.0, horizon + 0.34];
    let glow = [1.0, 0.4, 0.5, 0.6];
    out.push(shape(center, 1.5, stage.aspect, glow, Shape::Glow));
    let disc = [1.0, 0.78, 0.25, 1.0];
    out.push(shape(center, 0.95, stage.aspect, disc, Shape::Disc));
    let scroll = (stage.clock * 0.002).fract() * 0.075;
    for stripe in 0..6 {
        let y = center[1] - 0.02 - stripe as f32 * 0.075 - scroll;
        let thick = 0.012 + 0.008 * stripe as f32;
        let sky = [0.85, 0.28, 0.45, 1.0];
        out.push(solid([0.0, y], [1.0 / stage.aspect, thick], sky));
    }
    let floor = [0.06, 0.0, 0.12, 1.0];
    out.push(solid(
        [0.0, (horizon - 1.0) * 0.5],
        [2.0, horizon + 1.0],
        floor,
    ));
    out.push(solid([0.0, horizon], [2.0, 0.008], [1.0, 0.5, 0.9, 1.0]));
    let travel = (stage.clock * 0.03).fract();
    for row in 0..18 {
        let z = 1.0 + row as f32 - travel;
        let y = horizon - (horizon + 1.0) / z;
        let color = [1.0, 0.2, 0.85, (0.3 + 0.7 / z.sqrt()).min(1.0)];
        out.push(solid([0.0, y], [2.0, 0.003 + 0.01 / z], color));
    }
    for column in -10..=10 {
        let x = column as f32 * 0.3;
        let far = [x / 18.0, horizon - (horizon + 1.0) / 18.0];
        out.push(bar(far, [x, -1.0], 0.006, [1.0, 0.2, 0.85, 0.9]));
    }
}

/// Diagonal rainbow stripes whose hues flow across the screen.
fn rainbow(stage: &Stage, budget: usize, out: &mut Vec<Instance>) {
    let count = budget.min(40);
    let angle: f32 = 0.6;
    let width = 3.4 / count.max(1) as f32;
    for stripe in 0..count {
        let d = -1.7 + (stripe as f32 + 0.5) * width;
        let center = [angle.cos() * d, angle.sin() * d];
        let color = hue(d * 0.45 - stage.clock * 0.004).map(|channel| 0.15 + 0.7 * channel);
        let size = [width * 1.04, 4.0];
        out.push(quad(center, size, rgba(color, 1.0), angle, [0.0; 2]));
    }
}

/// The sun's spinning rays and halo, drawn behind it.
pub(super) fn sun_rays(stage: &Stage, center: [f32; 2], radius: f32, out: &mut Vec<Instance>) {
    let halo = [1.0, 0.55, 0.1, 0.6];
    out.push(shape(center, radius * 5.0, stage.aspect, halo, Shape::Glow));
    for index in 0..20 {
        let angle = stage.clock * 0.006 + index as f32 * TAU / 20.0;
        let wobble = 0.15 * (index as f32 * 1.7 + stage.clock * 0.05).sin();
        let length = radius * (1.45 + 0.45 * stage.bass + wobble);
        let color = [[1.0, 0.75, 0.2, 0.9], [1.0, 0.45, 0.1, 0.9]][index % 2];
        let end = ray(center, angle, length, stage.aspect);
        out.push(bar(center, end, radius * 0.16, color));
    }
}

/// A lens flare from the upper left through the centre, `strength` 0..1.
pub(super) fn flare(stage: &Stage, strength: f32, out: &mut Vec<Instance>) {
    if strength < 0.02 {
        return;
    }
    let source = [-0.62, 0.66];
    let warm = [1.0, 0.9, 0.7, 0.8 * strength];
    out.push(shape(
        source,
        0.9 * strength,
        stage.aspect,
        warm,
        Shape::Glow,
    ));
    let star = [1.0, 1.0, 1.0, strength];
    out.push(shape(
        source,
        0.7 * strength,
        stage.aspect,
        star,
        Shape::Sparkle,
    ));
    let ghosts = [
        (0.35, 0.12, 0.55),
        (0.6, 0.07, 0.15),
        (0.85, 0.22, 0.75),
        (1.1, 0.1, 0.35),
        (1.4, 0.3, 0.9),
    ];
    for (along, size, tint) in ghosts {
        let center = source.map(|axis| axis * (1.0 - 1.6 * along));
        let color = rgba(hue(tint), 0.3 * strength);
        out.push(shape(center, size, stage.aspect, color, Shape::Glow));
    }
}

/// Text centred on `center` in Static's 5x8 font, `height` NDC tall, with a
/// drop shadow. Characters the font lacks are skipped.
pub(super) fn text(
    stage: &Stage,
    line: &str,
    center: [f32; 2],
    height: f32,
    color: [f32; 4],
    out: &mut Vec<Instance>,
) {
    let pixel = height / 8.0;
    let pixel_x = pixel / stage.aspect;
    let count = line.chars().count() as f32;
    let left = center[0] - (count * 6.0 - 1.0) * pixel_x * 0.5;
    let shadow = [0.0, 0.0, 0.0, color[3] * 0.8];
    for (index, character) in line.chars().enumerate() {
        let Some(bits) = glyph(character).filter(|&bits| bits != 0) else {
            continue;
        };
        let pattern = [(bits & 0xf_ffff) as f32, (bits >> 20 & 0xf_ffff) as f32];
        let x = left + (index as f32 * 6.0 + 2.5) * pixel_x;
        let size = [5.0 * pixel_x, height];
        let offset = [x + pixel_x * 0.7, center[1] - pixel * 0.7];
        out.push(quad(offset, size, shadow, 0.0, pattern));
        out.push(quad([x, center[1]], size, color, 0.0, pattern));
    }
}

/// Camcorder playback: scanlines, a rolling tracking bar, "REC" and a
/// running date and clock.
pub(super) fn camcorder(stage: &Stage, out: &mut Vec<Instance>) {
    let scanlines = Shape::Scanlines.pattern();
    out.push(quad(
        [0.0; 2],
        [2.0; 2],
        [0.0, 0.0, 0.0, 0.3],
        0.0,
        scanlines,
    ));
    let roll = 1.25 - (stage.clock / 360.0).fract() * 2.5;
    let height = 0.06 + 0.05 * hash(stage.clock as u32 / 30);
    // Static's monochrome pixel noise: 45% blank, changing every frame.
    let clock = stage.clock as u32 & 0xffff;
    let noise = [-1.45, (clock << 4) as f32];
    let haze = [1.0, 1.0, 1.0, 0.55];
    out.push(quad([0.0, roll], [2.0, height], haze, 0.0, noise));
    let edge = [0.0, roll - height * 0.5];
    out.push(solid(edge, [2.0, 0.006], [1.0, 1.0, 1.0, 0.35]));
    let white = [0.95, 0.95, 0.95, 1.0];
    if (stage.clock as u32 / 30).is_multiple_of(2) {
        let red = [1.0, 0.05, 0.05, 1.0];
        out.push(shape([-0.88, 0.84], 0.055, stage.aspect, red, Shape::Disc));
    }
    text(stage, "REC", [-0.78, 0.84], 0.065, white, out);
    text(stage, "SP", [0.86, 0.84], 0.065, white, out);
    let seconds = stage.clock as u32 / 60;
    let clock = format!("{:02}:{:02}:{:02} PM", 11, seconds / 60 % 60, seconds % 60);
    text(stage, "JAN. 1 1989", [-0.62, -0.84], 0.065, white, out);
    text(stage, &clock, [0.62, -0.84], 0.065, white, out);
}

/// Infomercial graphics: a caption banner and a spinning "NEW!" burst.
pub(super) fn infomercial(stage: &Stage, object: Kind, out: &mut Vec<Instance>) {
    out.push(solid([0.0, -0.8], [2.0, 0.3], [0.02, 0.04, 0.3, 0.88]));
    out.push(solid([0.0, -0.65], [2.0, 0.012], GOLD));
    out.push(solid([0.0, -0.95], [2.0, 0.012], GOLD));
    text(stage, object.caption(), [0.0, -0.74], 0.1, GOLD, out);
    let tagline = TAGLINES[(stage.t as usize / 128) % TAGLINES.len()];
    text(stage, tagline, [0.0, -0.885], 0.055, [1.0; 4], out);
    let center = [0.72, 0.62];
    let radius = 0.16 * (1.0 + 0.15 * stage.punch);
    for index in 0..16 {
        let angle = stage.clock * 0.01 + index as f32 * TAU / 16.0;
        let end = ray(center, angle, radius * 1.55, stage.aspect);
        let color = [GOLD, [1.0, 0.3, 0.1, 1.0]][index % 2];
        out.push(bar(center, end, radius * 0.45, color));
    }
    let red = [0.85, 0.05, 0.1, 1.0];
    out.push(shape(center, radius * 2.0, stage.aspect, red, Shape::Disc));
    let height = 0.08 * (1.0 + 0.15 * stage.punch);
    text(stage, "NEW!", center, height, GOLD, out);
}
