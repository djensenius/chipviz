//! Mic layers: an oscilloscope ribbon and onset bursts drawn over any native
//! scene (issue #67).
//!
//! The layers are an overlay, not a registry scene: the scene byte keeps
//! selecting the scene underneath, and the overlay draws on top with its own
//! `ScreenSpace` uniforms. They are off by default and selected with
//! `--mic-layers` or cycled with `M` ([`Mode`]). The colour field (scene 7)
//! is never overlaid, since its colours must follow its trace alone.
//!
//! Input, per rendered frame:
//! - with live microphone frames, a [`ScopeSnapshot`] from the audio thread's
//!   side channel: the trigger-aligned waveform's upper and lower outlines,
//!   and an onset count whose changes fire bursts;
//! - otherwise (procedural or UDP frames, or a mic that UDP has taken over)
//!   the layers degrade to a ribbon synthesised from the eight spectrum bands
//!   and scaled by energy, and bursts fire when `beat_count` changes.
//!
//! The ribbon eases towards each new waveform (time constant `SMOOTH_S`), so
//! silence decays it smoothly to a flat line instead of cutting. It is two
//! thin lines through the outlines: one line for slow sound, where they
//! coincide, and two for tones too fast for the points, so loud high
//! frequencies never fill a band (see `decimate` in `audio::scope`).
//! Whatever its source, the ribbon is limited once more as it is drawn
//! ([`limit_ribbon`]): it never covers more than `RIBBON_SCREEN_FRACTION` of
//! the screen, at any aspect.
//!
//! Flash design (not a measured compliance claim): a burst is one smooth rise
//! (`ATTACK_S`) and exponential fall, so each is at most one flash. Its
//! colour and peak strength are fixed when it starts, so later palette or
//! energy changes cannot make a live burst flicker. Bursts
//! start at most once every `MIN_BURST_GAP_S` (2 per second, below the
//! 3-per-second WCAG 2.3.1 general flash limit that
//! `shared/specs/colour-field-v0.md` also follows) however fast onsets
//! arrive, and they are soft, partly transparent glows over a small part of
//! the screen. Nothing here inverts or flips the whole screen, so the layers
//! have no `unmeasured-flash` gate. The `flash_model` test checks a CPU model
//! of the burst shapes over black at the maximum onset rate; it is not a
//! photometric measurement of a display or projector.
//!
//! Geometry is built in aspect space (x spans `-aspect..aspect`, y `-1..1`)
//! and flattened by [`transform`], so rotated ribbon segments and round
//! bursts keep their shape at any aspect ratio. In portrait the whole ribbon
//! scales with the width. Overlay time is f64 seconds, so burst timing stays
//! exact however long the renderer runs.

use super::{colour_field, hash, palette};
use crate::audio::scope::{POINTS, ScopeSnapshot, envelope};
use crate::control_frame::ControlFrame;
use crate::scene::{Instance, SCREEN_SPACE_TRANSFORM};

const SMOOTH_S: f32 = 0.05;
const LEVEL_S: f32 = 0.12;
pub const ATTACK_S: f32 = 0.08;
pub const DECAY_S: f32 = 0.35;
pub const LIFETIME_S: f32 = 1.6;
/// Half a second: at most two starts in any 1 s, with margin under the
/// 3-per-second limit for dark rooms and projection.
pub const MIN_BURST_GAP_S: f32 = 0.5;
/// Enough slots for every burst alive at once under the start-rate cap.
const MAX_BURSTS: usize = 4;
const GLOW_ALPHA: f32 = 0.3;
const SPARK_ALPHA: f32 = 0.5;
const SPARKS: usize = 12;
const RIBBON_Y: f32 = -0.55;
const RIBBON_AMPLITUDE: f32 = 0.3;
const RIBBON_HALF_WIDTH: f32 = 0.92;
/// Shapes from `scenes::object` (see `shape_alpha` in `shader.wgsl`).
const GLOW: [f32; 2] = [0.0, -2.0];
/// Ribbon passes, drawn in order: (thickness, alpha, kind). Every pass
/// overlaps its neighbouring segments to hide joint gaps; the halo (a dimmed
/// tint) and the backing are opaque so overlaps cannot stack into dashes,
/// and the core is nearly opaque.
const RIBBON_PASSES: [(f32, f32, Pass); 3] = [
    (0.026, 1.0, Pass::Halo),
    (0.017, 1.0, Pass::Backing),
    (0.008, 0.95, Pass::Core),
];
/// Ribbon lines: the waveform's upper and lower outlines.
const OUTLINES: usize = 2;
/// Most of the screen the drawn ribbon may cover, at any aspect: its thin-line
/// budget (see [`limit_ribbon`]).
pub const RIBBON_SCREEN_FRACTION: f32 = 0.13;
/// Columns the ribbon's area bound is measured in.
const AREA_COLUMNS: usize = 2048;
/// Bisection steps [`limit_ribbon`] takes along its reduction path.
const LIMIT_STEPS: usize = 8;
/// Upper bound on overlay instances: a quad per ribbon segment per outline
/// per pass, plus each burst's glow and sparks.
pub const MAX_INSTANCES: usize =
    RIBBON_PASSES.len() * OUTLINES * (POINTS - 1) + MAX_BURSTS * (1 + SPARKS);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pass {
    Halo,
    Backing,
    Core,
}

/// The overlay's view-projection: flattens like `SCREEN_SPACE_TRANSFORM`,
/// and maps aspect-space x (`-aspect..aspect`) onto the viewport.
pub fn transform(aspect: f32) -> [[f32; 4]; 4] {
    let mut transform = SCREEN_SPACE_TRANSFORM;
    transform[0][0] = 1.0 / aspect.max(0.01);
    transform
}

/// Whether the overlay is drawn (and may start bursts) over `display`:
/// never when off, and never over the colour field, whose colours must follow
/// its trace alone.
pub fn shown(mode: Mode, display: colour_field::Display) -> bool {
    mode != Mode::Off && matches!(display, colour_field::Display::Native(_))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    Scope,
    Bursts,
    Both,
}

impl Mode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "scope" => Some(Self::Scope),
            "bursts" => Some(Self::Bursts),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Scope => "scope",
            Self::Bursts => "bursts",
            Self::Both => "both",
        }
    }

    /// `M`: off, scope, bursts, both, off, ...
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Scope,
            Self::Scope => Self::Bursts,
            Self::Bursts => Self::Both,
            Self::Both => Self::Off,
        }
    }

    fn scope(self) -> bool {
        matches!(self, Self::Scope | Self::Both)
    }

    fn bursts(self) -> bool {
        matches!(self, Self::Bursts | Self::Both)
    }
}

/// Burst brightness at `age` seconds: a smoothstep rise over `ATTACK_S`,
/// then an exponential fall that reaches 0 at `LIFETIME_S`. Continuous,
/// within 0..=1, one rise and one fall.
pub fn burst_envelope(age: f32) -> f32 {
    if !(0.0..LIFETIME_S).contains(&age) {
        return 0.0;
    }
    if age < ATTACK_S {
        let t = age / ATTACK_S;
        return t * t * (3.0 - 2.0 * t);
    }
    // Subtracting the tail value makes the fall end exactly at 0.
    let tail = (-(LIFETIME_S - ATTACK_S) / DECAY_S).exp();
    ((-(age - ATTACK_S) / DECAY_S).exp() - tail) / (1.0 - tail)
}

/// Root mean square of both outlines, at most 1.
fn rms(wave: &[[f32; POINTS]; OUTLINES]) -> f32 {
    let squares = wave.as_flattened().iter().map(|point| point * point);
    (squares.sum::<f32>() / (OUTLINES * POINTS) as f32)
        .sqrt()
        .min(1.0)
}

/// The no-mic ribbon: each spectrum band is a sine with its own number of
/// cycles across the ribbon and its own drift, weighted by its level; the sum
/// is normalised by the total level and scaled by energy, so silence is flat.
pub fn synthesise(frame: ControlFrame, elapsed: f64) -> [f32; POINTS] {
    let levels = frame.spectrum.map(|value| f32::from(value) / 255.0);
    let total = levels.iter().sum::<f32>();
    let energy = f32::from(frame.energy) / 255.0;
    if total <= f32::EPSILON || energy <= 0.0 {
        return [0.0; POINTS];
    }
    std::array::from_fn(|index| {
        let x = index as f32 / (POINTS - 1) as f32;
        let sum = levels
            .iter()
            .enumerate()
            .map(|(band, level)| {
                let cycles = 1.0 + (band as f32 * 0.65).exp2();
                // In f64 and wrapped, so drift stays smooth after days.
                let drift = (elapsed * (1.2 + band as f64 * 0.9)).rem_euclid(std::f64::consts::TAU);
                let drift = drift as f32;
                level * (std::f32::consts::TAU * cycles * x + drift).sin()
            })
            .sum::<f32>();
        sum / total * energy
    })
}

/// The ribbon's size factor at `aspect`: 1 in landscape. In portrait the
/// whole ribbon (its amplitude and strokes as well as its width) shrinks
/// with the width, so it stays the same thin line relative to the screen.
fn ribbon_scale(aspect: f32) -> f32 {
    aspect.clamp(0.01, 1.0)
}

