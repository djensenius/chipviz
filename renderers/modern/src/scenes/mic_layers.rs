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
//!   side channel: the trigger-aligned waveform, and an onset count whose
//!   changes fire bursts;
//! - otherwise (procedural or UDP frames, or a mic that UDP has taken over)
//!   the layers degrade to a ribbon synthesised from the eight spectrum bands
//!   and scaled by energy, and bursts fire when `beat_count` changes.
//!
//! The ribbon eases towards each new waveform (time constant `SMOOTH_S`), so
//! silence decays it smoothly to a flat line instead of cutting.
//!
//! Flash safety: a burst is one smooth rise (`ATTACK_S`) and exponential
//! fall, so each is at most one flash. Bursts start at most once every
//! `MIN_BURST_GAP_S` (3 per second, the WCAG 2.3.1 general flash limit that
//! `shared/specs/colour-field-v0.md` also follows) however fast onsets
//! arrive, and they are soft, partly transparent glows over a small part of
//! the screen. Nothing here inverts or flips the whole screen, so the layers
//! need no `unmeasured-flash` gate.

use super::{hash, palette};
use crate::audio::scope::{POINTS, ScopeSnapshot};
use crate::control_frame::ControlFrame;
use crate::scene::Instance;

const SMOOTH_S: f32 = 0.05;
const LEVEL_S: f32 = 0.12;
pub const ATTACK_S: f32 = 0.08;
pub const DECAY_S: f32 = 0.35;
pub const LIFETIME_S: f32 = 1.6;
/// Just over a third of a second, so even four starts span more than 1 s.
pub const MIN_BURST_GAP_S: f32 = 0.34;
/// Enough slots for every burst alive at once under the start-rate cap.
const MAX_BURSTS: usize = 6;
const SPARKS: usize = 12;
const RIBBON_Y: f32 = -0.55;
const RIBBON_AMPLITUDE: f32 = 0.3;
const RIBBON_HALF_WIDTH: f32 = 0.92;
/// Shapes from `scenes::object` (see `shape_alpha` in `shader.wgsl`).
const GLOW: [f32; 2] = [0.0, -2.0];
/// Upper bound on overlay instances: two quads per ribbon segment, plus each
/// burst's glow and sparks.
pub const MAX_INSTANCES: usize = 2 * (POINTS - 1) + MAX_BURSTS * (1 + SPARKS);

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
    strength: f32,
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
    wave: [f32; POINTS],
    level: f32,
    bursts: [Option<Burst>; MAX_BURSTS],
    serial: u32,
    last_trigger: Option<Trigger>,
    last_burst: Option<f32>,
    last_elapsed: Option<f32>,
}

