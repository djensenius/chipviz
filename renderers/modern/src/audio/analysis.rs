//! Deterministic live-audio analysis: mono samples in, control-frame audio
//! fields out (see `shared/specs/source-adapter-v0.md`).
//!
//! The analyzer runs a Hann-windowed real FFT every `hop` samples and derives:
//! - eight log-spaced band levels (40 Hz to 12 kHz, as in the host bridge),
//!   plus `energy` (band mean), `bass`, `mid`, and `treble` (band-pair maxima);
//! - automatic gain: band levels in dBFS are mapped through a window whose top
//!   follows a slow peak tracker. The top never drops below
//!   `NOISE_FLOOR_DB + MIN_HEADROOM_DB` and anything under `NOISE_FLOOR_DB`
//!   maps to 0, so silence and room hiss decay to 0 instead of being amplified;
//! - attack/release smoothing of every level;
//! - onsets from spectral flux over the gain-normalised bands with an adaptive
//!   (moving mean) threshold and a refractory period;
//! - tempo: gaps between each onset and the few before it are octave-folded
//!   into 0.3-1.0 s (60-200 BPM) and accumulated in a decaying histogram,
//!   weighted towards the nearest onsets. The gap to the previous onset is
//!   pinned to 0.3 s or 1.0 s when it lies within one analysis hop (or tempo
//!   bin) outside that endpoint, so hop jitter cannot fold 60 or 200 BPM to
//!   120 or 100; longer gaps fold strictly so they reinforce the shorter
//!   period. The beat period is the weighted centroid of the strongest
//!   three-bin neighbourhood. Folding makes steady eighths read as the
//!   quarter-note tempo and keeps off-beats from doubling it;
//! - a forward-only beat clock ([`BeatTracker`]): `beat_phase` advances at the
//!   estimated tempo and `beat_count` increments exactly when the phase wraps,
//!   so `beat_count * 256 + beat_phase` never decreases. Onsets within a
//!   quarter beat of the expected beat correct the clock: an early onset jumps
//!   forward to the next beat, a late one slows the clock (to at most half
//!   speed) until it is back in line. Off-beat onsets are ignored unless
//!   several arrive in a row or the clock has lost lock (no on-beat onset for
//!   two beats), in which case the next onset re-anchors the beat.
//!
//! Known limits: tempos outside 60-200 BPM are folded into that range (tempos
//! within about one hop of either end are pinned to it), the
//! tracker cannot tell a 3/4 or swung feel from straight time, and a pattern
//! of only off-beats ends up treated as the beat.
//!
//! Everything is plain arithmetic on the input, so identical input produces
//! identical output.

use std::collections::VecDeque;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

pub const BANDS: usize = 8;
pub const MIN_SAMPLE_RATE: u32 = 8_000;

const MIN_BAND_HZ: f32 = 40.0;
const MAX_BAND_HZ: f32 = 12_000.0;
/// Levels below this (dBFS, per band RMS) are treated as silence.
pub const NOISE_FLOOR_DB: f32 = -65.0;
/// Caps the automatic gain: the window top never sits below -41 dBFS.
pub const MIN_HEADROOM_DB: f32 = 24.0;
/// Dynamic range shown between 0 and 255.
pub const RANGE_DB: f32 = 36.0;
/// How fast the peak tracker falls after loud passages.
pub const PEAK_RELEASE_DB_PER_S: f32 = 3.0;
pub const ATTACK_S: f32 = 0.015;
pub const RELEASE_S: f32 = 0.18;
const FLUX_HISTORY_S: f32 = 1.0;
const ONSET_RATIO: f32 = 1.5;
const ONSET_DELTA: f32 = 0.08;
const ONSET_REFRACTORY_S: f32 = 0.12;
const DEFAULT_BEAT_PERIOD_S: f32 = 0.5;
/// Folded beat periods span 0.3-1.0 s (200-60 BPM).
const MIN_BEAT_PERIOD_S: f32 = 0.3;
const MAX_BEAT_PERIOD_S: f32 = 1.0;
const MIN_ONSET_GAP_S: f64 = 0.075;
const MAX_ONSET_GAP_S: f64 = 2.0;
const TEMPO_BIN_S: f32 = 0.01;
const TEMPO_BINS: usize = 70;
const TEMPO_DECAY: f32 = 0.9;
const TEMPO_MIN_WEIGHT: f32 = 1.5;
const RECENT_ONSETS: usize = 4;
/// Onsets within this fraction of a beat of the expected beat steer the clock.
const LOCK_WINDOW: f32 = 0.25;
/// Consecutive off-beat onsets that re-anchor the beat.
const OFF_BEAT_RELOCK: u32 = 4;
/// Lock is lost after this many beats without an on-beat onset.
const LOCK_TIMEOUT_BEATS: f32 = 2.0;
/// The clock never runs slower than this fraction of the tempo while catching up.
const MAX_SLOWDOWN: f32 = 0.5;
const SILENT_DB: f32 = -200.0;