/// Aspect-space position of `outline[index]` at `aspect`.
fn ribbon_point(outline: &[f32; POINTS], index: usize, aspect: f32) -> [f32; 2] {
    let x = (index as f32 / (POINTS - 1) as f32 * 2.0 - 1.0) * RIBBON_HALF_WIDTH * aspect;
    [
        x,
        RIBBON_Y + outline[index] * RIBBON_AMPLITUDE * ribbon_scale(aspect),
    ]
}

/// The quad joining `a` and `b`: centre, size (length plus `overlap`, then
/// `thickness`) and rotation.
fn segment(a: [f32; 2], b: [f32; 2], thickness: f32, overlap: f32) -> ([f32; 2], [f32; 2], f32) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    (
        [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5],
        [dx.hypot(dy) + overlap, thickness],
        dy.atan2(dx),
    )
}

/// The ribbon's area budget at `aspect` (aspect-space units squared, the
/// screen being `4 * aspect`).
pub fn ribbon_budget(aspect: f32) -> f32 {
    RIBBON_SCREEN_FRACTION * 4.0 * aspect.max(0.01)
}

/// An upper bound on the area (aspect-space units squared) the ribbon
/// covers when `wave` is drawn at `aspect`: the union of its halo quads,
/// which contain every other pass's. The ribbon is cut into
/// `AREA_COLUMNS` vertical slabs; in each, every quad contributes the
/// exact height range it covers within the slab (from its corners there
/// and its edges' crossings of the slab's sides), each outline's ranges
/// are joined into one span (an outline is connected, so this only adds
/// its slope across a slab), and the two outlines' spans are united, so
/// coinciding outlines count once.
pub fn ribbon_area_bound(wave: &[[f32; POINTS]; OUTLINES], aspect: f32) -> f32 {
    let aspect = aspect.max(0.01);
    let thickness = RIBBON_PASSES[0].0 * ribbon_scale(aspect);
    let overlap = thickness * 0.5;
    // Past every corner: a point's x plus half the overlap and thickness.
    let reach = RIBBON_HALF_WIDTH * aspect + thickness + overlap;
    let width = 2.0 * reach / AREA_COLUMNS as f32;
    let column = |x: f32| (((x + reach) / width).max(0.0) as usize).min(AREA_COLUMNS - 1);
    let mut spans = [[(f32::INFINITY, f32::NEG_INFINITY); AREA_COLUMNS]; OUTLINES];
    for (outline, spans) in wave.iter().zip(&mut spans) {
        let mut include = |index: usize, y: f32| {
            let span = &mut spans[index];
            *span = (span.0.min(y), span.1.max(y));
        };
        for index in 0..POINTS - 1 {
            let (center, size, rotation) = segment(
                ribbon_point(outline, index, aspect),
                ribbon_point(outline, index + 1, aspect),
                thickness,
                overlap,
            );
            let (cos, sin) = (rotation.cos(), rotation.sin());
            let (along, across) = (size[0] * 0.5, size[1] * 0.5);
            let corners = [
                (-along, -across),
                (along, -across),
                (along, across),
                (-along, across),
            ]
            .map(|(a, c)| [center[0] + a * cos - c * sin, center[1] + a * sin + c * cos]);
            let (mut left, mut right) = (f32::INFINITY, f32::NEG_INFINITY);
            for corner in corners {
                include(column(corner[0]), corner[1]);
                (left, right) = (left.min(corner[0]), right.max(corner[0]));
            }
            // Where edges cross a side shared by two slabs, both get it.
            for side in column(left) + 1..=column(right) {
                let x = -reach + side as f32 * width;
                for (edge, p) in corners.iter().enumerate() {
                    let q = corners[(edge + 1) % 4];
                    if p[0] != q[0] && (p[0] - x) * (q[0] - x) <= 0.0 {
                        let y = p[1] + (x - p[0]) / (q[0] - p[0]) * (q[1] - p[1]);
                        include(side - 1, y);
                        include(side, y);
                    }
                }
            }
        }
    }
    let length = |span: (f32, f32)| (span.1 - span.0).max(0.0);
    let [first, second] = &spans;
    first
        .iter()
        .zip(second)
        .map(|(&a, &b)| length(a) + length(b) - length((a.0.max(b.0), a.1.min(b.1))))
        .sum::<f32>()
        * width
}

