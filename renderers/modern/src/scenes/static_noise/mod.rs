//! Static: a field of noise that now and then resolves into something
//! readable (a multilingual phrase, a mathematical constant, or a 3D knot
//! traced in digits) and dissolves back.
//!
//! The field is a grid of glyph cells laid on a plane. Each cell is one
//! `ScreenSpace` quad whose fragment shader draws a 5x8 bitmap from the
//! instance `pattern` (see `font.rs` and `glyph_lit` in `shader.wgsl`), so a
//! glyph costs one instance and the grid fills the profile's instance budget.
//! In "pixel" sections the plane is instead covered by a few strips whose
//! shader draws hashed noise at every output pixel (`pixel_noise`).
//!
//! A section's index picks the content (text, constant, knot, or pure
//! noise), the style (glyph or pixel noise, alternating every four
//! sections), the camera move (flat, zoom out, orbit) and the background
//! (white every fifth section). Within a section each content cell first
//! cycles through random glyphs, lands on its target at a random frame in
//! 160..256, holds, then leaves at a random frame in 400..496 and cycles back
//! into noise.
//!
//! Sections follow the beat. [`StaticState`] (`beat.rs`) keeps the schedule
//! between frames, stepping only on observed frames like Lines, so captures
//! replayed from frame 0 are reproducible:
//!
//! - A section runs its whole 512-frame timeline (about 8.5 s at 60 fps),
//!   then the next starts on the first bar line, when `beat_count` crosses a
//!   multiple of 4. Sections are therefore at least 512 frames apart, so the
//!   white background still cuts in or out at most once per 8.5 s. With
//!   procedural input (a beat every 16 frames) that is exactly every 512
//!   frames, as before; at 120 bpm live it is every 600.
//! - Every bar line at least 60 frames (1 s) after the last one also
//!   reframes the camera: it eases over one beat (15 to 30 frames) to the
//!   next of six framings (head on, either side, closer, from above and
//!   pulled back), on top of the section's move. Head on comes every other
//!   bar, so text stays readable.
//! - In a text section, such a bar during the hold (frames 256 to 364)
//!   flips the phrase to another language like a split-flap board: each
//!   cell keeps its old character until its own frame in the next 24, cycles
//!   random glyphs for 12, then lands on the new phrase, so the change
//!   spreads over 36 frames and finishes before the dissolve.
//! - A backward or large jump in `beat_count` (a restart, loop or source
//!   handover) is no bar. If the counter stalls for 4 s, sections free-run
//!   every 512 frames and the camera holds its framing.
//! - Before the state has seen a frame, the schedule is the frame counter's
//!   (`frame / 512`, head on), and a state joins that schedule on its first
//!   frame. The recorded golden fingerprint is built this way, so it is
//!   unchanged.
//!
//! Flash safety: the beat changes only geometry and characters, never the
//! background or brightness, and every change eases or staggers over at
//! least 15 frames. `beat_changes_keep_luminance_flashes_bounded` in
//! `tests.rs` measures large-area luminance under racing beats: at most one
//! flash in any second, under WCAG 2.3.1's three (see its table).
//!
//! Audio drives the noise: energy shortens each cell's change period (1 to
//! 12 frames) and lowers the chance of a blank cell (80% when silent, 20% at
//! full energy); every beat reshuffles the field. `palette % 4` picks the
//! colour (0..2 tint the field like the other scenes, 3 is random hues).
//!
//! The camera mostly faces the plane head on, reframed on bars as above.
//! Some sections zoom out or orbit a full turn (passing behind the plane,
//! where text reads mirrored); the scene projects cell centres on the CPU as
//! Lines does.

mod beat;
pub(super) mod font;
mod text;

use super::{CameraMode, SceneBuild, SceneEntry, SceneState, palette, sin_cos};
use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scene::Instance;
pub use beat::StaticState;
use beat::{Clock, FLIP_CYCLE, FLIP_SPREAD, Framing, REST};
use font::{ALPHABET, FULL, glyph};
use std::f32::consts::{PI, TAU};
use text::{Embedded, TextSource};

pub const ENTRY: SceneEntry = SceneEntry {
    name: "Static",
    camera: CameraMode::ScreenSpace,
    build: SceneBuild::Stateful(build),
};