/// Forward-only beat clock with a folded inter-onset tempo histogram.
#[derive(Clone, Debug)]
pub struct BeatTracker {
    time_s: f64,
    recent_onsets: VecDeque<f64>,
    tempo_weight: [f32; TEMPO_BINS],
    tempo_sum: [f32; TEMPO_BINS],
    period_s: f32,
    tempo_known: bool,
    phase: f32,
    beat_count: u16,
    /// Phase still to be absorbed by running the clock slow.
    lag: f32,
    last_on_beat_s: Option<f64>,
    off_beat_streak: u32,
    /// Endpoint slack for [`fold_period`]; see [`BeatTracker::with_hop`].
    fold_tolerance_s: f32,
}

impl Default for BeatTracker {
    /// A tracker with no hop quantisation (one tempo bin of endpoint slack).
    fn default() -> Self {
        Self::with_hop(0.0)
    }
}

impl BeatTracker {
    /// A tracker fed onsets quantised to `hop_s` seconds. Onset gaps within
    /// `max(hop_s, TEMPO_BIN_S)` of the 0.3-1.0 s range endpoints are clamped
    /// onto them rather than octave-folded, so hop jitter around 60 or
    /// 200 BPM cannot fold a steady beat to half or double its tempo.
    /// A non-finite or negative `hop_s` is treated as `0.0`.
    pub fn with_hop(hop_s: f32) -> Self {
        let hop_s = if hop_s.is_finite() && hop_s > 0.0 {
            hop_s
        } else {
            0.0
        };
        Self {
            fold_tolerance_s: fold_tolerance(hop_s),
            time_s: 0.0,
            recent_onsets: VecDeque::with_capacity(RECENT_ONSETS),
            tempo_weight: [0.0; TEMPO_BINS],
            tempo_sum: [0.0; TEMPO_BINS],
            period_s: DEFAULT_BEAT_PERIOD_S,
            tempo_known: false,
            phase: 0.0,
            beat_count: 0,
            lag: 0.0,
            last_on_beat_s: None,
            off_beat_streak: 0,
        }
    }

    /// Advances the clock by `dt` seconds; `onset` marks an onset at the end.
    pub fn advance(&mut self, dt: f32, onset: bool) {
        self.time_s += f64::from(dt);
        let step = dt / self.period_s;
        let absorbed = self.lag.min(step * MAX_SLOWDOWN);
        self.lag -= absorbed;
        self.phase += step - absorbed;
        if self.phase >= 1.0 {
            self.phase = (self.phase - 1.0).min(0.999);
            self.beat_count = self.beat_count.wrapping_add(1);
        }
        if onset {
            self.update_tempo();
            self.align_to_onset();
        }
    }

    /// `None` until enough onset gaps agree on a tempo.
    pub fn bpm(&self) -> Option<f32> {
        self.tempo_known.then(|| 60.0 / self.period_s)
    }

    pub fn beat_phase(&self) -> u8 {
        (self.phase * 256.0).floor().clamp(0.0, 255.0) as u8
    }

    pub fn beat_count(&self) -> u16 {
        self.beat_count
    }

    fn update_tempo(&mut self) {
        for (weight, sum) in self.tempo_weight.iter_mut().zip(&mut self.tempo_sum) {
            *weight *= TEMPO_DECAY;
            *sum *= TEMPO_DECAY;
        }
        // Gaps to nearer onsets weigh more, so a multiple of the beat period
        // (two or three beats back) never outvotes the period itself. Only
        // the gap to the previous onset is pinned to a range endpoint; longer
        // gaps fold strictly so multiples reinforce the shorter period.
        for (lag, &previous) in self.recent_onsets.iter().rev().enumerate() {
            let gap = self.time_s - previous;
            if !(MIN_ONSET_GAP_S..=MAX_ONSET_GAP_S).contains(&gap) {
                continue;
            }
            let period = if lag == 0 {
                fold_period(gap as f32, self.fold_tolerance_s)
            } else {
                fold_multiple(gap as f32)
            };
            let bin = (((period - MIN_BEAT_PERIOD_S) / TEMPO_BIN_S) as usize).min(TEMPO_BINS - 1);
            let weight = 1.0 / (lag + 1) as f32;
            self.tempo_weight[bin] += weight;
            self.tempo_sum[bin] += period * weight;
        }
        if self.recent_onsets.len() == RECENT_ONSETS {
            self.recent_onsets.pop_front();
        }
        self.recent_onsets.push_back(self.time_s);

        // The peak is the bin whose neighbourhood (bin - 1..=bin + 1) holds the
        // most weight, so votes for one period that straddle two bins are not
        // outvoted by a single sharper bin. The period is the weighted
        // centroid of that neighbourhood. The first strongest neighbourhood
        // wins ties, so the result is deterministic.
        let neighbourhood = |bin: usize| bin.saturating_sub(1)..(bin + 2).min(TEMPO_BINS);
        let smoothed = |bin: usize| -> f32 { self.tempo_weight[neighbourhood(bin)].iter().sum() };
        let peak = (0..TEMPO_BINS).fold(0, |best, bin| {
            if smoothed(bin) > smoothed(best) {
                bin
            } else {
                best
            }
        });
        let weight = smoothed(peak);
        let sum: f32 = self.tempo_sum[neighbourhood(peak)].iter().sum();
        if weight >= TEMPO_MIN_WEIGHT {
            self.period_s = (sum / weight).clamp(MIN_BEAT_PERIOD_S, MAX_BEAT_PERIOD_S);
            self.tempo_known = true;
        }
    }