/// The final, guaranteed limit on the drawn ribbon: `wave` unchanged if its
/// [`ribbon_area_bound`] at `aspect` is within [`ribbon_budget`], else the
/// least reduction that is. Reductions follow one path: blend towards the
/// outlines' envelope (see `scope::envelope`), so alternating outlines
/// become steady lines, then scale that towards the centre line, ending at
/// a flat line, which is always within budget. Bisection along the path
/// keeps an end known to be within budget, so whatever the input (live,
/// synthesised, a torn read, any aspect) the result is.
pub fn limit_ribbon(wave: &[[f32; POINTS]; OUTLINES], aspect: f32) -> [[f32; POINTS]; OUTLINES] {
    let budget = ribbon_budget(aspect);
    if ribbon_area_bound(wave, aspect) <= budget {
        return *wave;
    }
    let (high, low) = envelope(&wave[0], &wave[1]);
    let along = |position: f32| -> [[f32; POINTS]; OUTLINES] {
        let (blend, gain) = (position.min(1.0), (2.0 - position).clamp(0.0, 1.0));
        [(&wave[0], &high), (&wave[1], &low)].map(|(raw, held)| {
            std::array::from_fn(|index| (raw[index] + (held[index] - raw[index]) * blend) * gain)
        })
    };
    let (mut over, mut within) = (0.0_f32, 2.0_f32);
    for _ in 0..LIMIT_STEPS {
        let middle = 0.5 * (over + within);
        if ribbon_area_bound(&along(middle), aspect) <= budget {
            within = middle;
        } else {
            over = middle;
        }
    }
    along(within)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Burst {
    born: f64,
    serial: u32,
    /// Peak intensity, from the energy at onset.
    strength: f32,
    /// RGB from the palette and energy at onset.
    color: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Trigger {
    Onsets(u32),
    Beat(u16),
}

/// Overlay state, stepped once per rendered frame.
#[derive(Clone, Debug)]
pub struct MicLayers {
    mode: Mode,
    /// Upper and lower outlines.
    wave: [[f32; POINTS]; OUTLINES],
    level: f32,
    /// Whether the ribbon was drawn last step; `wave` is stale otherwise.
    ribbon_shown: bool,
    bursts: [Option<Burst>; MAX_BURSTS],
    serial: u32,
    last_trigger: Option<Trigger>,
    last_burst: Option<f64>,
    /// Overlay time, in f64 seconds so burst timing stays exact however
    /// long the renderer runs (f32 seconds step by 125 ms after 12 days).
    last_elapsed: Option<f64>,
    /// Tests only: every burst at this aspect-space spot instead of a hashed
    /// one, so a single point sees every burst.
    #[cfg(test)]
    pinned: Option<[f32; 2]>,
}

impl MicLayers {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            wave: [[0.0; POINTS]; OUTLINES],
            level: 0.0,
            ribbon_shown: false,
            bursts: [None; MAX_BURSTS],
            serial: 0,
            last_trigger: None,
            last_burst: None,
            last_elapsed: None,
            #[cfg(test)]
            pinned: None,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Advances to `elapsed`: eases the ribbon towards this frame's waveform
    /// (`live`, or synthesised from `frame`) and starts a burst on a new onset
    /// (live) or beat (fallback). The ribbon is only updated while it is drawn
    /// (`shown` and the mode includes the scope), so nothing is synthesised
    /// for a hidden ribbon; when it reappears it starts at the current
    /// waveform instead of easing from a stale one. Bursts only start while
    /// they are drawn (`shown`, see [`shown`], and the mode includes
    /// bursts), so none pop in mid-fade. Live bursts are dropped while hidden
    /// (the start-rate cap is kept), so none reappear mid-fade either. Time
    /// never runs backwards here.
    pub fn step(
        &mut self,
        frame: ControlFrame,
        live: Option<&ScopeSnapshot>,
        elapsed: f64,
        shown: bool,
    ) {
        let previous = self.last_elapsed.unwrap_or(elapsed);
        let now = previous.max(elapsed);
        let delta = (now - previous) as f32;
        self.last_elapsed = Some(now);

        let ribbon = shown && self.mode.scope();
        if ribbon {
            let target = match live {
                Some(snapshot) => [snapshot.upper, snapshot.lower],
                None => [synthesise(frame, now); OUTLINES],
            };
            if self.ribbon_shown {
                let blend = -(-delta / SMOOTH_S).exp_m1();
                let points = self.wave.as_flattened_mut().iter_mut();
                for (point, target) in points.zip(target.as_flattened()) {
                    *point += (target - *point) * blend;
                }
                self.level += (rms(&self.wave) - self.level) * -(-delta / LEVEL_S).exp_m1();
            } else {
                self.wave = target;
                self.level = rms(&self.wave);
            }
        }
        self.ribbon_shown = ribbon;

        let trigger = match live {
            Some(snapshot) => Trigger::Onsets(snapshot.onsets),
            None => Trigger::Beat(frame.beat_count),
        };
        // A change of source re-seeds instead of counting as an onset.
        let onset = matches!(
            (self.last_trigger, trigger),
            (Some(Trigger::Onsets(a)), Trigger::Onsets(b)) if a != b
        ) || matches!(
            (self.last_trigger, trigger),
            (Some(Trigger::Beat(a)), Trigger::Beat(b)) if a != b
        );
        self.last_trigger = Some(trigger);

        let hidden = !shown || !self.mode.bursts();
        for slot in &mut self.bursts {
            if hidden || slot.is_some_and(|burst| now - burst.born >= f64::from(LIFETIME_S)) {
                *slot = None;
            }
        }
        let rested = self
            .last_burst
            .is_none_or(|last| now - last >= f64::from(MIN_BURST_GAP_S));
        if onset && rested && !hidden {
            let energy = f32::from(frame.energy) / 255.0;
            let [red, green, blue, _] = palette(frame.palette.wrapping_add(2), 1.0, energy);
            let burst = Burst {
                born: now,
                serial: self.serial,
                strength: 0.55 + 0.45 * energy,
                color: [red, green, blue],
            };
            self.serial = self.serial.wrapping_add(1);
            self.last_burst = Some(now);
            let slot = self
                .bursts
                .iter_mut()
                .min_by(|a, b| {
                    let age = |slot: &&mut Option<Burst>| slot.map_or(f64::MIN, |b| b.born);
                    age(a).total_cmp(&age(b))
                })
                .expect("MAX_BURSTS > 0");
            *slot = Some(burst);
        }
    }

    /// Appends at most [`MAX_INSTANCES`] `ScreenSpace` instances for the
    /// enabled layers, given the viewport aspect (width / height).
    pub fn build(&self, frame: ControlFrame, aspect: f32, instances: &mut Vec<Instance>) {
        let aspect = aspect.max(0.01);
        if self.mode.bursts() {
            self.build_bursts(aspect, instances);
        }
        if self.mode.scope() {
            self.build_ribbon(frame, aspect, instances);
        }
    }

    fn build_ribbon(&self, frame: ControlFrame, aspect: f32, instances: &mut Vec<Instance>) {
        // The final, guaranteed limit, on exactly what is drawn.
        self.push_ribbon(&limit_ribbon(&self.wave, aspect), frame, aspect, instances);
    }

    /// Draws `wave` as the ribbon, as it is (unlimited).
    fn push_ribbon(
        &self,
        wave: &[[f32; POINTS]; OUTLINES],
        frame: ControlFrame,
        aspect: f32,
        instances: &mut Vec<Instance>,
    ) {
        let energy = f32::from(frame.energy) / 255.0;
        let brightness = 0.55 + 0.45 * self.level.min(1.0);
        let scale = ribbon_scale(aspect);
        let tint = palette(frame.palette, brightness, energy);
        for (thickness, alpha, pass) in RIBBON_PASSES {
            let thickness = thickness * scale;
            let (color, overlap) = match pass {
                Pass::Halo => (
                    [tint[0] * 0.3, tint[1] * 0.3, tint[2] * 0.3, alpha],
                    thickness * 0.5,
                ),
                // A dark edge keeps the core readable over same-hued scenes.
                Pass::Backing => ([0.0, 0.0, 0.0, alpha], thickness * 0.5),
                // The core leans towards white for contrast with the palette.
                Pass::Core => (
                    [
                        0.4 * tint[0] + 0.6 * brightness,
                        0.4 * tint[1] + 0.6 * brightness,
                        0.4 * tint[2] + 0.6 * brightness,
                        alpha,
                    ],
                    thickness * 0.5,
                ),
            };
            // Each pass covers both outlines before the next, so a core is
            // never hidden by the other outline's halo.
            for (outline, index) in wave
                .iter()
                .flat_map(|outline| (0..POINTS - 1).map(move |index| (outline, index)))
            {
                let (center, size, rotation) = segment(
                    ribbon_point(outline, index, aspect),
                    ribbon_point(outline, index + 1, aspect),
                    thickness,
                    overlap,
                );
                instances.push(Instance {
                    center: [center[0], center[1], 0.0],
                    size,
                    color,
                    rotation,
                    scene_mix: 0.0,
                    pattern: [0.0; 2],
                });
            }
        }
    }

    fn build_bursts(&self, aspect: f32, instances: &mut Vec<Instance>) {
        let Some(now) = self.last_elapsed else {
            return;
        };
        let mut bursts: Vec<Burst> = self.bursts.iter().flatten().copied().collect();
        bursts.sort_by(|a, b| a.born.total_cmp(&b.born));
        for burst in bursts {
            let age = (now - burst.born) as f32;
            let envelope = burst_envelope(age) * burst.strength;
            if envelope <= 0.0 {
                continue;
            }
            let seed = burst.serial.wrapping_mul(7);
            let center = [
                (hash(seed) * 1.3 - 0.65) * aspect,
                hash(seed + 1) * 0.7 - 0.05,
            ];
            #[cfg(test)]
            let center = self.pinned.unwrap_or(center);
            let radius = 0.06 + 0.42 * (1.0 - (-age / 0.3).exp());
            let [red, green, blue] = burst.color;
            let mut color = [red, green, blue, envelope * GLOW_ALPHA];
            instances.push(Instance {
                center: [center[0], center[1], 0.0],
                size: [radius * 1.6, radius * 1.6],
                color,
                rotation: 0.0,
                scene_mix: 0.0,
                pattern: GLOW,
            });
            color[3] = envelope * SPARK_ALPHA;
            let spin = hash(seed + 2) * std::f32::consts::TAU;
            for spark in 0..SPARKS {
                let angle = spin + spark as f32 / SPARKS as f32 * std::f32::consts::TAU;
                let size = 0.05 * (0.4 + envelope);
                instances.push(Instance {
                    center: [
                        center[0] + angle.cos() * radius,
                        center[1] + angle.sin() * radius,
                        0.0,
                    ],
                    size: [size, size],
                    color,
                    rotation: 0.0,
                    scene_mix: 0.0,
                    pattern: GLOW,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: f32 = 60.0;

    fn quiet() -> ControlFrame {
        ControlFrame {
            energy: 0,
            spectrum: [0; 8],
            ..ControlFrame::procedural(0)
        }
    }

    /// A slow tone: both outlines on one curve.
    fn snapshot(amplitude: f32, onsets: u32) -> ScopeSnapshot {
        let points = std::array::from_fn(|index| {
            amplitude * (index as f32 / POINTS as f32 * std::f32::consts::TAU * 4.0).sin()
        });
        ScopeSnapshot {
            upper: points,
            lower: points,
            onsets,
        }
    }

    fn amplitude(layers: &MicLayers) -> f32 {
        layers
            .wave
            .as_flattened()
            .iter()
            .fold(0.0, |peak, point| peak.max(point.abs()))
    }

    fn live_bursts(layers: &MicLayers) -> usize {
        layers.bursts.iter().flatten().count()
    }

    #[test]
    fn modes_parse_and_cycle() {
        for mode in [Mode::Off, Mode::Scope, Mode::Bursts, Mode::Both] {
            assert_eq!(Mode::parse(mode.name()), Some(mode));
            assert_eq!(Mode::parse(&mode.name().to_uppercase()), Some(mode));
        }
        assert_eq!(Mode::parse("on"), None);
        assert_eq!(Mode::default(), Mode::Off);
        let cycle: Vec<_> = std::iter::successors(Some(Mode::Off), |mode| Some(mode.next()))
            .take(5)
            .collect();
        assert_eq!(
            cycle,
            [Mode::Off, Mode::Scope, Mode::Bursts, Mode::Both, Mode::Off]
        );
    }

    #[test]
    fn envelope_rises_smoothly_once_then_decays_to_zero() {
        assert_eq!(burst_envelope(-0.01), 0.0);
        assert_eq!(burst_envelope(0.0), 0.0);
        assert!((burst_envelope(ATTACK_S) - 1.0).abs() < 1e-6);
        assert_eq!(burst_envelope(LIFETIME_S), 0.0);
        let samples: Vec<f32> = (0..=200)
            .map(|step| burst_envelope(step as f32 * LIFETIME_S / 200.0))
            .collect();
        let peak = samples
            .iter()
            .position(|&value| value >= 1.0 - 1e-6)
            .unwrap();
        assert!(samples[..=peak].windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(samples[peak..].windows(2).all(|pair| pair[1] <= pair[0]));
        assert!(samples.iter().all(|value| (0.0..=1.0).contains(value)));
        // No jumps: the largest change in a 60 fps frame stays well under 1.
        let mut largest = 0.0_f32;
        for step in 0..(LIFETIME_S * FPS) as usize + 2 {
            let age = step as f32 / FPS;
            largest = largest.max((burst_envelope(age + 1.0 / FPS) - burst_envelope(age)).abs());
        }
        assert!(largest < 0.45, "{largest}");
        // Lifetime is the tail end: nearly gone a little before it.
        assert!(burst_envelope(LIFETIME_S - 0.05) < 0.01);
    }

    #[test]
    fn onset_storms_start_at_most_two_bursts_per_second() {
        let mut layers = MicLayers::new(Mode::Both);
        let mut starts = Vec::new();
        for step in 0..600u32 {
            let elapsed = step as f64 / f64::from(FPS);
            // A new onset every frame, far faster than the cap.
            layers.step(quiet(), Some(&snapshot(0.5, step)), elapsed, true);
            if layers.last_burst == Some(elapsed) {
                starts.push(elapsed);
            }
            assert!(live_bursts(&layers) <= MAX_BURSTS);
            let mut instances = Vec::new();
            layers.build(quiet(), 16.0 / 9.0, &mut instances);
            assert!(instances.len() <= MAX_INSTANCES);
        }
        for (index, &start) in starts.iter().enumerate() {
            let window = starts[index..]
                .iter()
                .take_while(|&&later| later < start + 1.0)
                .count();
            assert!(window <= 2, "{window} bursts within 1 s of {start}");
        }
        assert!(starts.len() >= 18, "{}", starts.len());
    }

    #[test]
    fn live_onsets_fire_bursts_that_decay_and_expire() {
        let mut layers = MicLayers::new(Mode::Bursts);
        layers.step(quiet(), Some(&snapshot(0.0, 7)), 0.0, true);
        assert_eq!(live_bursts(&layers), 0, "the first count only seeds");
        layers.step(quiet(), Some(&snapshot(0.0, 7)), 1.0 / f64::from(FPS), true);
        assert_eq!(live_bursts(&layers), 0);
        layers.step(quiet(), Some(&snapshot(0.0, 8)), 2.0 / f64::from(FPS), true);
        assert_eq!(live_bursts(&layers), 1);
        let mut alpha = Vec::new();
        for step in 3..200 {
            layers.step(
                quiet(),
                Some(&snapshot(0.0, 8)),
                step as f64 / f64::from(FPS),
                true,
            );
            let mut instances = Vec::new();
            layers.build(quiet(), 1.0, &mut instances);
            alpha.push(instances.first().map_or(0.0, |glow| glow.color[3]));
        }
        let peak = alpha.iter().copied().fold(0.0, f32::max);
        assert!(peak > 0.1 && peak <= GLOW_ALPHA, "{peak}");
        assert_eq!(alpha.last(), Some(&0.0));
        assert_eq!(live_bursts(&layers), 0);
    }

    #[test]
    fn fallback_bursts_follow_beat_count_and_ignore_held_frames() {
        let mut layers = MicLayers::new(Mode::Both);
        let frame = |beat_count| ControlFrame {
            beat_count,
            flags: 0x01,
            ..ControlFrame::procedural(3)
        };
        // A held frame keeps its beat flag but not a new count: no repeats.
        for step in 0..30 {
            layers.step(frame(4), None, step as f64 / f64::from(FPS), true);
        }
        assert_eq!(live_bursts(&layers), 0);
        layers.step(frame(5), None, 0.5, true);
        assert_eq!(live_bursts(&layers), 1);
        // Switching to live input re-seeds instead of firing.
        layers.step(frame(6), Some(&snapshot(0.3, 99)), 1.0, true);
        assert_eq!(live_bursts(&layers), 1);
    }

    #[test]
    fn fallback_ribbon_comes_from_spectrum_and_energy() {
        let frame = ControlFrame::procedural(40);
        let wave = synthesise(frame, 1.0);
        let peak = wave
            .iter()
            .fold(0.0_f32, |peak, point| peak.max(point.abs()));
        let energy = f32::from(frame.energy) / 255.0;
        assert!(peak > 0.1 && peak <= energy + 1e-6, "{peak} vs {energy}");
        assert_eq!(synthesise(quiet(), 1.0), [0.0; POINTS]);
        assert_eq!(synthesise(frame, 1.0), wave, "deterministic");
        assert_ne!(synthesise(frame, 1.5), wave, "drifts over time");

        let mut layers = MicLayers::new(Mode::Scope);
        for step in 0..60 {
            layers.step(frame, None, step as f64 / f64::from(FPS), true);
        }
        assert!(amplitude(&layers) > 0.1);
        let mut instances = Vec::new();
        let aspect = 16.0 / 9.0;
        layers.build(frame, aspect, &mut instances);
        assert_eq!(
            instances.len(),
            RIBBON_PASSES.len() * OUTLINES * (POINTS - 1)
        );
        assert!(instances.iter().all(|instance| {
            instance.center[0].abs() <= aspect && instance.center[1].abs() <= 1.0
        }));
    }

    #[test]
    fn silence_decays_the_ribbon_smoothly() {
        let mut layers = MicLayers::new(Mode::Scope);
        let mut elapsed = 0.0_f64;
        for _ in 0..60 {
            layers.step(quiet(), Some(&snapshot(0.8, 0)), elapsed, true);
            elapsed += 1.0 / f64::from(FPS);
        }
        let loud = amplitude(&layers);
        assert!((loud - 0.8).abs() < 0.01, "{loud}");
        let mut previous = loud;
        let mut previous_level = layers.level;
        for frame in 0..60 {
            layers.step(quiet(), Some(&snapshot(0.0, 0)), elapsed, true);
            elapsed += 1.0 / f64::from(FPS);
            let now = amplitude(&layers);
            assert!(now <= previous, "frame {frame}: {now} > {previous}");
            // Eases out: no frame drops more than a third of the remainder.
            assert!(now >= previous * 0.7 - 1e-6, "frame {frame}: cut to {now}");
            assert!(layers.level <= previous_level + 1e-6);
            previous = now;
            previous_level = layers.level;
        }
        assert!(previous < 1e-4, "{previous}");
        // The no-mic fallback decays the same way when energy drops to 0.
        let mut fallback = MicLayers::new(Mode::Scope);
        for step in 0..60 {
            fallback.step(
                ControlFrame::procedural(9),
                None,
                step as f64 / f64::from(FPS),
                true,
            );
        }
        for step in 60..180 {
            fallback.step(quiet(), None, step as f64 / f64::from(FPS), true);
        }
        assert!(amplitude(&fallback) < 1e-4);
    }

    #[test]
    fn a_hidden_ribbon_is_not_synthesised_and_starts_current_when_shown() {
        let frame = ControlFrame::procedural(40);
        // Off, bursts only, or over the colour field: nothing is computed.
        for (mode, visible) in [(Mode::Off, true), (Mode::Bursts, true), (Mode::Both, false)] {
            let mut layers = MicLayers::new(mode);
            for step in 0..30 {
                layers.step(frame, None, step as f64 / f64::from(FPS), visible);
            }
            assert_eq!(
                layers.wave, [[0.0; POINTS]; OUTLINES],
                "{mode:?} shown={visible}"
            );
            assert_eq!(layers.level, 0.0);
            // Shown: the first drawn frame is the current waveform, not an
            // ease from the stale (flat) one.
            layers.set_mode(Mode::Scope);
            layers.step(frame, None, 0.5, true);
            assert_eq!(layers.wave, [synthesise(frame, 0.5); OUTLINES], "{mode:?}");
            assert_eq!(layers.level, rms(&layers.wave));
            // Then it eases as usual: one frame towards silence keeps
            // exp(-1 / (60 SMOOTH_S)) of it.
            let shown_at = amplitude(&layers);
            layers.step(quiet(), None, 0.5 + 1.0 / f64::from(FPS), true);
            let kept = amplitude(&layers) / shown_at;
            let expected = (-1.0 / (FPS * SMOOTH_S)).exp();
            assert!((kept - expected).abs() < 1e-3, "{mode:?}: {kept}");
        }
        // Hiding again keeps the last waveform untouched, and live input also
        // starts current on return.
        let mut layers = MicLayers::new(Mode::Scope);
        layers.step(quiet(), Some(&snapshot(0.8, 0)), 0.0, true);
        let outlines = |snapshot: ScopeSnapshot| [snapshot.upper, snapshot.lower];
        assert_eq!(layers.wave, outlines(snapshot(0.8, 0)));
        let held = layers.wave;
        layers.step(quiet(), Some(&snapshot(0.2, 0)), 0.1, false);
        assert_eq!(layers.wave, held);
        layers.step(quiet(), Some(&snapshot(0.2, 0)), 0.2, true);
        assert_eq!(layers.wave, outlines(snapshot(0.2, 0)));
    }

    #[test]
    fn off_draws_nothing_and_time_never_rewinds() {
        let mut layers = MicLayers::new(Mode::Off);
        layers.step(quiet(), Some(&snapshot(0.5, 1)), 2.0, true);
        layers.step(quiet(), Some(&snapshot(0.5, 2)), 2.5, true);
        layers.step(quiet(), Some(&snapshot(0.5, 2)), 2.6, true);
        let mut instances = Vec::new();
        layers.build(quiet(), 1.0, &mut instances);
        assert!(instances.is_empty());
        layers.step(quiet(), Some(&snapshot(0.5, 2)), 1.0, true);
        assert_eq!(layers.last_elapsed, Some(2.6));
    }

    #[test]
    fn shown_only_over_native_scenes_and_never_when_off() {
        let native = colour_field::Display::Native(3);
        let field = colour_field::Display::ColourField(colour_field::Trace {
            mode: 0,
            family: 0,
            layout: 0,
            combo: 0,
            rot: 0,
            lvl: [0; 3],
        });
        for mode in [Mode::Scope, Mode::Bursts, Mode::Both] {
            assert!(shown(mode, native), "{mode:?}");
            assert!(!shown(mode, field), "{mode:?} over the colour field");
        }
        assert!(!shown(Mode::Off, native));
        assert!(!shown(Mode::Off, field));
    }

    #[test]
    fn no_bursts_start_while_hidden_so_none_pop_in() {
        // Hidden by the display (colour field) or by a mode without bursts:
        // onsets only re-seed, so showing bursts later starts empty.
        for (mode, visible) in [(Mode::Both, false), (Mode::Off, true), (Mode::Scope, true)] {
            let mut layers = MicLayers::new(mode);
            for step in 0..60u32 {
                let elapsed = step as f64 / f64::from(FPS);
                layers.step(quiet(), Some(&snapshot(0.5, step)), elapsed, visible);
            }
            assert_eq!(live_bursts(&layers), 0, "{mode:?} shown={visible}");
            layers.set_mode(Mode::Bursts);
            layers.step(quiet(), Some(&snapshot(0.5, 59)), 1.0, true);
            let mut instances = Vec::new();
            layers.build(quiet(), 1.0, &mut instances);
            assert!(instances.is_empty(), "{mode:?}: a held count fires nothing");
            layers.step(
                quiet(),
                Some(&snapshot(0.5, 60)),
                1.0 + 1.0 / f64::from(FPS),
                true,
            );
            assert_eq!(live_bursts(&layers), 1, "{mode:?}: the next onset fires");
        }
    }

    #[test]
    fn ribbon_passes_overlap_opaquely_and_scale_with_the_width_in_portrait() {
        let mut layers = MicLayers::new(Mode::Scope);
        let zigzag = std::array::from_fn(|index| if index % 2 == 0 { -1.0 } else { 1.0 });
        let steep = ScopeSnapshot {
            upper: zigzag,
            lower: zigzag.map(|point| -point),
            onsets: 0,
        };
        for step in 0..120 {
            layers.step(quiet(), Some(&steep), step as f64 / f64::from(FPS), true);
        }
        let segments = POINTS - 1;
        for aspect in [1.0, 16.0 / 9.0, 4.0 / 3.0, 0.5] {
            let mut instances = Vec::new();
            layers.build(quiet(), aspect, &mut instances);
            let lines = OUTLINES * segments;
            assert_eq!(instances.len(), RIBBON_PASSES.len() * lines);
            for (pass, (thickness, alpha, kind)) in RIBBON_PASSES.into_iter().enumerate() {
                // Overlapping translucent quads would stack into dashes.
                assert!(alpha >= 0.95, "{kind:?} is too translucent to overlap");
                let quads = &instances[pass * lines..(pass + 1) * lines];
                // Every quad keeps the pass thickness in aspect space (scaled
                // with the width in portrait), which `transform` maps to the
                // viewport uniformly in x and y.
                let thickness = thickness * ribbon_scale(aspect);
                assert!(quads.iter().all(|quad| quad.size[1] == thickness));
                for pair in quads.chunks(segments).flat_map(|line| line.windows(2)) {
                    let end = |quad: &Instance, sign: f32| {
                        let half = quad.size[0] * 0.5 * sign;
                        [
                            quad.center[0] + half * quad.rotation.cos(),
                            quad.center[1] + half * quad.rotation.sin(),
                        ]
                    };
                    let (a, b) = (end(&pair[0], 1.0), end(&pair[1], -1.0));
                    let gap = (a[0] - b[0]).hypot(a[1] - b[1]);
                    assert!(gap > 0.0, "{kind:?} overlaps to hide joints");
                }
            }
            let span = instances[segments - 1].center[0] - instances[0].center[0];
            let expected =
                2.0 * RIBBON_HALF_WIDTH * aspect * (segments - 1) as f32 / segments as f32;
            assert!((span - expected).abs() < 1e-3, "{span} vs {expected}");
        }
        let flat = transform(16.0 / 9.0);
        assert!((flat[0][0] - 9.0 / 16.0).abs() < 1e-6);
        assert_eq!(flat[1], SCREEN_SPACE_TRANSFORM[1]);
        assert_eq!(flat[3], SCREEN_SPACE_TRANSFORM[3]);
    }

    /// Linear relative luminance over black at `point` (aspect space), by
    /// compositing `instances` in order with the shader's soft-glow coverage
    /// (`shape_alpha` shape 1). Instances must be unrotated glows.
    fn luminance(instances: &[Instance], point: [f32; 2]) -> f32 {
        let mut rgb = [0.0_f32; 3];
        for instance in instances {
            assert_eq!(instance.pattern, GLOW);
            let local = [
                (point[0] - instance.center[0]) / instance.size[0],
                (point[1] - instance.center[1]) / instance.size[1],
            ];
            if local[0].abs() > 0.5 || local[1].abs() > 0.5 {
                continue;
            }
            let fall = (1.0 - 2.0 * local[0].hypot(local[1])).clamp(0.0, 1.0);
            let alpha = fall * fall * instance.color[3];
            for (channel, value) in rgb.iter_mut().zip(instance.color) {
                *channel = *channel * (1.0 - alpha) + value.clamp(0.0, 1.0) * alpha;
            }
        }
        0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
    }

    /// Opposing changes of at least `step` in `series` (WCAG 2.3.1 counts a
    /// flash as a pair of them), as the indices where each completes.
    fn transitions(series: &[f32], step: f32) -> Vec<usize> {
        let mut found = Vec::new();
        let (mut low, mut high) = (series[0], series[0]);
        let mut rising: Option<bool> = None;
        for (index, &value) in series.iter().enumerate() {
            low = low.min(value);
            high = high.max(value);
            if rising != Some(true) && value - low >= step {
                found.push(index);
                rising = Some(true);
                (low, high) = (value, value);
            } else if rising != Some(false) && high - value >= step {
                found.push(index);
                rising = Some(false);
                (low, high) = (value, value);
            }
        }
        found
    }

    #[test]
    fn transitions_count_opposing_swings_only() {
        let series = [0.0, 0.2, 0.25, 0.1, 0.0, 0.05, 0.3, 0.3];
        assert_eq!(transitions(&series, 0.1), [1, 3, 6]);
        assert!(transitions(&[0.0, 0.05, 0.0, 0.09], 0.1).is_empty());
    }

    /// The most flashes (pairs of opposing changes of at least 0.1 relative
    /// luminance, see [`transitions`]) in any 1 s at any probe, and the
    /// brightest value seen, over 6 s at `fps`. `input` gives each frame and
    /// whether the onset arrives live (a new onset count every frame) or by
    /// fallback (a new `beat_count` every frame).
    fn worst_flashes(
        fps: f32,
        pinned: Option<[f32; 2]>,
        input: impl Fn(usize) -> (ControlFrame, bool),
    ) -> (usize, usize, f32) {
        let aspect = 16.0 / 9.0;
        let (columns, rows) = (48, 27);
        let mut probes: Vec<[f32; 2]> = (0..columns * rows)
            .map(|probe| {
                [
                    ((probe % columns) as f32 + 0.5) / columns as f32 * 2.0 * aspect - aspect,
                    ((probe / columns) as f32 + 0.5) / rows as f32 * 2.0 - 1.0,
                ]
            })
            .collect();
        // The pinned spot itself and points across its glow.
        probes.extend((0..=8).map(|step| [step as f32 * 0.03, 0.0]));
        let mut layers = MicLayers::new(Mode::Bursts);
        layers.pinned = pinned;
        let frames = 6 * fps as usize;
        let mut series = vec![Vec::with_capacity(frames); probes.len()];
        let mut brightest = 0.0_f32;
        for step in 0..frames {
            let (mut frame, live) = input(step);
            frame.beat_count = step as u16;
            let scope = snapshot(0.0, step as u32);
            layers.step(
                frame,
                live.then_some(&scope),
                step as f64 / f64::from(fps),
                true,
            );
            let mut instances = Vec::new();
            layers.build(frame, aspect, &mut instances);
            for (point, values) in probes.iter().zip(&mut series) {
                let value = luminance(&instances, *point);
                brightest = brightest.max(value);
                values.push(value);
            }
        }
        let window = fps as usize;
        let mut worst = 0;
        for values in &series {
            let found = transitions(values, 0.1);
            for (index, &start) in found.iter().enumerate() {
                let within = found[index..]
                    .iter()
                    .take_while(|&&later| later < start + window)
                    .count();
                worst = worst.max(within);
            }
        }
        (worst / 2, worst, brightest)
    }

    /// A CPU model of the bursts, not a photometric measurement: onsets every
    /// frame at full energy, the brightest burst palette, over black. With
    /// hashed positions few bursts land on any one point, so the second case
    /// pins every burst to one spot: there each burst is a flash, and only
    /// the start-rate cap keeps it to 2 flashes (pairs of opposing changes of
    /// at least 0.1 relative luminance) in any 1 s, under the limit of 3.
    #[test]
    fn flash_model_stays_under_three_flashes_per_second_at_max_onset_rate() {
        // Burst colours use `frame.palette + 2`; palette 0 is white.
        let frame = ControlFrame {
            energy: 255,
            palette: 2,
            ..quiet()
        };
        for pinned in [None, Some([0.0, 0.0])] {
            let (flashes, worst, brightest) = worst_flashes(FPS, pinned, |_| (frame, true));
            assert!(
                brightest > 0.1,
                "the model must see the bursts: {brightest}"
            );
            assert!(
                flashes <= 2,
                "{pinned:?}: {flashes} flashes ({worst} opposing changes) in 1 s"
            );
            if pinned.is_some() {
                // Every burst flashes the pinned spot, so only the cap bounds
                // the count: it must reach the capped rate.
                assert_eq!(flashes, 2, "pinned bursts must each flash: {worst}");
            }
        }
    }

    /// Input that changes every frame (palettes 0 and 2, full and zero
    /// energy, as from UDP) must not re-colour live bursts into extra
    /// flashes: each burst keeps its onset colour and strength, so the cap
    /// still bounds the rate below the limit of 3 at any frame rate.
    #[test]
    fn flash_model_holds_while_palette_and_energy_change_every_frame() {
        let changing = |step: usize| ControlFrame {
            palette: if step.is_multiple_of(2) { 0 } else { 2 },
            energy: if step % 4 < 2 { 255 } else { 0 },
            ..quiet()
        };
        for fps in [60.0, 120.0, 240.0] {
            for live in [true, false] {
                for pinned in [None, Some([0.0, 0.0])] {
                    let (flashes, worst, brightest) =
                        worst_flashes(fps, pinned, |step| (changing(step), live));
                    let case = format!("{fps} Hz, live {live}, pinned {pinned:?}");
                    assert!(brightest > 0.1, "{case}: unseen bursts: {brightest}");
                    assert!(
                        flashes <= 2,
                        "{case}: {flashes} flashes ({worst} opposing changes) in 1 s"
                    );
                }
            }
        }
    }

    #[test]
    fn hiding_drops_live_bursts_but_keeps_the_start_cap() {
        for hide in [
            |layers: &mut MicLayers| (false, layers.mode()),
            |_: &mut MicLayers| (true, Mode::Off),
            |_: &mut MicLayers| (true, Mode::Scope),
        ] {
            let mut layers = MicLayers::new(Mode::Both);
            layers.step(quiet(), Some(&snapshot(0.0, 0)), 0.0, true);
            layers.step(quiet(), Some(&snapshot(0.0, 1)), 0.1, true);
            assert_eq!(live_bursts(&layers), 1);
            // Hidden by the colour field (not shown) or by cycling the mode.
            let (shown, mode) = hide(&mut layers);
            layers.set_mode(mode);
            layers.step(quiet(), Some(&snapshot(0.0, 1)), 0.2, shown);
            assert_eq!(live_bursts(&layers), 0, "{mode:?} shown={shown}");
            // Shown again mid-fade: nothing reappears, and an onset inside
            // the start gap of the dropped burst still waits.
            layers.set_mode(Mode::Both);
            layers.step(quiet(), Some(&snapshot(0.0, 2)), 0.3, true);
            let mut instances = Vec::new();
            layers.build(quiet(), 1.0, &mut instances);
            assert_eq!(live_bursts(&layers), 0, "{mode:?}: start cap kept");
            assert_eq!(
                instances.len(),
                RIBBON_PASSES.len() * OUTLINES * (POINTS - 1)
            );
            layers.step(
                quiet(),
                Some(&snapshot(0.0, 3)),
                0.1 + f64::from(MIN_BURST_GAP_S),
                true,
            );
            assert_eq!(live_bursts(&layers), 1, "{mode:?}: fires after the gap");
        }
    }

    /// Drawn area (aspect-space units squared) of the ribbon: every pass is
    /// opaque or nearly so (`ScreenSpace` pattern 0 draws its colour), so
    /// each covered pixel replaces the scene, as a bright core or halo over
    /// dark scenes and a black backing over bright ones. Only the lower half
    /// is sampled, which holds the whole ribbon.
    fn ribbon_area(instances: &[Instance], aspect: f32) -> f32 {
        let cell = 0.004 * ribbon_scale(aspect);
        let (columns, rows) = ((2.0 * aspect / cell) as usize, (1.0 / cell) as usize);
        let mut covered = vec![false; columns * rows];
        for quad in instances {
            assert_eq!(quad.pattern, [0.0; 2]);
            assert!(quad.color[3] >= 0.95);
            let (cos, sin) = (quad.rotation.cos(), quad.rotation.sin());
            // The rotated rectangle's bounding box.
            let (half_width, half_height) = (0.5 * quad.size[0], 0.5 * quad.size[1]);
            let reach_x = cos.abs() * half_width + sin.abs() * half_height + cell;
            let reach_y = sin.abs() * half_width + cos.abs() * half_height + cell;
            let column = |x: f32| ((x + aspect) / cell).clamp(0.0, columns as f32) as usize;
            let row = |y: f32| (-y / cell).clamp(0.0, rows as f32) as usize;
            let (left, right) = (
                column(quad.center[0] - reach_x),
                column(quad.center[0] + reach_x),
            );
            let (top, bottom) = (row(quad.center[1] + reach_y), row(quad.center[1] - reach_y));
            for y_index in top..(bottom + 1).min(rows) {
                for x_index in left..(right + 1).min(columns) {
                    let x = (x_index as f32 + 0.5) * cell - aspect - quad.center[0];
                    let y = -(y_index as f32 + 0.5) * cell - quad.center[1];
                    let (along, across) = (x * cos + y * sin, y * cos - x * sin);
                    if along.abs() <= 0.5 * quad.size[0] && across.abs() <= 0.5 * quad.size[1] {
                        covered[y_index * columns + x_index] = true;
                    }
                }
            }
        }
        covered.iter().filter(|&&covered| covered).count() as f32 * cell * cell
    }

    const ASPECT: f32 = 16.0 / 9.0;
    /// Absolute cap on the ribbon's drawn area: 13% of the 16:9 screen
    /// (2 * aspect by 2 units), which pins the thin-line budget so it cannot
    /// quietly grow with the reference ribbon.
    const AREA_CAP: f32 = 0.13 * 4.0 * ASPECT;

    /// Which frames [`drawn`] rasterises.
    #[derive(Clone, Copy, PartialEq)]
    enum Areas {
        Every,
        /// Only the frame whose drawn outlines cover the most height,
        /// appended as an extra last element.
        Worst,
    }

    /// Steps `layers` through `seconds` of `audio` at `rate` (60 fps) via a
    /// real scope tap; returns each frame's published-snapshot coverage
    /// (see `scope::coverage`) and drawn area (0 where `areas` skips it).
    fn drawn(
        rate: f32,
        seconds: f32,
        areas: Areas,
        audio: impl Fn(usize) -> f32,
    ) -> Vec<(f32, f32)> {
        use crate::audio::scope::{ScopeTap, coverage};
        let mut tap = ScopeTap::new(rate);
        let mut layers = MicLayers::new(Mode::Scope);
        let per_frame = (rate / FPS) as usize;
        let area = |layers: &MicLayers| {
            let mut instances = Vec::new();
            layers.build(quiet(), ASPECT, &mut instances);
            ribbon_area(&instances, ASPECT)
        };
        let mut most = (-1.0, layers.clone());
        let mut frames: Vec<(f32, f32)> = (0..(seconds * FPS) as usize)
            .map(|frame| {
                let start = frame * per_frame;
                let samples: Vec<f32> = (start..start + per_frame).map(&audio).collect();
                tap.push(&samples);
                let snapshot = tap.snapshot(0);
                layers.step(
                    quiet(),
                    Some(&snapshot),
                    frame as f64 / f64::from(FPS),
                    true,
                );
                let published = coverage(&snapshot.upper, &snapshot.lower);
                match areas {
                    Areas::Every => (published, area(&layers)),
                    Areas::Worst => {
                        let shown = coverage(&layers.wave[0], &layers.wave[1]);
                        if shown > most.0 {
                            most = (shown, layers.clone());
                        }
                        (published, 0.0)
                    }
                }
            })
            .collect();
        if areas == Areas::Worst {
            frames.push((0.0, area(&most.1)));
        }
        frames
    }

    /// Drawn areas over 1 s of `audio` at 48 kHz, one per frame.
    fn drawn_areas(audio: impl Fn(usize) -> f32) -> Vec<f32> {
        drawn(48_000.0, 1.0, Areas::Every, audio)
            .into_iter()
            .map(|(_, area)| area)
            .collect()
    }

    fn sine(rate: f32, hz: f32) -> impl Fn(usize) -> f32 {
        move |n: usize| 0.9 * (std::f32::consts::TAU * hz * n as f32 / rate).sin()
    }

    /// A steady, full-scale 440 Hz ribbon sets the thin-line budget: about
    /// 19 full-height cycles of thin line at 48 kHz, 20 at 44.1 kHz (the
    /// larger of the rates is taken).
    fn budget() -> f32 {
        [44_100.0, 48_000.0, 96_000.0]
            .into_iter()
            .map(|rate| {
                let settled = &drawn(rate, 0.3, Areas::Every, sine(rate, 440.0))[10..];
                max_area(&settled.iter().map(|&(_, area)| area).collect::<Vec<_>>())
            })
            .fold(0.0, f32::max)
    }

    /// The ribbon's flash exemption rests on it being thin: loud high
    /// frequencies (a tone or noise) gated on and off at 3 to 10 Hz must
    /// draw no more area, and change it by no more, than a loud slow tone's
    /// ribbon (the thin-line budget), never a filled band.
    #[test]
    fn gated_high_frequencies_keep_the_ribbon_thin() {
        let rate = 48_000.0;
        let budget = budget();
        let flat = drawn_areas(|_| 0.0)[30];
        assert!(flat > 0.0 && budget > 5.0 * flat, "{flat} {budget}");
        assert!(budget <= AREA_CAP, "budget {budget} over cap {AREA_CAP}");
        let noise = |n: usize| {
            let hashed = (n as u32).wrapping_mul(0x9e37_79b9).rotate_left(13) ^ 0x5bd1_e995;
            0.9 * (hash(hashed) * 2.0 - 1.0)
        };
        for gate_hz in [3.0, 5.0, 10.0] {
            let gate =
                move |n: usize| ((n as f32 / rate * gate_hz * 2.0) as usize).is_multiple_of(2);
            for (name, signal) in [
                ("4 kHz", &sine(rate, 4_000.0) as &dyn Fn(usize) -> f32),
                ("noise", &noise),
            ] {
                let areas = drawn_areas(|n| if gate(n) { signal(n) } else { 0.0 });
                let low = areas.iter().copied().fold(f32::INFINITY, f32::min);
                let high = max_area(&areas);
                assert!(
                    high <= budget,
                    "{name} gated at {gate_hz} Hz draws {high}, budget {budget}"
                );
                assert!(
                    high - low <= budget,
                    "{name} at {gate_hz} Hz: {low}..{high}"
                );
                // The gate is seen: one flat line opens into two (a loud
                // tone's envelope) or a ragged pair (noise).
                assert!(high > 1.25 * low, "{name} at {gate_hz} Hz: {low}..{high}");
            }
        }
        // A filled band (one line zigzagging between the extremes, as a
        // per-point alternating envelope drew) is well over budget.
        let mut layers = MicLayers::new(Mode::Scope);
        let zigzag = std::array::from_fn(|index| if index % 2 == 0 { 0.9 } else { -0.9 });
        let band = ScopeSnapshot {
            upper: zigzag,
            lower: zigzag,
            onsets: 0,
        };
        layers.step(quiet(), Some(&band), 0.0, true);
        let mut instances = Vec::new();
        layers.push_ribbon(&layers.wave, quiet(), ASPECT, &mut instances);
        let filled = ribbon_area(&instances, ASPECT);
        assert!(filled > 2.0 * budget, "{filled} vs budget {budget}");
        // Which the final limiter draws within the cap.
        instances.clear();
        layers.build(quiet(), ASPECT, &mut instances);
        let limited = ribbon_area(&instances, ASPECT);
        assert!(limited <= AREA_CAP, "{limited} vs cap {AREA_CAP}");
    }

    /// Every loud tone from 0.5 to 8 kHz (every 100 Hz, plus those whose
    /// period is a whole number of half-points, where raw outlines
    /// alternate between extremes) at 44.1, 48 and 96 kHz, steady and gated
    /// at 3, 5 and 10 Hz, publishes outlines within the scope's coverage
    /// bound in every frame, and draws within the thin-line budget and the
    /// absolute cap. Each run rasterises its frame whose drawn outlines
    /// cover the most height. Gated runs are for every third tone, to keep
    /// the test quick.
    #[test]
    fn mid_and_high_tones_never_fill_a_band() {
        use crate::audio::scope::MAX_COVERAGE;
        let budget = budget();
        let limit = budget.min(AREA_CAP);
        for rate in [44_100.0_f32, 48_000.0, 96_000.0] {
            let divisors = (2..=16)
                .map(|half_points| rate / (4.0 * half_points as f32))
                .filter(|hz| (500.0..=8_000.0).contains(hz));
            let tones: Vec<f32> = (5..=80)
                .map(|step| step as f32 * 100.0)
                .chain(divisors)
                .collect();
            for (index, &hz) in tones.iter().enumerate() {
                let tone = sine(rate, hz);
                // Steady, then gated for every third tone; the gated runs
                // cover a full 3 Hz cycle (20 frames).
                let gates: &[Option<f32>] = if index % 3 == 0 {
                    &[None, Some(3.0), Some(5.0), Some(10.0)]
                } else {
                    &[None]
                };
                for &gate_hz in gates {
                    let gate = move |n: usize| {
                        gate_hz.is_none_or(|gate_hz| {
                            ((n as f32 / rate * gate_hz * 2.0) as usize).is_multiple_of(2)
                        })
                    };
                    let seconds = if gate_hz.is_none() { 0.1 } else { 0.35 };
                    let frames = drawn(rate, seconds, Areas::Worst, |n| {
                        if gate(n) { tone(n) } else { 0.0 }
                    });
                    for &(published, area) in &frames {
                        assert!(
                            published <= MAX_COVERAGE,
                            "{hz} Hz at {rate} gated at {gate_hz:?} Hz: coverage {published}"
                        );
                        assert!(
                            area <= limit,
                            "{hz} Hz at {rate} gated at {gate_hz:?} Hz: area {area}, limit {limit}"
                        );
                    }
                }
            }
        }
    }

    /// The pinned thin-line cap at `aspect`: 13% of the screen (`2 * aspect`
    /// by 2 units), so the limiter's budget cannot quietly grow.
    fn cap(aspect: f32) -> f32 {
        0.13 * 4.0 * aspect
    }

    /// Asserts `wave`, limited at draw time, is within the limiter's own
    /// budget and rasterises within [`cap`] at `aspect`.
    fn assert_drawn_within_cap(wave: &[[f32; POINTS]; OUTLINES], aspect: f32, what: &str) {
        assert!(ribbon_budget(aspect) <= cap(aspect));
        let limited = limit_ribbon(wave, aspect);
        let bound = ribbon_area_bound(&limited, aspect);
        assert!(
            bound <= ribbon_budget(aspect),
            "{what} at {aspect}: bound {bound}"
        );
        let mut layers = MicLayers::new(Mode::Scope);
        layers.wave = *wave;
        let mut instances = Vec::new();
        layers.build(quiet(), aspect, &mut instances);
        let area = ribbon_area(&instances, aspect);
        assert!(
            area <= cap(aspect),
            "{what} at {aspect}: area {area}, cap {}",
            cap(aspect)
        );
    }

    /// Publishes `seconds` of `audio` at `rate` through a scope tap, at 60
    /// fps; returns every snapshot.
    fn published(rate: f32, seconds: f32, audio: impl Fn(usize) -> f32) -> Vec<ScopeSnapshot> {
        let mut tap = crate::audio::scope::ScopeTap::new(rate);
        let per_frame = (rate / FPS) as usize;
        (0..(seconds * FPS) as usize)
            .map(|frame| {
                let start = frame * per_frame;
                tap.push(&(start..start + per_frame).map(&audio).collect::<Vec<_>>());
                tap.snapshot(0)
            })
            .collect()
    }

    const PORTRAITS: [f32; 2] = [9.0 / 16.0, 1.0 / 10.0];

    /// A sparse pulse train's full envelope covers more than the scope's
    /// first pass allows; the drawn ribbon stays within the cap anyway.
    #[test]
    fn a_pulse_train_is_drawn_within_the_cap() {
        use crate::audio::scope::{MAX_COVERAGE, coverage};
        let pulses = published(48_000.0, 0.3, |n| match n % 80 {
            0 => -1.0,
            1 => 1.0,
            _ => 0.0,
        });
        let first_pass = pulses
            .iter()
            .map(|snapshot| coverage(&snapshot.upper, &snapshot.lower))
            .fold(0.0, f32::max);
        assert!(first_pass > MAX_COVERAGE, "{first_pass}");
        for snapshot in pulses.iter().skip(5).step_by(4) {
            for aspect in [ASPECT, PORTRAITS[0]] {
                assert_drawn_within_cap(&[snapshot.upper, snapshot.lower], aspect, "pulses");
            }
        }
    }

    /// A read racing a publish mixes two snapshots that were each limited
    /// by the scope's first pass; what is drawn is limited again.
    #[test]
    fn a_torn_read_is_drawn_within_the_cap() {
        let square = |hz: f32| {
            move |n: usize| {
                if (n as f32 * hz / 48_000.0).fract() < 0.5 {
                    0.9
                } else {
                    -0.9
                }
            }
        };
        let last = |hz| *published(48_000.0, 0.3, square(hz)).last().unwrap();
        let (a, b) = (last(440.0), last(450.0));
        let tears: [&dyn Fn(usize) -> bool; 3] = [
            &|index| index < POINTS / 2,
            &|index| (index / 8).is_multiple_of(2),
            &|index| index.is_multiple_of(2),
        ];
        for tear in tears {
            let pick = |a: &[f32; POINTS], b: &[f32; POINTS]| -> [f32; POINTS] {
                std::array::from_fn(|index| if tear(index) { a[index] } else { b[index] })
            };
            let torn = [pick(&a.upper, &b.upper), pick(&a.lower, &b.lower)];
            for aspect in [ASPECT, 1.0, PORTRAITS[0]] {
                assert_drawn_within_cap(&torn, aspect, "torn");
            }
        }
    }

    /// The synthesised (procedural and UDP) ribbon goes through the same
    /// final limit: full energy in only the top band is the densest it
    /// draws.
    #[test]
    fn a_synthesised_top_band_is_drawn_within_the_cap() {
        use crate::audio::scope::{MAX_COVERAGE, coverage};
        let mut spectrum = [0; 8];
        spectrum[7] = 255;
        let frame = ControlFrame {
            energy: 255,
            spectrum,
            ..ControlFrame::procedural(0)
        };
        let mut layers = MicLayers::new(Mode::Scope);
        let mut densest = 0.0_f32;
        for step in 0..40 {
            layers.step(frame, None, f64::from(step) / f64::from(FPS), true);
            densest = densest.max(coverage(&layers.wave[0], &layers.wave[1]));
            if step % 8 == 7 {
                for aspect in [ASPECT, PORTRAITS[0], PORTRAITS[1]] {
                    assert_drawn_within_cap(&layers.wave, aspect, "band 7");
                }
            }
        }
        // Denser than the scope's first pass lets live input be.
        assert!(densest > MAX_COVERAGE, "{densest}");
    }

    /// In portrait the whole ribbon scales with the width, so it keeps the
    /// thin-line budget as a fraction of the screen: a loud 440 Hz tone
    /// draws as is, and a filled band is limited within the cap.
    #[test]
    fn portrait_ribbons_keep_the_thin_line_budget() {
        let tone = *published(48_000.0, 0.3, sine(48_000.0, 440.0))
            .last()
            .unwrap();
        let tone = [tone.upper, tone.lower];
        let zigzag = std::array::from_fn(|index| if index % 2 == 0 { 1.0 } else { -1.0 });
        for aspect in PORTRAITS {
            assert_eq!(limit_ribbon(&tone, aspect), tone, "440 Hz at {aspect}");
            assert_drawn_within_cap(&tone, aspect, "440 Hz");
            assert_drawn_within_cap(&[zigzag, zigzag], aspect, "zigzag");
            // A flat line stays a thin line: under 2% of the screen.
            let flat = ribbon_area_bound(&[[0.0; POINTS]; OUTLINES], aspect);
            assert!(flat < 0.02 * 4.0 * aspect, "{aspect}: {flat}");
        }
    }

    /// The limiter's last resort, a flat line at the centre, is within
    /// budget at any aspect, so its bisection always ends within budget.
    #[test]
    fn a_flat_ribbon_is_within_budget_at_any_aspect() {
        for aspect in [
            0.001,
            0.01,
            0.1,
            9.0 / 16.0,
            1.0,
            ASPECT,
            4.0,
            21.0 / 9.0,
            100.0,
        ] {
            let flat = ribbon_area_bound(&[[0.0; POINTS]; OUTLINES], aspect);
            assert!(flat <= 0.25 * ribbon_budget(aspect), "{aspect}: {flat}");
        }
    }

    /// A loud 440 Hz tone, the reference thin line, is drawn exactly as
    /// published at 16:9; a loud 6 kHz tone stays visibly loud.
    #[test]
    fn the_final_limiter_keeps_slow_tones_and_loud_high_tones() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            for snapshot in published(rate, 0.3, sine(rate, 440.0)).iter().skip(5) {
                let wave = [snapshot.upper, snapshot.lower];
                assert_eq!(limit_ribbon(&wave, ASPECT), wave, "440 Hz at {rate}");
            }
        }
        let snapshot = *published(48_000.0, 0.3, |n| {
            (std::f32::consts::TAU * 6_000.0 * n as f32 / 48_000.0).sin()
        })
        .last()
        .unwrap();
        let [upper, lower] = limit_ribbon(&[snapshot.upper, snapshot.lower], ASPECT);
        let middle = POINTS / 4..3 * POINTS / 4;
        assert!(upper[middle.clone()].iter().all(|&value| value >= 0.5));
        assert!(lower[middle].iter().all(|&value| value <= -0.5));
    }

    /// A thousand seeded random and adversarial outline pairs (noise,
    /// zigzags, pulses, extremes, walks, tones and crossed outlines) at
    /// several aspects all draw within the budget after the final limit;
    /// some are rasterised against the cap too.
    #[test]
    fn the_final_limiter_holds_for_any_outlines() {
        let mut seed = 0x6d69_635f_u32;
        let mut next = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            hash(seed)
        };
        let aspects = [0.1, 9.0 / 16.0, 1.0, ASPECT, 3.0];
        for case in 0..1_000 {
            let kind = case % 7;
            let (a, b, c) = (next(), next(), next());
            let period = 2 + (a * 6.0) as usize;
            let mut walk = 0.0_f32;
            let mut outline = |offset: f32| -> [f32; POINTS] {
                std::array::from_fn(|index| match kind {
                    0 => next() * 2.0 - 1.0,
                    1 => {
                        if (index / period.div_ceil(2)).is_multiple_of(2) {
                            b
                        } else {
                            -c
                        }
                    }
                    2 => {
                        if (index % (period * 7)) < 1 + (c * 3.0) as usize {
                            b * 2.0 - 1.0
                        } else {
                            0.0
                        }
                    }
                    3 => {
                        if next() < 0.5 {
                            -1.0
                        } else {
                            1.0
                        }
                    }
                    4 => {
                        walk = (walk + (next() - 0.5) * 0.6).clamp(-1.0, 1.0);
                        walk
                    }
                    5 => {
                        let cycles = index as f32 * (0.02 + a * 0.6);
                        b * (std::f32::consts::TAU * cycles + offset).sin()
                    }
                    _ => {
                        if index.is_multiple_of(2) {
                            offset.signum()
                        } else {
                            -offset.signum()
                        }
                    }
                })
            };
            let wave = [outline(1.0), outline(-1.0)];
            let aspect = aspects[case % aspects.len()];
            let limited = limit_ribbon(&wave, aspect);
            let bound = ribbon_area_bound(&limited, aspect);
            assert!(
                bound <= ribbon_budget(aspect),
                "case {case} at {aspect}: {bound}"
            );
            if case % 40 == 0 {
                assert_drawn_within_cap(&wave, aspect, &format!("case {case}"));
            }
        }
    }

    /// Overlay time is f64: after 30 days a burst's attack still advances
    /// frame by frame (f32 seconds would step by 250 ms), and the start-rate
    /// cap still holds.
    #[test]
    fn burst_timing_stays_exact_after_thirty_days() {
        let start = 30.0 * 86_400.0;
        let mut layers = MicLayers::new(Mode::Bursts);
        let mut starts = Vec::new();
        let mut previous: Option<(u32, f32)> = None;
        for step in 0..600u32 {
            let elapsed = start + f64::from(step) / f64::from(FPS);
            layers.step(quiet(), Some(&snapshot(0.5, step)), elapsed, true);
            if layers.last_burst == Some(elapsed) {
                starts.push(elapsed);
            }
            let newest = layers
                .bursts
                .iter()
                .flatten()
                .max_by(|a, b| a.born.total_cmp(&b.born))
                .copied();
            if let Some(burst) = newest {
                let age = (elapsed - burst.born) as f32;
                let frames = ((elapsed - burst.born) * f64::from(FPS)).round();
                assert!((age - frames as f32 / FPS).abs() < 1e-4, "{age}");
                let envelope = burst_envelope(age);
                if let Some((serial, before)) = previous
                    && serial == burst.serial
                {
                    assert!((envelope - before).abs() < 0.45, "{before} -> {envelope}");
                }
                previous = Some((burst.serial, envelope));
            }
        }
        for pair in starts.windows(2) {
            assert!(
                pair[1] - pair[0] >= f64::from(MIN_BURST_GAP_S) - 1e-6,
                "{pair:?}"
            );
        }
        assert!(starts.len() >= 18, "{}", starts.len());
    }

    fn max_area(areas: &[f32]) -> f32 {
        areas.iter().copied().fold(0.0, f32::max)
    }
}
