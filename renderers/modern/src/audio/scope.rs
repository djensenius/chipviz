//! Renderer-internal waveform and onset side channel for the mic layers.
//!
//! The control frame carries no waveform, so the analysis thread also feeds
//! its mono samples to a [`ScopeTap`]. After each poll it publishes a
//! [`ScopeSnapshot`] (a trigger-aligned, decimated, gain-normalised
//! waveform envelope plus the analyzer's onset count) to [`SharedScope`]. Both sides
//! are wait-free: the writer stores one atomic per point and the reader loads
//! them, so the render thread never waits on audio and memory stays bounded.
//! A read that races a publish can mix points from two consecutive snapshots;
//! they are a few milliseconds apart, which is harmless for drawing.

use std::sync::atomic::{AtomicU32, Ordering};

/// Points in a published waveform.
pub const POINTS: usize = 256;
/// Samples shown per snapshot (about 43 ms at 48 kHz).
pub const SPAN: usize = 2048;
/// Samples searched before the shown span for a rising zero crossing, so a
/// steady tone stands still instead of scrolling: at least one period of
/// `LOWEST_HZ` at the input rate, within `MIN_SEARCH..=MAX_SEARCH`.
const LOWEST_HZ: f32 = 30.0;
const MIN_SEARCH: usize = 1024;
/// One `LOWEST_HZ` period at 384 kHz, which bounds memory for any rate.
const MAX_SEARCH: usize = 12_800;
/// The gain normaliser's peak never drops below this (about -34 dBFS), so
/// room hiss stays small instead of being amplified to full height.
pub const PEAK_FLOOR: f32 = 0.02;
const PEAK_RELEASE_S: f32 = 1.5;

/// One published waveform as two outlines, the maximum (`upper`) and
/// minimum (`lower`) of each point's samples (see [`decimate`]), in -1..=1
/// (normalised by the recent peak); `onsets` counts the analyzer's onsets
/// (wrapping).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeSnapshot {
    pub upper: [f32; POINTS],
    pub lower: [f32; POINTS],
    pub onsets: u32,
}

impl Default for ScopeSnapshot {
    fn default() -> Self {
        Self {
            upper: [0.0; POINTS],
            lower: [0.0; POINTS],
            onsets: 0,
        }
    }
}

/// Writer-side state, owned by the analysis thread. Allocation-free after
/// construction.
pub struct ScopeTap {
    ring: Vec<f32>,
    head: usize,
    peak: f32,
    release: f32,
    scratch: Vec<f32>,
}

impl ScopeTap {
    pub fn new(sample_rate: f32) -> Self {
        let history = SPAN + search(sample_rate);
        Self {
            ring: vec![0.0; history],
            head: 0,
            peak: PEAK_FLOOR,
            release: (-1.0 / (sample_rate.max(1.0) * PEAK_RELEASE_S)).exp(),
            scratch: vec![0.0; history],
        }
    }

    pub fn push(&mut self, samples: &[f32]) {
        for &sample in samples {
            let sample = if sample.is_finite() { sample } else { 0.0 };
            self.peak = sample.abs().max(self.peak * self.release);
            self.ring[self.head] = sample;
            self.head = (self.head + 1) % self.ring.len();
        }
    }

    pub fn push_silence(&mut self, count: usize) {
        const CHUNK: [f32; 256] = [0.0; 256];
        let history = self.ring.len();
        let mut remaining = count.min(history);
        while remaining > 0 {
            let take = remaining.min(CHUNK.len());
            self.push(&CHUNK[..take]);
            remaining -= take;
        }
        // Long silences still decay the peak by their full length.
        if count > history {
            self.peak *= self.release.powf((count - history) as f32);
        }
    }