    fn align_to_onset(&mut self) {
        let locked = self.last_on_beat_s.is_some_and(|last| {
            self.time_s - last <= f64::from(LOCK_TIMEOUT_BEATS * self.period_s)
        });
        let early = self.phase >= 1.0 - LOCK_WINDOW;
        let late = self.phase <= LOCK_WINDOW;
        if locked && !early && !late && self.off_beat_streak + 1 < OFF_BEAT_RELOCK {
            self.off_beat_streak += 1;
            return;
        }
        if late {
            // The clock is ahead of the music: slow down, never rewind.
            self.lag = self.phase;
        } else {
            // Early, or (re-)anchoring: jump forward to the next beat.
            self.phase = 0.0;
            self.lag = 0.0;
            self.beat_count = self.beat_count.wrapping_add(1);
        }
        self.off_beat_streak = 0;
        self.last_on_beat_s = Some(self.time_s);
    }
}

/// Endpoint slack for onsets quantised to `hop_s`: at least one analysis hop
/// and one tempo bin, capped well below the range so folding still terminates.
fn fold_tolerance(hop_s: f32) -> f32 {
    hop_s.clamp(TEMPO_BIN_S, MIN_BEAT_PERIOD_S / 4.0)
}

/// Octave-folds a positive, finite onset gap into the inclusive 0.3-1.0 s
/// beat-period range. Gaps that fold to within `tolerance` outside an
/// endpoint are clamped onto it instead of being doubled or halved, so
/// 60 BPM (1.0 s) and 200 BPM (0.3 s) beats map to themselves despite hop
/// jitter. `tolerance` must be below `MIN_BEAT_PERIOD_S`.
fn fold_period(gap: f32, tolerance: f32) -> f32 {
    debug_assert!(gap.is_finite() && gap > 0.0, "onset gap {gap}");
    let mut period = gap;
    while period < MIN_BEAT_PERIOD_S - tolerance {
        period *= 2.0;
    }
    while period > MAX_BEAT_PERIOD_S + tolerance {
        period /= 2.0;
    }
    period.clamp(MIN_BEAT_PERIOD_S, MAX_BEAT_PERIOD_S)
}

/// Octave-folds a gap spanning several onsets into 0.3 s up to (excluding)
/// 1.0 s with no endpoint pinning, so a gap of two beats at 120 BPM votes for
/// 0.5 s rather than for 60 BPM.
fn fold_multiple(gap: f32) -> f32 {
    debug_assert!(gap.is_finite() && gap > 0.0, "onset gap {gap}");
    let mut period = gap;
    while period < MIN_BEAT_PERIOD_S {
        period *= 2.0;
    }
    while period >= MAX_BEAT_PERIOD_S {
        period /= 2.0;
    }
    period
}

/// Analysed audio fields in control-frame units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioFeatures {
    /// Estimated tempo, `0` until enough onset gaps agree.
    pub bpm: u8,
    pub beat_phase: u8,
    pub beat_count: u16,
    pub energy: u8,
    pub bass: u8,
    pub mid: u8,
    pub treble: u8,
    pub spectrum: [u8; BANDS],
}

pub struct Analyzer {
    sample_rate: f32,
    hop: usize,
    dt: f32,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// Converts summed |X|^2 over a band into that band's RMS^2.
    power_scale: f32,
    band_bins: [(usize, usize); BANDS],
    history: VecDeque<f32>,
    pending: usize,
    fft_input: Vec<f32>,
    fft_output: Vec<Complex<f32>>,
    fft_scratch: Vec<Complex<f32>>,
    peak_db: f32,
    previous_raw: [f32; BANDS],
    smoothed: [f32; BANDS],
    flux_history: VecDeque<f32>,
    flux_history_len: usize,
    time_s: f64,
    last_onset_s: Option<f64>,
    onsets: u64,
    beat: BeatTracker,
    features: AudioFeatures,
}