/// Divides the u16 frame counter's wrap.
const SECTION_FRAMES: u16 = 512;
/// Cells land on their target at a random frame in `RESOLVE_START..HOLD_START`.
const RESOLVE_START: u16 = 160;
const SPREAD: u16 = 96;
/// Every content cell has landed from here until `DISSOLVE_START`.
const HOLD_START: u16 = RESOLVE_START + SPREAD;
/// Cells leave their target at a random frame in `DISSOLVE_START..+SPREAD`.
const DISSOLVE_START: u16 = 400;
/// Frames a content cell spends cycling glyphs before landing / after leaving.
const CYCLE_FRAMES: u16 = 40;
/// Camera moves run over this part of a section and rest at either end.
const MOVE_START: u16 = 32;
const MOVE_END: u16 = 480;
const NOISE_STRIPS: usize = 16;
/// Instances kept back from the grid: a background quad and the noise strips.
const RESERVED: usize = 1 + NOISE_STRIPS;
/// A cell in font pixels: a 5x8 glyph and a one-pixel gap on two sides.
const CELL_W: f32 = 6.0;
const CELL_H: f32 = 9.0;
/// Blank-cell chance at full energy and in silence.
const MIN_BLANK: f32 = 0.2;
const MAX_BLANK: f32 = 0.8;
/// The longest change period (frames) of a noise cell, reached in silence.
const SLOWEST: u32 = 12;
const NEAR: f32 = 0.1;
/// 1 / tan(fov / 2) for a 60 degree vertical field of view.
const FOCAL: f32 = 1.732_050_8;
const SCENE_MIX: f32 = 4.0;
/// Largest font-pixel block, in cells, for resolved text.
const MAX_SCALE: usize = 3;
/// Each text flip moves this many phrases on (coprime with the 18 embedded
/// phrases, so flips visit them all).
const FLIP_STRIDE: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Content {
    Noise,
    Text(usize),
    Constant(usize),
    Knot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Glyphs,
    Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Move {
    Flat,
    ZoomOut,
    Orbit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Section {
    index: u16,
    local: u16,
    content: Content,
    style: Style,
    camera: Move,
    white: bool,
    /// Text flips so far, and the `local` frame the last one started.
    flips: u16,
    flip_at: u16,
}

/// The frame-timed section for `frame` (the clock before any frame is seen).
#[cfg(test)]
fn section(frame: ControlFrame) -> Section {
    section_at(Clock::frame_timed(frame))
}

fn section_at(clock: Clock) -> Section {
    let index = clock.index;
    let cycle = usize::from(index / 4);
    Section {
        index,
        local: clock.local,
        content: match index % 4 {
            0 => Content::Text(cycle),
            1 => Content::Constant(cycle),
            2 => Content::Knot,
            _ => Content::Noise,
        },
        style: if (index / 4).is_multiple_of(2) {
            Style::Glyphs
        } else {
            Style::Pixels
        },
        camera: match index % 3 {
            0 => Move::Flat,
            1 => Move::ZoomOut,
            _ => Move::Orbit,
        },
        white: index % 5 == 4,
        flips: clock.flips,
        flip_at: clock.flip_at,
    }
}

impl Section {
    /// While a text flip is animating, the section as it was before it.
    fn before_flip(self) -> Option<Self> {
        let animating = self.local < self.flip_at + FLIP_SPREAD + FLIP_CYCLE;
        (self.flips > 0 && animating).then(|| Self {
            flips: self.flips - 1,
            ..self
        })
    }
}

/// Glyph cells, sized so `RESERVED + cols * rows` fits the instance budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Grid {
    cols: usize,
    rows: usize,
}

impl Grid {
    fn new(profile: &RenderProfile, aspect: f32) -> Self {
        let budget = profile.instance_count.saturating_sub(RESERVED);
        let per_row = aspect.clamp(0.25, 4.0) * CELL_H / CELL_W;
        let mut rows = (budget as f32 / per_row).sqrt() as usize;
        let mut cols = (rows as f32 * per_row) as usize;
        while rows * cols > budget {
            if cols > rows { cols -= 1 } else { rows -= 1 }
        }
        Self { cols, rows }
    }

    fn len(self) -> usize {
        self.cols * self.rows
    }

    /// World size of one cell; the plane is 2 units tall.
    fn cell(self) -> [f32; 2] {
        let height = 2.0 / self.rows.max(1) as f32;
        [height * CELL_W / CELL_H, height]
    }
}

/// What each cell should read once resolved: `None` is blank.
fn target(section: Section, grid: Grid, source: &dyn TextSource) -> Vec<Option<char>> {
    match section.content {
        Content::Noise => vec![None; grid.len()],
        Content::Text(_) if source.len() == 0 => vec![None; grid.len()],
        Content::Text(cycle) => {
            let index = cycle + usize::from(section.flips) * FLIP_STRIDE;
            lay_out(&[source.phrase(index)], grid)
        }
        Content::Constant(index) => {
            let [name, value] = text::constant(index);
            lay_out(&[name.as_str(), value.as_str()], grid)
        }
        Content::Knot => knot(grid, section.local),
    }
}

/// Centres `lines` on the grid, drawn large when they fit (each font pixel
/// a `scale` x `scale` block of cells, every lit cell showing its letter),
/// else one cell per character.
fn lay_out(lines: &[&str], grid: Grid) -> Vec<Option<char>> {
    for scale in (1..=MAX_SCALE).rev() {
        let width = grid.cols.saturating_sub(scale) / (6 * scale);
        let wrapped = wrap(lines, width);
        let fits = wrapped.iter().all(|line| line.len() <= width);
        if fits && wrapped.len() <= grid.rows.saturating_sub(scale) / (9 * scale) {
            return place(&wrapped, grid, scale);
        }
    }
    place(&wrap(lines, grid.cols.saturating_sub(2).max(1)), grid, 0)
}

/// Greedy word wrap to `width` characters; overlong words keep their own line.
fn wrap(lines: &[&str], width: usize) -> Vec<Vec<char>> {
    let mut wrapped: Vec<Vec<char>> = Vec::new();
    for line in lines {
        let mut current: Vec<char> = Vec::new();
        for word in line.split(' ').filter(|word| !word.is_empty()) {
            let word: Vec<char> = word.chars().collect();
            if !current.is_empty() && current.len() + 1 + word.len() > width {
                wrapped.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.extend(word);
        }
        wrapped.push(current);
    }
    wrapped
}

/// Large letters (`scale` >= 1) sit 6 x 9 font pixels apart, leaving a
/// one-pixel gap; `scale` 0 is one cell per character.
fn place(lines: &[Vec<char>], grid: Grid, scale: usize) -> Vec<Option<char>> {
    let mut cells = vec![None; grid.len()];
    let mut set = |x: usize, y: usize, character: char| {
        if x < grid.cols && y < grid.rows {
            cells[y * grid.cols + x] = Some(character);
        }
    };
    let (step_x, step_y) = if scale == 0 {
        (1, 1)
    } else {
        (6 * scale, 9 * scale)
    };
    let height = (lines.len() * step_y).saturating_sub(scale);
    let top = grid.rows.saturating_sub(height) / 2;
    for (number, line) in lines.iter().enumerate() {
        let width = (line.len() * step_x).saturating_sub(scale);
        let left = grid.cols.saturating_sub(width) / 2;
        for (position, &character) in line.iter().enumerate() {
            let (x, y) = (left + position * step_x, top + number * step_y);
            if scale == 0 {
                if character != ' ' {
                    set(x, y, character);
                }
                continue;
            }
            let bits = glyph_or_noise(character);
            for bit in (0..40).filter(|bit| bits >> bit & 1 == 1) {
                for cell in 0..scale * scale {
                    let column = x + (bit % 5) * scale + cell % scale;
                    set(column, y + (bit / 5) * scale + cell / scale, character);
                }
            }
        }
    }
    cells
}

/// A (3, 2) torus knot drawn in digits as it turns: the trace grows across
/// the section and each point shows a digit of its depth. The bottom row
/// reads the newest point's coordinates.
fn knot(grid: Grid, local: u16) -> Vec<Option<char>> {
    let mut cells = vec![None; grid.len()];
    if grid.rows < 4 {
        return cells;
    }
    let angle = f32::from(local) * 0.012;
    let span = f32::from(DISSOLVE_START - RESOLVE_START);
    let progress = (f32::from(local.saturating_sub(RESOLVE_START)) / span).clamp(0.0, 1.0);
    let samples = (grid.len() / 6).clamp(64, 1200);
    let shown = (samples as f32 * progress).ceil() as usize;
    let height = grid.rows - 2;
    let scale = height as f32 * 0.48;
    let mut newest = text::knot_point(0.0, angle);
    for sample in 0..shown {
        newest = text::knot_point(sample as f32 / samples as f32, angle);
        let x = grid.cols as f32 * 0.5 + newest[0] * scale * CELL_H / CELL_W;
        let y = height as f32 * 0.5 - newest[1] * scale;
        if (0.0..grid.cols as f32).contains(&x) && (0.0..height as f32).contains(&y) {
            let digit = ((newest[2] + 1.0) * 5000.0) as u32 % 10;
            cells[y as usize * grid.cols + x as usize] = char::from_digit(digit, 10);
        }
    }
    let label: Vec<char> = text::coordinates(newest).chars().collect();
    let left = grid.cols.saturating_sub(label.len()) / 2;
    let row = (grid.rows - 1) * grid.cols;
    for (offset, character) in label.into_iter().enumerate().take(grid.cols) {
        if character != ' ' {
            cells[row + left + offset] = Some(character);
        }
    }
    cells
}

/// The glyph for `character`, or stable glyph-like noise if the font lacks it.
fn glyph_or_noise(character: char) -> u64 {
    glyph(character).unwrap_or_else(|| random_bits(mix(u32::from(character))))
}

fn random_bits(seed: u32) -> u64 {
    let bits = (u64::from(mix(seed ^ 7)) | u64::from(mix(seed ^ 11)) << 32) & FULL;
    if bits == 0 { FULL } else { bits }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Blank,
    Noise,
    Cycling,
    Target,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    glyph: u64,
    role: Role,
    /// Per-cell hash for colour variation.
    tone: u32,
}

/// Audio-driven noise controls.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Noise {
    blank: f32,
    /// Longest change period in frames; each cell picks 1..=longest.
    longest: u32,
    clock: u32,
    /// Low bits of the beat count: each beat reshuffles every cell.
    beat: u32,
}

impl Noise {
    fn new(frame: ControlFrame) -> Self {
        let quiet = 1.0 - f32::from(frame.energy) / 255.0;
        Self {
            blank: MIN_BLANK + (MAX_BLANK - MIN_BLANK) * quiet,
            longest: 1 + (quiet * (SLOWEST - 1) as f32).round() as u32,
            clock: u32::from(frame.frame),
            beat: u32::from(frame.beat_count) & 7,
        }
    }
}

/// Cell `index` of `section`. `before` is its target before the latest
/// text flip, while that flip is animating (see [`Section::before_flip`]).
fn cell(
    index: usize,
    target: Option<char>,
    before: Option<Option<char>>,
    section: Section,
    noise: Noise,
) -> Cell {
    let seed = mix(index as u32 ^ mix(u32::from(section.index)));
    let land = RESOLVE_START + (unit(mix(seed ^ 1)) * f32::from(SPREAD)) as u16;
    let leave = DISSOLVE_START + (unit(mix(seed ^ 2)) * f32::from(SPREAD)) as u16;
    let local = section.local;
    let active = section.content != Content::Noise;
    let landed = |target: Option<char>| match target {
        Some(character) => Cell {
            glyph: glyph_or_noise(character),
            role: Role::Target,
            tone: seed,
        },
        None => blank(seed),
    };
    // A fresh random glyph every other frame until it lands.
    let cycling = || {
        let pick = mix(seed ^ mix(u32::from(local / 2) + 3));
        Cell {
            glyph: glyph_or_noise(ALPHABET[pick as usize % ALPHABET.len()]),
            role: Role::Cycling,
            tone: pick,
        }
    };
    if active && (land..leave).contains(&local) {
        // A flip works like a split-flap board: each cell keeps its old
        // character until its own staggered switch, cycles, then lands.
        if let Some(before) = before.filter(|&before| before != target) {
            let switch = section.flip_at + (unit(mix(seed ^ 3)) * f32::from(FLIP_SPREAD)) as u16;
            if local < switch {
                return landed(before);
            }
            if local < switch + FLIP_CYCLE {
                return cycling();
            }
        }
        return landed(target);
    }
    if active && target.is_some() && local + CYCLE_FRAMES >= land && local < leave + CYCLE_FRAMES {
        return cycling();
    }
    match section.style {
        Style::Pixels => blank(seed),
        Style::Glyphs => noise_cell(index, noise),
    }
}

fn blank(tone: u32) -> Cell {
    Cell {
        glyph: 0,
        role: Role::Blank,
        tone,
    }
}

/// A noise cell changes every `period` frames (its own phase), and is blank,
/// a font glyph, a solid block or a random bitmap.
fn noise_cell(index: usize, noise: Noise) -> Cell {
    let seed = mix(index as u32 ^ 0x9e37_79b9);
    let period = 1 + seed % noise.longest.max(1);
    let step = (noise.clock + (seed >> 8) % period) / period;
    let value = mix(seed ^ mix(step.wrapping_add(noise.beat.wrapping_mul(0x85eb_ca6b))));
    if unit(value) < noise.blank {
        return blank(value);
    }
    let pick = mix(value);
    let kind = unit(pick);
    let glyph = if kind < 0.7 {
        glyph_or_noise(ALPHABET[pick as usize % ALPHABET.len()])
    } else if kind < 0.85 {
        FULL
    } else {
        random_bits(pick)
    };
    Cell {
        glyph,
        role: Role::Noise,
        tone: value,
    }
}

/// 0 outside content, 1 while it holds: how far the pixel noise thins out.
fn content_amount(section: Section) -> f32 {
    if section.content == Content::Noise {
        return 0.0;
    }
    let ramp = |start: u16| {
        let t = ((f32::from(section.local) - f32::from(start)) / f32::from(SPREAD)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    ramp(RESOLVE_START) * (1.0 - ramp(DISSOLVE_START))
}

/// Instance pattern for per-pixel noise; decoded by `pixel_noise` in
/// `shader.wgsl`. `inverted` (white sections) complements rainbow hues, as
/// `ink` does for every other colour.
fn noise_pattern(blank: f32, rainbow: bool, inverted: bool, noise: Noise) -> [f32; 2] {
    let mode = if rainbow { 2.0 } else { 0.0 } + if inverted { 4.0 } else { 0.0 };
    let clock = (noise.clock & 0xffff) | noise.beat << 16;
    let longest = noise.longest.clamp(1, 16) - 1;
    [
        -(1.0 + mode + blank.clamp(0.0, 0.99)),
        (clock << 4 | longest) as f32,
    ]
}

fn glyph_pattern(bits: u64) -> [f32; 2] {
    [(bits & 0xf_ffff) as f32, (bits >> 20 & 0xf_ffff) as f32]
}

struct View {
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
    aspect: f32,
}

/// (yaw, pitch, distance). At rest the camera sits `FOCAL` in front of the
/// plane, where the plane's 2-unit height spans the viewport exactly.
fn camera_pose(section: Section) -> (f32, f32, f32) {
    let span = f32::from(MOVE_END - MOVE_START);
    let t = (f32::from(section.local.saturating_sub(MOVE_START)) / span).min(1.0);
    if t <= 0.0 || t >= 1.0 {
        return (0.0, 0.0, FOCAL);
    }
    let bump = (PI * t).sin().powi(2);
    match section.camera {
        Move::Flat => (0.0, 0.0, FOCAL),
        Move::ZoomOut => (
            0.3 * (TAU * t).sin() * bump,
            0.2 * bump,
            FOCAL * (1.0 + 1.8 * bump),
        ),
        Move::Orbit => {
            let eased = t * t * (3.0 - 2.0 * t);
            (TAU * eased, 0.12 * bump, FOCAL * (1.0 + 1.4 * bump))
        }
    }
}

impl View {
    fn new((yaw, pitch, distance): (f32, f32, f32), aspect: f32) -> Self {
        let ((yaw_sin, yaw_cos), (pitch_sin, pitch_cos)) = (sin_cos(yaw), sin_cos(pitch));
        let eye = [
            distance * yaw_sin * pitch_cos,
            distance * pitch_sin,
            distance * yaw_cos * pitch_cos,
        ];
        let forward = normalize(eye.map(|axis| -axis));
        let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
        Self {
            eye,
            right,
            up: cross(right, forward),
            forward,
            aspect,
        }
    }

    fn project(&self, point: [f32; 3]) -> Option<[f32; 2]> {
        let offset = std::array::from_fn(|axis| point[axis] - self.eye[axis]);
        let depth = dot(offset, self.forward);
        if depth.is_nan() || depth < NEAR {
            return None;
        }
        Some([
            dot(offset, self.right) * FOCAL / (self.aspect * depth),
            dot(offset, self.up) * FOCAL / depth,
        ])
    }

    /// NDC centre and signed size of a plane rectangle (negative width when
    /// seen from behind, which mirrors the glyph), or `None` when it is
    /// behind the near plane or off screen. The centre is the midpoint of the
    /// projected edges, not the projected centre (perspective moves that), so
    /// neighbours sharing an edge tile without overlap.
    fn rect(&self, center: [f32; 2], half: [f32; 2]) -> Option<([f32; 2], [f32; 2])> {
        let at = |dx: f32, dy: f32| self.project([center[0] + dx, center[1] + dy, 0.0]);
        let (left, right) = (at(-half[0], 0.0)?, at(half[0], 0.0)?);
        let (bottom, top) = (at(0.0, -half[1])?, at(0.0, half[1])?);
        let size = [right[0] - left[0], top[1] - bottom[1]];
        let middle = [(left[0] + right[0]) * 0.5, (bottom[1] + top[1]) * 0.5];
        let visible = (0..2).all(|axis| middle[axis].abs() - size[axis].abs() * 0.5 < 1.0);
        (visible && size.iter().all(|value| value.is_finite())).then_some((middle, size))
    }
}

/// The section's camera pose with a bar framing on top. Head on adds
/// nothing, so the frame-timed schedule's poses are unchanged bit for bit.
fn framed(pose: (f32, f32, f32), framing: Framing) -> (f32, f32, f32) {
    if framing == REST {
        return pose;
    }
    let (yaw, pitch, distance) = pose;
    (
        yaw + framing.yaw,
        pitch + framing.pitch,
        distance * framing.zoom,
    )
}

fn build(
    profile: &RenderProfile,
    frame: ControlFrame,
    _elapsed: f32,
    aspect: f32,
    state: &SceneState,
    instances: &mut Vec<Instance>,
) {
    let clock = state.static_noise.clock(frame);
    build_with(profile, frame, aspect, clock, &Embedded, instances);
}

fn build_with(
    profile: &RenderProfile,
    frame: ControlFrame,
    aspect: f32,
    clock: Clock,
    source: &dyn TextSource,
    instances: &mut Vec<Instance>,
) {
    let aspect = aspect.max(0.01);
    let section = section_at(clock);
    let grid = Grid::new(profile, aspect);
    let view = View::new(framed(camera_pose(section), clock.framing), aspect);
    let noise = Noise::new(frame);
    let energy = f32::from(frame.energy) / 255.0;
    let rainbow = frame.palette % 4 == 3;
    let ink = |brightness: f32, tone: Option<u32>| {
        let mut color = match tone {
            Some(tone) => hue(unit(mix(tone ^ 0x27d4_eb2f))).map(|channel| channel * brightness),
            None => {
                let [red, green, blue, _] = palette(frame.palette, brightness, energy);
                [red, green, blue]
            }
        };
        if section.white {
            color = color.map(|channel| 1.0 - channel);
        }
        [color[0], color[1], color[2], 1.0]
    };
    let quad = |center: [f32; 2], size: [f32; 2], color: [f32; 4], pattern: [f32; 2]| Instance {
        center: [center[0], center[1], 0.0],
        size,
        color,
        rotation: 0.0,
        scene_mix: SCENE_MIX,
        pattern,
    };

    let mut drawn = Vec::with_capacity(RESERVED + grid.len());
    if section.white {
        drawn.push(quad([0.0; 2], [2.0; 2], [1.0; 4], [0.0; 2]));
    }
    let [cell_w, cell_h] = grid.cell();
    let half_width = grid.cols as f32 * cell_w * 0.5;
    if section.style == Style::Pixels {
        let blank = noise.blank + (0.985 - noise.blank) * content_amount(section);
        let pattern = noise_pattern(blank, rainbow, section.white, noise);
        let strip = 2.0 * half_width / NOISE_STRIPS as f32;
        for index in 0..NOISE_STRIPS {
            let x = -half_width + (index as f32 + 0.5) * strip;
            if let Some((center, size)) = view.rect([x, 0.0], [strip * 0.5, 1.0]) {
                drawn.push(quad(center, size, ink(1.0, None), pattern));
            }
        }
    }
    let targets = target(section, grid, source);
    let before = section
        .before_flip()
        .map(|before| target(before, grid, source));
    for (index, &target) in targets.iter().enumerate() {
        let before = before.as_ref().map(|before| before[index]);
        let cell = cell(index, target, before, section, noise);
        if cell.glyph == 0 {
            continue;
        }
        let (column, row) = (index % grid.cols, index / grid.cols);
        let x = -half_width + (column as f32 + 0.5) * cell_w;
        let y = 1.0 - (row as f32 + 0.5) * cell_h;
        let Some((center, size)) = view.rect([x, y], [cell_w * 0.5, cell_h * 0.5]) else {
            continue;
        };
        let (brightness, tone) = match cell.role {
            Role::Target => (1.0, None),
            Role::Cycling => (0.75, rainbow.then_some(cell.tone)),
            _ => (
                0.3 + 0.45 * unit(mix(cell.tone)),
                rainbow.then_some(cell.tone),
            ),
        };
        let size = [size[0] * 5.0 / CELL_W, size[1] * 8.0 / CELL_H];
        drawn.push(quad(
            center,
            size,
            ink(brightness, tone),
            glyph_pattern(cell.glyph),
        ));
    }
    drawn.truncate(profile.instance_count.saturating_sub(instances.len()));
    instances.extend(drawn);
}

/// Same finalizer as `scenes::hash`, keeping the integer. Three copies must
/// stay in sync: this one, `scenes::hash` in `scenes/mod.rs`, and `mix32` in
/// `shader.wgsl` (`pixel_noise` there must also match `gpu_reference`).
fn mix(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^ value >> 16
}

fn unit(value: u32) -> f32 {
    (value >> 8) as f32 / 16_777_216.0
}

/// Fully saturated colour for hue `h` in 0..1.
fn hue(h: f32) -> [f32; 3] {
    [0.0, 2.0 / 3.0, 1.0 / 3.0]
        .map(|offset: f32| (((h + offset).fract() * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0))
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

/// CPU mirror of the shader's Static paths, for the offscreen GPU tests in
/// `capture.rs`.
#[cfg(test)]
pub(crate) mod gpu_reference {
    use super::{Noise, glyph, glyph_pattern, hue, mix, noise_pattern, unit};

    /// Glyph bitmap (bit `row * 5 + column`, row 0 on top) and its pattern.
    pub(crate) fn glyph_bits(character: char) -> (u64, [f32; 2]) {
        let bits = glyph(character).expect("glyph in font");
        (bits, glyph_pattern(bits))
    }

    /// Pattern for pixel noise, rainbow (palette 3) or monochrome, and
    /// inverted as in a white section.
    pub(crate) fn noise(
        blank: f32,
        longest: u32,
        clock: u32,
        beat: u32,
        rainbow: bool,
        inverted: bool,
    ) -> [f32; 2] {
        let noise = Noise {
            blank,
            longest,
            clock,
            beat,
        };
        noise_pattern(blank, rainbow, inverted, noise)
    }

    /// `pixel_noise` in `shader.wgsl`: `None` where the pixel at integer
    /// coordinates (`x`, `y`) is discarded, else the straight-alpha RGBA it
    /// is blended with for instance colour `color`.
    pub(crate) fn pixel(x: u32, y: u32, color: [f32; 4], pattern: [f32; 2]) -> Option<[f32; 4]> {
        let flags = -pattern[0] - 1.0;
        let inverted = flags >= 4.0;
        let code = flags - if inverted { 4.0 } else { 0.0 };
        let rainbow = code >= 2.0;
        let blank = code - if rainbow { 2.0 } else { 0.0 };
        let packed = (pattern[1] + 0.5) as u32;
        let longest = (packed & 15) + 1;
        let clock = packed >> 4;
        let seed = mix(x ^ mix(y.wrapping_add(0x9e37_79b9)));
        let period = 1 + seed % longest;
        let step = ((clock & 0xffff) + (seed >> 8) % period) / period;
        let value = mix(seed ^ mix(step.wrapping_add((clock >> 16).wrapping_mul(0x85eb_ca6b))));
        if unit(value) < blank {
            return None;
        }
        let alpha = color[3] * unit(mix(value));
        let rgb = if rainbow {
            let rgb = hue(unit(mix(value ^ 0x27d4_eb2f)));
            if inverted {
                rgb.map(|channel| 1.0 - channel)
            } else {
                rgb
            }
        } else {
            [color[0], color[1], color[2]]
        };
        Some([rgb[0], rgb[1], rgb[2], alpha])
    }
}

#[cfg(test)]
mod tests;
