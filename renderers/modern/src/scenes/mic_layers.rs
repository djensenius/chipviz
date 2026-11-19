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
//! bursts keep their shape at any aspect ratio.

use super::{colour_field, hash, palette};
use crate::audio::scope::{POINTS, ScopeSnapshot};
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
pub fn synthesise(frame: ControlFrame, elapsed: f32) -> [f32; POINTS] {
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
                let drift = elapsed * (1.2 + band as f32 * 0.9);
                level * (std::f32::consts::TAU * cycles * x + drift).sin()
            })
            .sum::<f32>();
        sum / total * energy
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Burst {
    born: f32,
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
    last_burst: Option<f32>,
    last_elapsed: Option<f32>,
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
        elapsed: f32,
        shown: bool,
    ) {
        let previous = self.last_elapsed.unwrap_or(elapsed);
        let now = previous.max(elapsed);
        let delta = now - previous;
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
            if hidden || slot.is_some_and(|burst| now - burst.born >= LIFETIME_S) {
                *slot = None;
            }
        }
        let rested = self
            .last_burst
            .is_none_or(|last| now - last >= MIN_BURST_GAP_S);
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
                    let age = |slot: &&mut Option<Burst>| slot.map_or(f32::MIN, |b| b.born);
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
        let energy = f32::from(frame.energy) / 255.0;
        let brightness = 0.55 + 0.45 * self.level.min(1.0);
        let point = |outline: &[f32; POINTS], index: usize| {
            let x = (index as f32 / (POINTS - 1) as f32 * 2.0 - 1.0) * RIBBON_HALF_WIDTH * aspect;
            [x, RIBBON_Y + outline[index] * RIBBON_AMPLITUDE]
        };
        let tint = palette(frame.palette, brightness, energy);
        for (thickness, alpha, pass) in RIBBON_PASSES {
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
            for (outline, index) in self
                .wave
                .iter()
                .flat_map(|outline| (0..POINTS - 1).map(move |index| (outline, index)))
            {
                let (a, b) = (point(outline, index), point(outline, index + 1));
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                instances.push(Instance {
                    center: [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, 0.0],
                    size: [dx.hypot(dy) + overlap, thickness],
                    color,
                    rotation: dy.atan2(dx),
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
            let age = now - burst.born;
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
            let elapsed = step as f32 / FPS;
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
        layers.step(quiet(), Some(&snapshot(0.0, 7)), 1.0 / FPS, true);
        assert_eq!(live_bursts(&layers), 0);
        layers.step(quiet(), Some(&snapshot(0.0, 8)), 2.0 / FPS, true);
        assert_eq!(live_bursts(&layers), 1);
        let mut alpha = Vec::new();
        for step in 3..200 {
            layers.step(quiet(), Some(&snapshot(0.0, 8)), step as f32 / FPS, true);
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
            layers.step(frame(4), None, step as f32 / FPS, true);
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
            layers.step(frame, None, step as f32 / FPS, true);
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
        let mut elapsed = 0.0;
        for _ in 0..60 {
            layers.step(quiet(), Some(&snapshot(0.8, 0)), elapsed, true);
            elapsed += 1.0 / FPS;
        }
        let loud = amplitude(&layers);
        assert!((loud - 0.8).abs() < 0.01, "{loud}");
        let mut previous = loud;
        let mut previous_level = layers.level;
        for frame in 0..60 {
            layers.step(quiet(), Some(&snapshot(0.0, 0)), elapsed, true);
            elapsed += 1.0 / FPS;
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
            fallback.step(ControlFrame::procedural(9), None, step as f32 / FPS, true);
        }
        for step in 60..180 {
            fallback.step(quiet(), None, step as f32 / FPS, true);
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
                layers.step(frame, None, step as f32 / FPS, visible);
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
            layers.step(quiet(), None, 0.5 + 1.0 / FPS, true);
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
                let elapsed = step as f32 / FPS;
                layers.step(quiet(), Some(&snapshot(0.5, step)), elapsed, visible);
            }
            assert_eq!(live_bursts(&layers), 0, "{mode:?} shown={visible}");
            layers.set_mode(Mode::Bursts);
            layers.step(quiet(), Some(&snapshot(0.5, 59)), 1.0, true);
            let mut instances = Vec::new();
            layers.build(quiet(), 1.0, &mut instances);
            assert!(instances.is_empty(), "{mode:?}: a held count fires nothing");
            layers.step(quiet(), Some(&snapshot(0.5, 60)), 1.0 + 1.0 / FPS, true);
            assert_eq!(live_bursts(&layers), 1, "{mode:?}: the next onset fires");
        }
    }

    #[test]
    fn ribbon_passes_overlap_opaquely_and_keep_their_width_at_any_aspect() {
        let mut layers = MicLayers::new(Mode::Scope);
        let zigzag = std::array::from_fn(|index| if index % 2 == 0 { -1.0 } else { 1.0 });
        let steep = ScopeSnapshot {
            upper: zigzag,
            lower: zigzag.map(|point| -point),
            onsets: 0,
        };
        for step in 0..120 {
            layers.step(quiet(), Some(&steep), step as f32 / FPS, true);
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
                // Every quad keeps the pass thickness in aspect space, which
                // `transform` maps to the viewport uniformly in x and y.
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
            layers.step(frame, live.then_some(&scope), step as f32 / fps, true);
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
                0.1 + MIN_BURST_GAP_S,
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
        const PIXEL: f32 = 0.004;
        let (columns, rows) = ((2.0 * aspect / PIXEL) as usize, (1.0 / PIXEL) as usize);
        let mut covered = vec![false; columns * rows];
        for quad in instances {
            assert_eq!(quad.pattern, [0.0; 2]);
            assert!(quad.color[3] >= 0.95);
            let (cos, sin) = (quad.rotation.cos(), quad.rotation.sin());
            let reach = 0.5 * quad.size[0].hypot(quad.size[1]);
            let column = |x: f32| ((x + aspect) / PIXEL).clamp(0.0, columns as f32) as usize;
            let row = |y: f32| (-y / PIXEL).clamp(0.0, rows as f32) as usize;
            let (left, right) = (
                column(quad.center[0] - reach),
                column(quad.center[0] + reach),
            );
            let (top, bottom) = (row(quad.center[1] + reach), row(quad.center[1] - reach));
            for y_index in top..(bottom + 1).min(rows) {
                for x_index in left..(right + 1).min(columns) {
                    let x = (x_index as f32 + 0.5) * PIXEL - aspect - quad.center[0];
                    let y = -(y_index as f32 + 0.5) * PIXEL - quad.center[1];
                    let (along, across) = (x * cos + y * sin, y * cos - x * sin);
                    if along.abs() <= 0.5 * quad.size[0] && across.abs() <= 0.5 * quad.size[1] {
                        covered[y_index * columns + x_index] = true;
                    }
                }
            }
        }
        covered.iter().filter(|&&pixel| pixel).count() as f32 * PIXEL * PIXEL
    }

    /// Steps `layers` through 1 s of `audio` (48 kHz, 60 fps) via a real
    /// scope tap and returns the ribbon's drawn area per frame.
    fn drawn_areas(audio: impl Fn(usize) -> f32) -> Vec<f32> {
        use crate::audio::scope::ScopeTap;
        let (rate, aspect) = (48_000.0, 16.0 / 9.0);
        let mut tap = ScopeTap::new(rate);
        let mut layers = MicLayers::new(Mode::Scope);
        let per_frame = (rate / FPS) as usize;
        (0..FPS as usize)
            .map(|frame| {
                let start = frame * per_frame;
                let samples: Vec<f32> = (start..start + per_frame).map(&audio).collect();
                tap.push(&samples);
                layers.step(quiet(), Some(&tap.snapshot(0)), frame as f32 / FPS, true);
                let mut instances = Vec::new();
                layers.build(quiet(), aspect, &mut instances);
                ribbon_area(&instances, aspect)
            })
            .collect()
    }

    /// The ribbon's flash exemption rests on it being thin: loud high
    /// frequencies (a tone or noise) gated on and off at 3 to 10 Hz must
    /// draw no more area, and change it by no more, than a loud slow tone's
    /// ribbon (the thin-line budget), never a filled band.
    #[test]
    fn gated_high_frequencies_keep_the_ribbon_thin() {
        let rate = 48_000.0;
        let sine =
            |hz: f32| move |n: usize| 0.9 * (std::f32::consts::TAU * hz * n as f32 / rate).sin();
        // A steady, full-scale 440 Hz ribbon sets the budget: about nine
        // full-height cycles of thin line.
        let budget = max_area(&drawn_areas(sine(440.0))[30..]);
        let flat = drawn_areas(|_| 0.0)[30];
        assert!(flat > 0.0 && budget > 5.0 * flat, "{flat} {budget}");
        let noise = |n: usize| {
            let hashed = (n as u32).wrapping_mul(0x9e37_79b9).rotate_left(13) ^ 0x5bd1_e995;
            0.9 * (hash(hashed) * 2.0 - 1.0)
        };
        for gate_hz in [3.0, 5.0, 10.0] {
            let gate =
                move |n: usize| ((n as f32 / rate * gate_hz * 2.0) as usize).is_multiple_of(2);
            for (name, signal) in [
                ("4 kHz", &sine(4_000.0) as &dyn Fn(usize) -> f32),
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
                // The gate is seen: the lines do move.
                assert!(high > 1.5 * low, "{name} at {gate_hz} Hz: {low}..{high}");
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
        layers.build(quiet(), 16.0 / 9.0, &mut instances);
        let filled = ribbon_area(&instances, 16.0 / 9.0);
        assert!(filled > 2.0 * budget, "{filled} vs budget {budget}");
    }

    fn max_area(areas: &[f32]) -> f32 {
        areas.iter().copied().fold(0.0, f32::max)
    }
}