    /// The latest span, trigger-aligned, decimated and normalised.
    pub fn snapshot(&mut self, onsets: u32) -> ScopeSnapshot {
        // `head` is the oldest sample: oldest first is `ring[head..]` then
        // `ring[..head]`.
        let (newest, oldest) = self.ring.split_at(self.head);
        self.scratch[..oldest.len()].copy_from_slice(oldest);
        self.scratch[oldest.len()..].copy_from_slice(newest);
        let start = trigger(&self.scratch);
        let (mut upper, mut lower) = ([0.0; POINTS], [0.0; POINTS]);
        decimate(&self.scratch[start..start + SPAN], &mut upper, &mut lower);
        let gain = 1.0 / self.peak.max(PEAK_FLOOR);
        for point in upper.iter_mut().chain(&mut lower) {
            *point = (*point * gain).clamp(-1.0, 1.0);
        }
        ScopeSnapshot {
            upper,
            lower,
            onsets,
        }
    }
}

/// Trigger search length for `sample_rate`; see `MIN_SEARCH`.
fn search(sample_rate: f32) -> usize {
    let period = (sample_rate / LOWEST_HZ).ceil();
    if period.is_nan() {
        return MIN_SEARCH;
    }
    // Saturating float-to-int conversion, then bounded.
    (period as usize).clamp(MIN_SEARCH, MAX_SEARCH)
}

/// Start of the shown span in `history` (oldest first, `SPAN` plus the
/// search length):
/// the latest rising zero crossing that leaves a full span after it, or the
/// newest span when there is none.
pub fn trigger(history: &[f32]) -> usize {
    let last = history.len().saturating_sub(SPAN);
    (1..=last)
        .rev()
        .find(|&index| history[index - 1] < 0.0 && history[index] >= 0.0)
        .unwrap_or(last)
}

/// Peak-detect decimation into two outlines: each point's `upper` and
/// `lower` are the maximum and minimum of its share of `samples`. The same
/// rule applies to every point, so nothing switches style between
/// neighbours. Where the signal is slow, a share is a short monotone piece
/// and the outlines are the same curve offset by at most one point, so they
/// draw as one thin line; a one-sided click keeps its full height in one
/// outline. A tone too fast for the points (a cycle or more per share)
/// keeps its loudness as two separate thin lines at its envelope instead of
/// cancelling to a flat line or filling a band. Empty shares (more points
/// than samples) repeat the nearest sample.
pub fn decimate(samples: &[f32], upper: &mut [f32], lower: &mut [f32]) {
    let points = upper.len().min(lower.len());
    for index in 0..points {
        let start = index * samples.len() / points;
        let end = ((index + 1) * samples.len() / points).max(start + 1);
        let share = samples
            .get(start..end.min(samples.len()))
            .unwrap_or_default();
        let (low, high) = share
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), &v| {
                (low.min(v), high.max(v))
            });
        if share.is_empty() {
            (upper[index], lower[index]) = (0.0, 0.0);
        } else {
            (upper[index], lower[index]) = (high, low);
        }
    }
}

/// Wait-free single-writer publication of the latest [`ScopeSnapshot`].
pub struct SharedScope {
    upper: [AtomicU32; POINTS],
    lower: [AtomicU32; POINTS],
    onsets: AtomicU32,
}

impl Default for SharedScope {
    fn default() -> Self {
        Self {
            upper: std::array::from_fn(|_| AtomicU32::new(0)),
            lower: std::array::from_fn(|_| AtomicU32::new(0)),
            onsets: AtomicU32::new(0),
        }
    }
}

impl SharedScope {
    pub fn store(&self, snapshot: &ScopeSnapshot) {
        let slots = self.upper.iter().chain(&self.lower);
        for (slot, point) in slots.zip(snapshot.upper.iter().chain(&snapshot.lower)) {
            slot.store(point.to_bits(), Ordering::Relaxed);
        }
        self.onsets.store(snapshot.onsets, Ordering::Release);
    }