impl Analyzer {
    pub fn new(sample_rate: u32) -> Result<Self, String> {
        if sample_rate < MIN_SAMPLE_RATE {
            return Err(format!(
                "audio sample rate {sample_rate} Hz is below the {MIN_SAMPLE_RATE} Hz minimum"
            ));
        }
        let rate = sample_rate as f32;
        // ~25 Hz bins: enough to resolve the lowest (40-80 Hz) band.
        let fft_size = ((sample_rate / 25) as usize)
            .next_power_of_two()
            .clamp(512, 8192);
        let hop = fft_size / 4;
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(fft_size);
        let window: Vec<f32> = (0..fft_size)
            .map(|n| {
                let x = std::f32::consts::PI * n as f32 / fft_size as f32;
                x.sin() * x.sin()
            })
            .collect();
        let window_power: f32 = window.iter().map(|w| w * w).sum();
        let bin_hz = rate / fft_size as f32;
        let last_bin = fft_size / 2;
        let mut band_bins = [(0, 0); BANDS];
        for (bins, (low, high)) in band_bins.iter_mut().zip(band_edges(rate)) {
            let start = ((low / bin_hz).ceil() as usize).clamp(1, last_bin);
            let end = ((high / bin_hz).ceil() as usize).clamp(start + 1, last_bin + 1);
            *bins = (start, end);
        }
        let flux_history_len = ((FLUX_HISTORY_S * rate) / hop as f32).round().max(1.0) as usize;
        Ok(Self {
            sample_rate: rate,
            hop,
            dt: hop as f32 / rate,
            fft_input: fft.make_input_vec(),
            fft_output: fft.make_output_vec(),
            fft_scratch: fft.make_scratch_vec(),
            fft,
            window,
            power_scale: 2.0 / (fft_size as f32 * window_power),
            band_bins,
            history: VecDeque::from(vec![0.0; fft_size]),
            pending: 0,
            peak_db: NOISE_FLOOR_DB,
            previous_raw: [0.0; BANDS],
            smoothed: [0.0; BANDS],
            flux_history: VecDeque::with_capacity(flux_history_len),
            flux_history_len,
            time_s: 0.0,
            last_onset_s: None,
            onsets: 0,
            beat: BeatTracker::with_hop(hop as f32 / rate),
            features: AudioFeatures::default(),
        })
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Seconds of audio between analysis steps.
    #[cfg(test)]
    pub fn hop_seconds(&self) -> f32 {
        self.dt
    }

    pub fn features(&self) -> AudioFeatures {
        self.features
    }

    /// Total onsets detected since creation.
    #[cfg(test)]
    pub fn onsets(&self) -> u64 {
        self.onsets
    }

    /// Feeds mono samples, running one analysis step per completed hop.
    pub fn push(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.history.pop_front();
            self.history
                .push_back(if sample.is_finite() { sample } else { 0.0 });
            self.pending += 1;
            if self.pending == self.hop {
                self.pending = 0;
                self.step();
            }
        }
    }

    /// Accounts for `count` samples that were captured but dropped before
    /// analysis: the beat clock advances by their duration so it stays in
    /// step with real time; levels are left as they are.
    pub fn skip(&mut self, count: usize) {
        self.pending += count;
        while self.pending >= self.hop {
            self.pending -= self.hop;
            self.time_s += f64::from(self.dt);
            self.beat.advance(self.dt, false);
        }
        self.publish();
    }

    /// Feeds `count` zero samples (used when the device stops delivering).
    pub fn push_silence(&mut self, count: usize) {
        const CHUNK: [f32; 256] = [0.0; 256];
        let mut remaining = count;
        while remaining > 0 {
            let take = remaining.min(CHUNK.len());
            self.push(&CHUNK[..take]);
            remaining -= take;
        }
    }

    fn step(&mut self) {
        let dt = self.dt;
        self.time_s += f64::from(dt);

        for ((slot, sample), weight) in self
            .fft_input
            .iter_mut()
            .zip(&self.history)
            .zip(&self.window)
        {
            *slot = sample * weight;
        }
        // Buffer sizes come from the planner, so this cannot fail.
        if self
            .fft
            .process_with_scratch(
                &mut self.fft_input,
                &mut self.fft_output,
                &mut self.fft_scratch,
            )
            .is_err()
        {
            return;
        }

        let mut band_db = [SILENT_DB; BANDS];
        for (db, &(start, end)) in band_db.iter_mut().zip(&self.band_bins) {
            let power: f32 = self.fft_output[start..end]
                .iter()
                .map(Complex::norm_sqr)
                .sum();
            *db = to_db((power * self.power_scale).sqrt());
        }

        // Peak tracker: instant attack, slow linear (dB) release.
        let loudest = band_db.iter().copied().fold(SILENT_DB, f32::max);
        self.peak_db = loudest.max(self.peak_db - PEAK_RELEASE_DB_PER_S * dt);
        let top = self.peak_db.max(NOISE_FLOOR_DB + MIN_HEADROOM_DB);
        let bottom = (top - RANGE_DB).max(NOISE_FLOOR_DB);

        let mut raw = [0.0; BANDS];
        for (value, db) in raw.iter_mut().zip(band_db) {
            *value = ((db - bottom) / (top - bottom)).clamp(0.0, 1.0);
        }

        let flux = raw
            .iter()
            .zip(&self.previous_raw)
            .map(|(now, before)| (now - before).max(0.0))
            .sum::<f32>()
            / BANDS as f32;
        self.previous_raw = raw;
        let mean_flux = if self.flux_history.is_empty() {
            0.0
        } else {
            self.flux_history.iter().sum::<f32>() / self.flux_history.len() as f32
        };
        let threshold = mean_flux * ONSET_RATIO + ONSET_DELTA;
        if self.flux_history.len() == self.flux_history_len {
            self.flux_history.pop_front();
        }
        self.flux_history.push_back(flux);
        let refractory_over = self
            .last_onset_s
            .is_none_or(|last| self.time_s - last >= f64::from(ONSET_REFRACTORY_S));
        let onset = flux > threshold && refractory_over;

        let attack = 1.0 - (-dt / ATTACK_S).exp();
        let release = 1.0 - (-dt / RELEASE_S).exp();
        for (level, target) in self.smoothed.iter_mut().zip(raw) {
            let coefficient = if target > *level { attack } else { release };
            *level += (target - *level) * coefficient;
        }

        if onset {
            self.onsets += 1;
            self.last_onset_s = Some(self.time_s);
        }
        self.beat.advance(dt, onset);
        self.publish();
    }