impl MicLayers {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            wave: [0.0; POINTS],
            level: 0.0,
            bursts: [None; MAX_BURSTS],
            serial: 0,
            last_trigger: None,
            last_burst: None,
            last_elapsed: None,
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
    /// (live) or beat (fallback). Runs whatever the mode, so toggling a layer
    /// on shows its current state. Time never runs backwards here.
    pub fn step(&mut self, frame: ControlFrame, live: Option<&ScopeSnapshot>, elapsed: f32) {
        let previous = self.last_elapsed.unwrap_or(elapsed);
        let now = previous.max(elapsed);
        let delta = now - previous;
        self.last_elapsed = Some(now);

        let target = match live {
            Some(snapshot) => snapshot.points,
            None => synthesise(frame, now),
        };
        let blend = -(-delta / SMOOTH_S).exp_m1();
        for (point, target) in self.wave.iter_mut().zip(target) {
            *point += (target - *point) * blend;
        }
        let rms = (self.wave.iter().map(|point| point * point).sum::<f32>() / POINTS as f32).sqrt();
        self.level += (rms.min(1.0) - self.level) * -(-delta / LEVEL_S).exp_m1();

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

        for slot in &mut self.bursts {
            if slot.is_some_and(|burst| now - burst.born >= LIFETIME_S) {
                *slot = None;
            }
        }
        let rested = self
            .last_burst
            .is_none_or(|last| now - last >= MIN_BURST_GAP_S);
        if onset && rested {
            let energy = f32::from(frame.energy) / 255.0;
            let burst = Burst {
                born: now,
                serial: self.serial,
                strength: 0.55 + 0.45 * energy,
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
            self.build_bursts(frame, aspect, instances);
        }
        if self.mode.scope() {
            self.build_ribbon(frame, instances);
        }
    }

    fn build_ribbon(&self, frame: ControlFrame, instances: &mut Vec<Instance>) {
        let energy = f32::from(frame.energy) / 255.0;
        let brightness = 0.55 + 0.45 * self.level.min(1.0);
        let point = |index: usize| {
            let x = (index as f32 / (POINTS - 1) as f32 * 2.0 - 1.0) * RIBBON_HALF_WIDTH;
            [x, RIBBON_Y + self.wave[index] * RIBBON_AMPLITUDE]
        };
        for (thickness, alpha) in [(0.028, 0.16), (0.006, 0.9)] {
            let mut color = palette(frame.palette, brightness, energy);
            color[3] = alpha;
            for index in 0..POINTS - 1 {
                let (a, b) = (point(index), point(index + 1));
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                instances.push(Instance {
                    center: [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, 0.0],
                    // Overlap neighbours slightly so joints have no gaps.
                    size: [dx.hypot(dy) + thickness * 0.5, thickness],
                    color,
                    rotation: dy.atan2(dx),
                    scene_mix: 0.0,
                    pattern: [0.0; 2],
                });
            }
        }
    }

    fn build_bursts(&self, frame: ControlFrame, aspect: f32, instances: &mut Vec<Instance>) {
        let Some(now) = self.last_elapsed else {
            return;
        };
        let energy = f32::from(frame.energy) / 255.0;
        let mut bursts: Vec<Burst> = self.bursts.iter().flatten().copied().collect();
        bursts.sort_by(|a, b| a.born.total_cmp(&b.born));
        for burst in bursts {
            let age = now - burst.born;
            let envelope = burst_envelope(age) * burst.strength;
            if envelope <= 0.0 {
                continue;
            }
            let seed = burst.serial.wrapping_mul(7);
            let center = [hash(seed) * 1.3 - 0.65, hash(seed + 1) * 0.7 - 0.05];
            let radius = 0.06 + 0.42 * (1.0 - (-age / 0.3).exp());
            let mut color = palette(frame.palette.wrapping_add(2), 1.0, energy);
            color[3] = envelope * 0.3;
            instances.push(Instance {
                center: [center[0], center[1], 0.0],
                size: [radius * 1.6 / aspect, radius * 1.6],
                color,
                rotation: 0.0,
                scene_mix: 0.0,
                pattern: GLOW,
            });
            color[3] = envelope * 0.85;
            let spin = hash(seed + 2) * std::f32::consts::TAU;
            for spark in 0..SPARKS {
                let angle = spin + spark as f32 / SPARKS as f32 * std::f32::consts::TAU;
                let size = 0.05 * (0.4 + envelope);
                instances.push(Instance {
                    center: [
                        center[0] + angle.cos() * radius / aspect,
                        center[1] + angle.sin() * radius,
                        0.0,
                    ],
                    size: [size / aspect, size],
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

    fn snapshot(amplitude: f32, onsets: u32) -> ScopeSnapshot {
        ScopeSnapshot {
            points: std::array::from_fn(|index| {
                amplitude * (index as f32 / POINTS as f32 * std::f32::consts::TAU * 4.0).sin()
            }),
            onsets,
        }
    }

    fn amplitude(layers: &MicLayers) -> f32 {
        layers
            .wave
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
    fn onset_storms_start_at_most_three_bursts_per_second() {
        let mut layers = MicLayers::new(Mode::Both);
        let mut starts = Vec::new();
        for step in 0..600u32 {
            let elapsed = step as f32 / FPS;
            // A new onset every frame, far faster than the cap.
            layers.step(quiet(), Some(&snapshot(0.5, step)), elapsed);
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
            assert!(window <= 3, "{window} bursts within 1 s of {start}");
        }
        assert!(starts.len() >= 25, "{}", starts.len());
    }

    #[test]
    fn live_onsets_fire_bursts_that_decay_and_expire() {
        let mut layers = MicLayers::new(Mode::Bursts);
        layers.step(quiet(), Some(&snapshot(0.0, 7)), 0.0);
        assert_eq!(live_bursts(&layers), 0, "the first count only seeds");
        layers.step(quiet(), Some(&snapshot(0.0, 7)), 1.0 / FPS);
        assert_eq!(live_bursts(&layers), 0);
        layers.step(quiet(), Some(&snapshot(0.0, 8)), 2.0 / FPS);
        assert_eq!(live_bursts(&layers), 1);
        let mut alpha = Vec::new();
        for step in 3..200 {
            layers.step(quiet(), Some(&snapshot(0.0, 8)), step as f32 / FPS);
            let mut instances = Vec::new();
            layers.build(quiet(), 1.0, &mut instances);
            alpha.push(instances.first().map_or(0.0, |glow| glow.color[3]));
        }
        let peak = alpha.iter().copied().fold(0.0, f32::max);
        assert!(peak > 0.1 && peak <= 0.3, "{peak}");
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
            layers.step(frame(4), None, step as f32 / FPS);
        }
        assert_eq!(live_bursts(&layers), 0);
        layers.step(frame(5), None, 0.5);
        assert_eq!(live_bursts(&layers), 1);
        // Switching to live input re-seeds instead of firing.
        layers.step(frame(6), Some(&snapshot(0.3, 99)), 1.0);
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
            layers.step(frame, None, step as f32 / FPS);
        }
        assert!(amplitude(&layers) > 0.1);
        let mut instances = Vec::new();
        layers.build(frame, 16.0 / 9.0, &mut instances);
        assert_eq!(instances.len(), 2 * (POINTS - 1));
        assert!(instances.iter().all(|instance| {
            instance.center[0].abs() <= 1.0 && instance.center[1].abs() <= 1.0
        }));
    }

    #[test]
    fn silence_decays_the_ribbon_smoothly() {
        let mut layers = MicLayers::new(Mode::Scope);
        let mut elapsed = 0.0;
        for _ in 0..60 {
            layers.step(quiet(), Some(&snapshot(0.8, 0)), elapsed);
            elapsed += 1.0 / FPS;
        }
        let loud = amplitude(&layers);
        assert!((loud - 0.8).abs() < 0.01, "{loud}");
        let mut previous = loud;
        let mut previous_level = layers.level;
        for frame in 0..60 {
            layers.step(quiet(), Some(&snapshot(0.0, 0)), elapsed);
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
            fallback.step(ControlFrame::procedural(9), None, step as f32 / FPS);
        }
        for step in 60..180 {
            fallback.step(quiet(), None, step as f32 / FPS);
        }
        assert!(amplitude(&fallback) < 1e-4);
    }

    #[test]
    fn off_draws_nothing_and_time_never_rewinds() {
        let mut layers = MicLayers::new(Mode::Off);
        layers.step(quiet(), Some(&snapshot(0.5, 1)), 2.0);
        layers.step(quiet(), Some(&snapshot(0.5, 2)), 2.5);
        layers.step(quiet(), Some(&snapshot(0.5, 2)), 2.6);
        let mut instances = Vec::new();
        layers.build(quiet(), 1.0, &mut instances);
        assert!(instances.is_empty());
        // A burst still started, so turning bursts on shows it.
        layers.set_mode(Mode::Bursts);
        layers.build(quiet(), 1.0, &mut instances);
        assert_eq!(instances.len(), 1 + SPARKS);
        layers.step(quiet(), Some(&snapshot(0.5, 2)), 1.0);
        assert_eq!(layers.last_elapsed, Some(2.6));
    }
}