    pub fn load(&self) -> ScopeSnapshot {
        let onsets = self.onsets.load(Ordering::Acquire);
        ScopeSnapshot {
            upper: std::array::from_fn(|index| {
                f32::from_bits(self.upper[index].load(Ordering::Relaxed))
            }),
            lower: std::array::from_fn(|index| {
                f32::from_bits(self.lower[index].load(Ordering::Relaxed))
            }),
            onsets,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn sine(count: usize, hz: f32, amplitude: f32, phase: f32) -> Vec<f32> {
        (0..count)
            .map(|n| amplitude * (std::f32::consts::TAU * hz * n as f32 / RATE + phase).sin())
            .collect()
    }

    /// Both outlines, upper then lower, for drift comparisons.
    fn outlines(snapshot: &ScopeSnapshot) -> Vec<f32> {
        snapshot
            .upper
            .iter()
            .chain(&snapshot.lower)
            .copied()
            .collect()
    }

    #[test]
    fn decimation_keeps_shape_and_short_clicks() {
        let (mut upper, mut lower) = ([0.0; 8], [0.0; 8]);
        let mut samples = vec![0.1; 64];
        samples[37] = -0.9;
        decimate(&samples, &mut upper, &mut lower);
        assert_eq!(upper, [0.1; 8]);
        for (index, point) in lower.into_iter().enumerate() {
            // The click keeps its full height in the lower outline.
            let expected = if index == 4 { -0.9 } else { 0.1 };
            assert_eq!(point, expected, "{index}");
        }
        // A steep ramp spans its share: its ends, one point apart.
        let ramp: Vec<f32> = (0..8).map(|n| n as f32 - 3.5).collect();
        decimate(&ramp, &mut upper[..1], &mut lower[..1]);
        assert_eq!((upper[0], lower[0]), (3.5, -3.5));

        // Uneven and short inputs still fill every point.
        let (mut upper, mut lower) = ([9.0; 5], [9.0; 5]);
        decimate(&[1.0, -2.0, 3.0], &mut upper, &mut lower);
        assert_eq!(upper, [1.0, 1.0, -2.0, -2.0, 3.0]);
        assert_eq!(lower, upper);
        let (mut upper, mut lower) = ([9.0; 4], [9.0; 4]);
        decimate(&[], &mut upper, &mut lower);
        assert_eq!((upper, lower), ([0.0; 4], [0.0; 4]));
    }

    /// Decimated outlines of a full-scale `hz` tone at `rate` for each start
    /// phase within one sample of a rising zero crossing, as the trigger
    /// leaves it.
    fn tone_outlines(rate: f32, hz: f32) -> Vec<([f32; POINTS], [f32; POINTS])> {
        let step = std::f32::consts::TAU * hz / rate;
        (0..8)
            .map(|offset| {
                let phase = step * offset as f32 / 8.0;
                let tone: Vec<f32> = (0..SPAN).map(|n| (step * n as f32 + phase).sin()).collect();
                let (mut upper, mut lower) = ([0.0; POINTS], [0.0; POINTS]);
                decimate(&tone, &mut upper, &mut lower);
                (upper, lower)
            })
            .collect()
    }

    #[test]
    fn decimation_keeps_tones_tied_to_the_point_spacing_visible() {
        // Eight samples per point at 48 kHz (and at 96 kHz, where 12 kHz is
        // one cycle a share): 6 kHz puts exactly one cycle in each share,
        // 12 kHz two, and 6.01 kHz drifts slowly across them. A cycle-mean
        // decimation draws these flat; the outlines hold their envelope, so
        // every upper point is at least about 0.5 and every lower point at
        // most about -0.5 (4 kHz samples its trough at exactly -0.5).
        for (rate, hz) in [
            (48_000.0, 4_000.0),
            (48_000.0, 6_000.0),
            (48_000.0, 6_010.0),
            (48_000.0, 12_000.0),
            (48_000.0, 18_000.0),
            (96_000.0, 12_000.0),
        ] {
            for (upper, lower) in tone_outlines(rate, hz) {
                let top = upper.iter().copied().fold(f32::INFINITY, f32::min);
                let bottom = lower.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                assert!(
                    top >= 0.49 && bottom <= -0.49,
                    "{hz} Hz at {rate}: {top} {bottom}"
                );
            }
        }
        // A few points per cycle: the outlines follow the wave and reach
        // its peaks.
        for hz in [2_000.0, 3_000.0] {
            for (upper, lower) in tone_outlines(48_000.0, hz) {
                let top = upper.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let bottom = lower.iter().copied().fold(f32::INFINITY, f32::min);
                assert!(top >= 0.9 && bottom <= -0.9, "{hz} Hz: {top} {bottom}");
            }
        }
        // A slow tone's outlines are one curve: apart by at most one point's
        // travel (2 pi 440 * 8 / 48000 = 0.46), and smooth.
        for (upper, lower) in tone_outlines(48_000.0, 440.0) {
            let apart = upper
                .iter()
                .zip(lower)
                .map(|(high, low)| high - low)
                .fold(0.0, f32::max);
            assert!(apart < 0.5, "a 440 Hz tone splits by {apart}");
            let jump = upper
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .fold(0.0, f32::max);
            assert!(jump < 0.5, "a 440 Hz tone zigzags by {jump}");
        }
    }

    #[test]
    fn trigger_holds_a_steady_tone_still() {
        let mut tap = ScopeTap::new(RATE);
        let tone = sine(RATE as usize, 440.0, 0.5, 0.3);
        let mut shots = Vec::new();
        for chunk in tone.chunks(800).take(20) {
            tap.push(chunk);
            shots.push(tap.snapshot(0));
        }
        let first = outlines(&shots[5]);
        for shot in &shots[6..] {
            let drift = first
                .iter()
                .zip(outlines(shot))
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            // At most one sample of trigger jitter: 2 pi 440 / 48000 per sample.
            assert!(drift < 0.08, "aligned tone drifted by {drift}");
        }
        // Starts on a rising crossing and is normalised to full height.
        assert!(first[0] >= 0.0 && first[1] > first[0], "{:?}", &first[..4]);
        let peak = first.iter().copied().fold(0.0, |a: f32, b| a.max(b.abs()));
        assert!(peak > 0.95 && peak <= 1.0, "{peak}");
    }

    fn tone(rate: f32, count: usize, hz: f32) -> Vec<f32> {
        (0..count)
            .map(|n| 0.5 * (std::f32::consts::TAU * hz * n as f32 / rate + 0.3).sin())
            .collect()
    }

    #[test]
    fn low_tones_stay_aligned_at_any_rate() {
        // One period of each is 1,200 samples, longer than a fixed 1,024
        // sample search would cover.
        for (rate, hz) in [(48_000.0, 40.0), (96_000.0, 80.0)] {
            let mut tap = ScopeTap::new(rate);
            let signal = tone(rate, 4 * rate as usize, hz);
            let mut shots = Vec::new();
            // Chunks that do not divide the period, so each snapshot ends at a
            // different phase.
            for chunk in signal.chunks(733).take(80) {
                tap.push(chunk);
                shots.push(outlines(&tap.snapshot(0)));
            }
            let first = &shots[10];
            for (index, shot) in shots[11..].iter().enumerate() {
                let drift = first
                    .iter()
                    .zip(shot)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0, f32::max);
                assert!(
                    drift < 0.03,
                    "{rate} Hz rate, {hz} Hz tone, shot {index}: {drift}"
                );
            }
            assert!(first[0] >= 0.0 && first[1] > first[0], "{:?}", &first[..4]);
        }
    }

    #[test]
    fn history_covers_the_lowest_tone_period() {
        for rate in [8_000.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            let tap = ScopeTap::new(rate);
            let search = tap.ring.len() - SPAN;
            assert!(search as f32 >= rate / LOWEST_HZ, "{rate}: {search}");
            assert!(search >= MIN_SEARCH);
        }
        // Absurd rates stay bounded.
        assert!(ScopeTap::new(f32::INFINITY).ring.len() <= SPAN + MAX_SEARCH);
        assert_eq!(ScopeTap::new(f32::NAN).ring.len(), SPAN + MIN_SEARCH);
    }

    #[test]
    fn trigger_falls_back_to_the_newest_span() {
        let flat = vec![0.25; SPAN + MIN_SEARCH];
        assert_eq!(trigger(&flat), MIN_SEARCH);
        let mut edge = vec![-1.0; SPAN + MIN_SEARCH];
        edge[100..].fill(1.0);
        assert_eq!(trigger(&edge), 100);
    }

    #[test]
    fn silence_flattens_and_hiss_is_not_amplified() {
        let mut tap = ScopeTap::new(RATE);
        let history = tap.ring.len();
        tap.push(&sine(4800, 220.0, 0.8, 0.0));
        tap.push_silence(history);
        assert_eq!(outlines(&tap.snapshot(3)), [0.0; 2 * POINTS]);
        // Long silence lets the peak fall to the floor; quiet hiss stays small.
        tap.push_silence(10 * RATE as usize);
        tap.push(&sine(history, 3000.0, 0.002, 0.0));
        let hiss = outlines(&tap.snapshot(0));
        assert!(hiss.iter().all(|point| point.abs() <= 0.11), "{hiss:?}");
        // Non-finite samples are treated as silence.
        tap.push(&vec![f32::NAN; history]);
        assert_eq!(outlines(&tap.snapshot(0)), [0.0; 2 * POINTS]);
    }

    #[test]
    fn shared_scope_round_trips_without_blocking() {
        let shared = SharedScope::default();
        assert_eq!(shared.load(), ScopeSnapshot::default());
        let mut snapshot = ScopeSnapshot {
            onsets: 42,
            ..ScopeSnapshot::default()
        };
        for (index, point) in snapshot.upper.iter_mut().enumerate() {
            *point = index as f32 / POINTS as f32 - 0.5;
            snapshot.lower[index] = -*point;
        }
        shared.store(&snapshot);
        assert_eq!(shared.load(), snapshot);
        snapshot.onsets = u32::MAX;
        shared.store(&snapshot);
        assert_eq!(shared.load().onsets, u32::MAX);
    }

    #[test]
    fn shared_scope_is_read_while_another_thread_writes() {
        use std::sync::atomic::AtomicUsize;
        const MIN_WRITES: u32 = 2_000;
        const MIN_READS: usize = 1_000;
        let shared = std::sync::Arc::new(SharedScope::default());
        // Reads that overlapped the writer; it keeps writing until there
        // are enough, so the overlap is guaranteed rather than left to timing.
        let reads = std::sync::Arc::new(AtomicUsize::new(0));
        let writer = {
            let (shared, reads) = (shared.clone(), reads.clone());
            std::thread::spawn(move || {
                let mut onsets = 0;
                while onsets < MIN_WRITES || reads.load(Ordering::Acquire) < MIN_READS {
                    onsets += 1;
                    shared.store(&ScopeSnapshot {
                        // Exact in f32 far beyond any count reached here.
                        upper: [onsets as f32; POINTS],
                        lower: [onsets as f32; POINTS],
                        onsets,
                    });
                }
                onsets
            })
        };
        let mut last = 0;
        while !writer.is_finished() {
            let snapshot = shared.load();
            // Points are never older than the onset count they are read with.
            let floor = snapshot.onsets as f32;
            assert!(outlines(&snapshot).iter().all(|&point| point >= floor));
            assert!(snapshot.onsets >= last);
            last = snapshot.onsets;
            reads.fetch_add(1, Ordering::Release);
        }
        let written = writer.join().unwrap();
        assert!(reads.load(Ordering::Acquire) >= MIN_READS);
        assert_eq!(shared.load().onsets, written);
    }
}