    fn publish(&mut self) {
        let level = |value: f32| (value * 255.0).round().clamp(0.0, 255.0) as u8;
        let bands = self.smoothed;
        let mut spectrum = [0; BANDS];
        for (byte, value) in spectrum.iter_mut().zip(bands) {
            *byte = level(value);
        }
        self.features = AudioFeatures {
            bpm: self
                .beat
                .bpm()
                .map_or(0, |bpm| bpm.round().clamp(1.0, 255.0) as u8),
            beat_phase: self.beat.beat_phase(),
            beat_count: self.beat.beat_count(),
            energy: level(bands.iter().sum::<f32>() / BANDS as f32),
            bass: level(bands[0].max(bands[1])),
            mid: level(bands[3].max(bands[4])),
            treble: level(bands[6].max(bands[7])),
            spectrum,
        };
    }
}

/// Log-spaced band edges, matching `band_edges` in `host/bridge/live_bridge.py`.
pub fn band_edges(sample_rate: f32) -> [(f32, f32); BANDS] {
    let high = MAX_BAND_HZ.min(sample_rate / 2.0);
    let ratio = (high / MIN_BAND_HZ).powf(1.0 / BANDS as f32);
    let mut edges = [(0.0, 0.0); BANDS];
    let mut start = MIN_BAND_HZ;
    for edge in &mut edges {
        let end = start * ratio;
        *edge = (start, end);
        start = end;
    }
    edges
}

fn to_db(rms: f32) -> f32 {
    if rms > 1e-10 {
        20.0 * rms.log10()
    } else {
        SILENT_DB
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(frequency: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        let count = (seconds * RATE as f32) as usize;
        (0..count)
            .map(|n| {
                let t = n as f32 / RATE as f32;
                amplitude * (2.0 * std::f32::consts::PI * frequency * t).sin()
            })
            .collect()
    }

    /// Deterministic white noise in -amplitude..amplitude.
    fn noise(amplitude: f32, seconds: f32, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..(seconds * RATE as f32) as usize)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                amplitude * ((state >> 8) as f32 / (1 << 23) as f32 - 1.0)
            })
            .collect()
    }

    /// 10 ms decaying noise clicks at `bpm`.
    fn click_train(bpm: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        let period = (60.0 / bpm * RATE as f32) as usize;
        let click = noise(1.0, 0.01, 7);
        let mut samples = vec![0.0; (seconds * RATE as f32) as usize];
        for start in (0..samples.len()).step_by(period) {
            for (offset, value) in click.iter().enumerate() {
                if let Some(slot) = samples.get_mut(start + offset) {
                    let decay = 1.0 - offset as f32 / click.len() as f32;
                    *slot = amplitude * value * decay;
                }
            }
        }
        samples
    }

    fn analyze(samples: &[f32]) -> Analyzer {
        let mut analyzer = Analyzer::new(RATE).unwrap();
        analyzer.push(samples);
        analyzer
    }

    fn band_of(frequency: f32) -> usize {
        band_edges(RATE as f32)
            .iter()
            .position(|&(low, high)| frequency >= low && frequency < high)
            .unwrap()
    }

    #[test]
    fn band_edges_are_log_spaced_like_the_bridge() {
        let edges = band_edges(48_000.0);
        assert!((edges[0].0 - 40.0).abs() < 1e-3);
        assert!((edges[7].1 - 12_000.0).abs() < 0.5);
        let ratio = edges[0].1 / edges[0].0;
        for (low, high) in edges {
            assert!((high / low - ratio).abs() < 1e-3);
        }
        assert!((band_edges(8_000.0)[7].1 - 4_000.0).abs() < 0.5);
    }

    #[test]
    fn rejects_low_sample_rates_and_sizes_fft_for_low_band() {
        assert!(Analyzer::new(4_000).is_err());
        let analyzer = Analyzer::new(RATE).unwrap();
        assert_eq!(analyzer.history.len(), 2048);
        assert!(analyzer.sample_rate() / analyzer.history.len() as f32 <= 25.0);
        for (start, end) in analyzer.band_bins {
            assert!(start < end, "every band has at least one bin");
        }
        assert!(Analyzer::new(8_000).is_ok());
    }

    #[test]
    fn sine_lights_its_own_band_and_mapped_fields() {
        for frequency in [60.0, 250.0, 1_000.0, 3_000.0, 9_000.0] {
            let band = band_of(frequency);
            let features = analyze(&sine(frequency, 0.5, 1.0)).features();
            assert!(
                features.spectrum[band] >= 240,
                "{frequency} Hz band {band}: {features:?}"
            );
            for (index, &value) in features.spectrum.iter().enumerate() {
                if index.abs_diff(band) > 1 {
                    assert!(
                        value < 100,
                        "{frequency} Hz leaked into {index}: {features:?}"
                    );
                }
            }
            let pair_max = |a: usize, b: usize| features.spectrum[a].max(features.spectrum[b]);
            assert_eq!(features.bass, pair_max(0, 1));
            assert_eq!(features.mid, pair_max(3, 4));
            assert_eq!(features.treble, pair_max(6, 7));
            let mean = features.spectrum.iter().map(|&v| u32::from(v)).sum::<u32>() / 8;
            assert!(u32::from(features.energy).abs_diff(mean) <= 1);
        }
        assert!(analyze(&sine(60.0, 0.5, 1.0)).features().bass >= 240);
        assert!(analyze(&sine(9_000.0, 0.5, 1.0)).features().treble >= 240);
    }

    #[test]
    fn noise_fills_every_band() {
        let features = analyze(&noise(0.3, 2.0, 1)).features();
        assert!(
            features.spectrum.iter().all(|&value| value > 60),
            "{features:?}"
        );
        assert!(features.energy > 100, "{features:?}");
    }

    #[test]
    fn automatic_gain_makes_quiet_and_loud_input_usable() {
        // -46 dBFS (quiet room) and -6 dBFS (loud room) both reach near full scale.
        let quiet = analyze(&sine(1_000.0, 0.007, 2.0)).features();
        let loud = analyze(&sine(1_000.0, 0.7, 2.0)).features();
        let band = band_of(1_000.0);
        assert!(quiet.spectrum[band] >= 200, "{quiet:?}");
        assert!(loud.spectrum[band] >= 240, "{loud:?}");
    }

    #[test]
    fn gain_is_capped_so_hiss_is_not_amplified() {
        // -80 dBFS hiss sits under the noise floor and stays dark.
        let features = analyze(&noise(1.7e-4, 2.0, 3)).features();
        assert_eq!(features.spectrum, [0; BANDS], "{features:?}");
        assert_eq!(features.energy, 0);
        // A tone 60 dB below the loudest recent level is dimmed, not boosted.
        let mut analyzer = analyze(&sine(1_000.0, 0.7, 1.0));
        analyzer.push(&sine(1_000.0, 0.0007, 0.5));
        assert!(analyzer.features().spectrum[band_of(1_000.0)] < 40);
    }

    #[test]
    fn silence_decays_smoothly_to_zero() {
        let mut analyzer = analyze(&noise(0.5, 1.0, 5));
        let mut previous = analyzer.features();
        assert!(previous.energy > 100);
        let hop = (analyzer.hop_seconds() * RATE as f32).round() as usize;
        // Until one FFT window of silence has passed, the window still holds
        // noise, so levels may wobble; after that they must only fall.
        let flush_steps = 4;
        let mut steps = 0;
        while previous.energy > 0 {
            analyzer.push_silence(hop);
            let next = analyzer.features();
            if steps >= flush_steps {
                assert!(next.energy <= previous.energy, "{previous:?} -> {next:?}");
            }
            // No single step drops more than a fifth of full scale.
            assert!(
                previous.energy.saturating_sub(next.energy) < 51,
                "{previous:?} -> {next:?}"
            );
            previous = next;
            steps += 1;
            assert!(steps < 400, "energy never reached 0");
        }
        let seconds = steps as f32 * analyzer.hop_seconds();
        assert!((0.3..2.0).contains(&seconds), "decayed in {seconds} s");
        analyzer.push_silence(RATE as usize);
        let features = analyzer.features();
        assert_eq!(features.spectrum, [0; BANDS]);
        assert_eq!((features.bass, features.mid, features.treble), (0, 0, 0));
    }

    #[test]
    fn silence_from_start_stays_zero_and_produces_no_onsets() {
        let mut analyzer = Analyzer::new(RATE).unwrap();
        analyzer.push_silence(4 * RATE as usize);
        let features = analyzer.features();
        assert_eq!(features.energy, 0);
        assert_eq!(features.spectrum, [0; BANDS]);
        assert_eq!(features.bpm, 0);
        assert_eq!(analyzer.onsets(), 0);
    }

    #[test]
    fn steady_tone_and_noise_do_not_retrigger_onsets() {
        for samples in [sine(440.0, 0.5, 4.0), noise(0.3, 4.0, 9)] {
            let mut analyzer = analyze(&samples[..RATE as usize]);
            let initial = analyzer.onsets();
            analyzer.push(&samples[RATE as usize..]);
            assert!(
                analyzer.onsets() - initial <= 1,
                "{} onsets",
                analyzer.onsets()
            );
        }
    }

    #[test]
    fn click_train_at_120_bpm_gives_two_beats_per_second() {
        let seconds = 8.0;
        let analyzer = analyze(&click_train(120.0, 0.5, seconds));
        let onsets = analyzer.onsets();
        assert!(
            (15..=17).contains(&onsets),
            "{onsets} onsets in {seconds} s"
        );
        let features = analyzer.features();
        assert!((118..=122).contains(&features.bpm), "{features:?}");
        assert!(
            (15..=17).contains(&features.beat_count),
            "{} beats in {seconds} s",
            features.beat_count
        );
    }

    #[test]
    fn quiet_click_train_is_detected_too() {
        let analyzer = analyze(&click_train(120.0, 0.03, 8.0));
        assert!(
            (15..=17).contains(&analyzer.onsets()),
            "{}",
            analyzer.onsets()
        );
    }

    /// 10 ms clicks at the given times, with per-click amplitude.
    fn clicks(events: &[(f32, f32)], seconds: f32) -> Vec<f32> {
        let click = noise(1.0, 0.01, 7);
        let mut samples = vec![0.0; (seconds * RATE as f32) as usize];
        for &(time, amplitude) in events {
            let start = (time * RATE as f32) as usize;
            for (offset, value) in click.iter().enumerate() {
                if let Some(slot) = samples.get_mut(start + offset) {
                    let decay = 1.0 - offset as f32 / click.len() as f32;
                    *slot = amplitude * value * decay;
                }
            }
        }
        samples
    }

    /// Quarter notes at `bpm` from `start` for `beats` beats, with optional
    /// softer off-beat eighths.
    fn pattern(bpm: f32, start: f32, beats: usize, off_beats: Option<f32>) -> Vec<(f32, f32)> {
        let period = 60.0 / bpm;
        let mut events = Vec::new();
        for beat in 0..beats {
            let time = start + beat as f32 * period;
            events.push((time, 0.5));
            if let Some(amplitude) = off_beats {
                events.push((time + period / 2.0, amplitude));
            }
        }
        events
    }

    fn position(features: AudioFeatures) -> u32 {
        u32::from(features.beat_count) * 256 + u32::from(features.beat_phase)
    }

    /// Replays `samples` hop by hop, asserting the beat position never
    /// decreases, and returns the final features.
    fn replay_monotonic(samples: &[f32]) -> AudioFeatures {
        let mut analyzer = Analyzer::new(RATE).unwrap();
        let hop = (analyzer.hop_seconds() * RATE as f32).round() as usize;
        let mut previous = analyzer.features();
        for chunk in samples.chunks(hop) {
            analyzer.push(chunk);
            let next = analyzer.features();
            assert!(
                position(next) >= position(previous),
                "beat went backwards: {previous:?} -> {next:?}"
            );
            assert!(
                next.beat_count.wrapping_sub(previous.beat_count) <= 1,
                "{previous:?} -> {next:?}"
            );
            previous = next;
        }
        previous
    }

    #[test]
    fn beat_position_never_decreases_for_steady_quarters() {
        let features = replay_monotonic(&click_train(120.0, 0.5, 8.0));
        assert!((118..=122).contains(&features.bpm), "{features:?}");
        assert!((15..=17).contains(&features.beat_count), "{features:?}");
    }

    #[test]
    fn off_beat_eighths_neither_rewind_nor_double_count() {
        let events = pattern(120.0, 0.0, 16, Some(0.3));
        let features = replay_monotonic(&clicks(&events, 8.0));
        assert!((118..=122).contains(&features.bpm), "{features:?}");
        assert!((15..=17).contains(&features.beat_count), "{features:?}");
    }

    #[test]
    fn steady_eighths_report_the_quarter_note_tempo() {
        let features = replay_monotonic(&click_train(240.0, 0.5, 8.0));
        assert!((118..=122).contains(&features.bpm), "{features:?}");
        assert!((14..=18).contains(&features.beat_count), "{features:?}");
    }

    #[test]
    fn tempo_change_keeps_the_beat_moving_forward() {
        let mut events = pattern(100.0, 0.0, 10, None);
        events.extend(pattern(140.0, 6.0, 14, None));
        let features = replay_monotonic(&clicks(&events, 12.0));
        assert!((138..=142).contains(&features.bpm), "{features:?}");

        // Same change after a gap of silence.
        let mut samples = click_train(100.0, 0.5, 6.0);
        samples.extend(vec![0.0; 2 * RATE as usize]);
        samples.extend(click_train(140.0, 0.5, 6.0));
        let features = replay_monotonic(&samples);
        assert!((138..=142).contains(&features.bpm), "{features:?}");
    }

    #[test]
    fn beat_tracker_is_monotonic_for_arbitrary_onsets() {
        let mut tracker = BeatTracker::default();
        let mut state = 12_345u32;
        let mut previous = 0u32;
        for _ in 0..20_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let onset = state >> 28 == 0;
            tracker.advance(0.0107, onset);
            let now = u32::from(tracker.beat_count()) * 256 + u32::from(tracker.beat_phase());
            assert!(now >= previous, "{previous} -> {now}");
            previous = now;
        }
    }

    #[test]
    fn fold_period_maps_gaps_into_the_beat_range() {
        let tolerance = fold_tolerance(0.0);
        assert!((fold_period(0.25, tolerance) - 0.5).abs() < 1e-6);
        assert!((fold_period(0.125, tolerance) - 0.5).abs() < 1e-6);
        assert!((fold_period(1.2, tolerance) - 0.6).abs() < 1e-6);
        assert!((fold_period(0.75, tolerance) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn fold_tolerance_is_at_least_one_hop_and_one_bin() {
        assert_eq!(fold_tolerance(0.0), TEMPO_BIN_S);
        assert_eq!(fold_tolerance(0.0107), 0.0107);
        assert!(fold_tolerance(1.0) < MIN_BEAT_PERIOD_S);
    }

    #[test]
    fn fold_period_keeps_both_range_endpoints() {
        let tolerance = fold_tolerance(512.0 / RATE as f32);
        // 60 BPM: one-second gaps stay put; two-second gaps fold onto them.
        assert_eq!(fold_period(MAX_BEAT_PERIOD_S, tolerance), MAX_BEAT_PERIOD_S);
        assert_eq!(
            fold_period(2.0 * MAX_BEAT_PERIOD_S, tolerance),
            MAX_BEAT_PERIOD_S
        );
        // 200 BPM is neither doubled nor halved out of range.
        assert_eq!(fold_period(MIN_BEAT_PERIOD_S, tolerance), MIN_BEAT_PERIOD_S);
        assert_eq!(
            fold_period(MIN_BEAT_PERIOD_S / 2.0, tolerance),
            MIN_BEAT_PERIOD_S
        );
        // Hop jitter just outside an endpoint clamps onto it.
        let jitter = 0.9 * tolerance;
        assert_eq!(
            fold_period(MAX_BEAT_PERIOD_S + jitter, tolerance),
            MAX_BEAT_PERIOD_S
        );
        assert_eq!(
            fold_period(2.0 * (MAX_BEAT_PERIOD_S + jitter), tolerance),
            MAX_BEAT_PERIOD_S
        );
        assert_eq!(
            fold_period(MIN_BEAT_PERIOD_S - jitter, tolerance),
            MIN_BEAT_PERIOD_S
        );
        // Beyond the tolerance, gaps still fold by octaves.
        let beyond = MAX_BEAT_PERIOD_S + 2.0 * tolerance;
        assert!((fold_period(beyond, tolerance) - beyond / 2.0).abs() < 1e-6);
        let below = MIN_BEAT_PERIOD_S - 2.0 * tolerance;
        assert!((fold_period(below, tolerance) - below * 2.0).abs() < 1e-6);
    }

    #[test]
    fn multiples_fold_strictly_onto_the_shorter_period() {
        assert_eq!(fold_multiple(1.0), 0.5);
        assert_eq!(fold_multiple(2.0), 0.5);
        assert!((fold_multiple(1.5) - 0.75).abs() < 1e-6);
        assert_eq!(fold_multiple(0.3), 0.3);
    }

    #[test]
    fn with_hop_sanitises_bad_hops() {
        for hop in [f32::NAN, f32::INFINITY, -1.0] {
            assert_eq!(BeatTracker::with_hop(hop).fold_tolerance_s, TEMPO_BIN_S);
        }
    }

    /// Quarter-note clicks at `bpm` with deterministic uniform timing jitter
    /// of up to `jitter` seconds either way.
    fn jittered_clicks(bpm: f32, jitter: f32, seconds: f32, seed: u32) -> Vec<f32> {
        let period = 60.0 / bpm;
        let mut state = seed;
        let mut events = Vec::new();
        let mut beat = 0.05;
        while beat < seconds {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let unit = (state >> 8) as f32 / (1 << 23) as f32 - 1.0;
            events.push((beat + unit * jitter, 0.5));
            beat += period;
        }
        clicks(&events, seconds)
    }

    #[test]
    fn jittered_click_trains_read_their_tempo() {
        for (bpm, jitter, seconds, range) in [
            (120.0, 0.005, 10.0, 118..=122),
            (60.0, 0.005, 12.0, 58..=62),
            (121.0, 0.0, 10.0, 119..=123),
            (200.0, 0.003, 8.0, 192..=200),
        ] {
            for seed in 1..=4 {
                let features = replay_monotonic(&jittered_clicks(bpm, jitter, seconds, seed));
                assert!(
                    range.contains(&features.bpm),
                    "{bpm} BPM ±{jitter} s seed {seed}: {features:?}"
                );
            }
        }
    }

    #[test]
    fn click_train_at_60_bpm_reads_as_60_not_120() {
        let features = replay_monotonic(&click_train(60.0, 0.5, 12.0));
        assert!((58..=62).contains(&features.bpm), "{features:?}");
    }

    #[test]
    fn click_train_at_200_bpm_reads_as_200_not_100() {
        let features = replay_monotonic(&click_train(200.0, 0.5, 8.0));
        assert!((194..=200).contains(&features.bpm), "{features:?}");
    }

    #[test]
    fn skipped_samples_advance_the_beat_clock_only() {
        let mut skipped = analyze(&click_train(120.0, 0.5, 4.0));
        let mut silent = analyze(&click_train(120.0, 0.5, 4.0));
        let levels = skipped.features().energy;
        skipped.skip(RATE as usize);
        silent.push_silence(RATE as usize);
        let (a, b) = (skipped.features(), silent.features());
        assert_eq!((a.beat_count, a.beat_phase), (b.beat_count, b.beat_phase));
        assert_eq!(a.energy, levels);
    }

    #[test]
    fn analysis_is_deterministic_across_chunkings() {
        let samples = click_train(128.0, 0.3, 3.0);
        let whole = analyze(&samples).features();
        let mut chunked = Analyzer::new(RATE).unwrap();
        for chunk in samples.chunks(333) {
            chunked.push(chunk);
        }
        assert_eq!(whole, chunked.features());
        assert_eq!(whole, analyze(&samples).features());
    }

    #[test]
    fn non_finite_samples_are_ignored() {
        let mut samples = sine(1_000.0, 0.5, 0.5);
        samples[100] = f32::NAN;
        samples[200] = f32::INFINITY;
        let features = analyze(&samples).features();
        assert!(features.spectrum[band_of(1_000.0)] >= 200, "{features:?}");
    }
}
