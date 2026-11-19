//! Renderer-internal waveform and onset side channel for the mic layers.
//!
//! The control frame carries no waveform, so the analysis thread also feeds
//! its mono samples to a [`ScopeTap`]. After each poll it publishes a
//! [`ScopeSnapshot`] (a trigger-aligned, decimated, gain-normalised
//! waveform plus the analyzer's onset count) to [`SharedScope`]. Both sides
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
/// steady tone stands still instead of scrolling.
const SEARCH: usize = 1024;
const HISTORY: usize = SPAN + SEARCH;
/// The gain normaliser's peak never drops below this (about -34 dBFS), so
/// room hiss stays small instead of being amplified to full height.
pub const PEAK_FLOOR: f32 = 0.02;
const PEAK_RELEASE_S: f32 = 1.5;

/// One published waveform: `points` are in -1..=1 (normalised by the recent
/// peak) and `onsets` counts the analyzer's onsets (wrapping).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeSnapshot {
    pub points: [f32; POINTS],
    pub onsets: u32,
}

impl Default for ScopeSnapshot {
    fn default() -> Self {
        Self {
            points: [0.0; POINTS],
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
        Self {
            ring: vec![0.0; HISTORY],
            head: 0,
            peak: PEAK_FLOOR,
            release: (-1.0 / (sample_rate.max(1.0) * PEAK_RELEASE_S)).exp(),
            scratch: vec![0.0; HISTORY],
        }
    }

    pub fn push(&mut self, samples: &[f32]) {
        for &sample in samples {
            let sample = if sample.is_finite() { sample } else { 0.0 };
            self.peak = sample.abs().max(self.peak * self.release);
            self.ring[self.head] = sample;
            self.head = (self.head + 1) % HISTORY;
        }
    }

    pub fn push_silence(&mut self, count: usize) {
        const CHUNK: [f32; 256] = [0.0; 256];
        let mut remaining = count.min(HISTORY);
        while remaining > 0 {
            let take = remaining.min(CHUNK.len());
            self.push(&CHUNK[..take]);
            remaining -= take;
        }
        // Long silences still decay the peak by their full length.
        if count > HISTORY {
            self.peak *= self.release.powf((count - HISTORY) as f32);
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
        let mut points = [0.0; POINTS];
        decimate(&self.scratch[start..start + SPAN], &mut points);
        let gain = 1.0 / self.peak.max(PEAK_FLOOR);
        for point in &mut points {
            *point = (*point * gain).clamp(-1.0, 1.0);
        }
        ScopeSnapshot { points, onsets }
    }
}

/// Start of the shown span in `history` (`HISTORY` samples, oldest first):
/// the latest rising zero crossing that leaves a full span after it, or the
/// newest span when there is none.
pub fn trigger(history: &[f32]) -> usize {
    let last = history.len().saturating_sub(SPAN);
    (1..=last)
        .rev()
        .find(|&index| history[index - 1] < 0.0 && history[index] >= 0.0)
        .unwrap_or(last)
}

/// Click-preserving decimation: each output point is `max + min - mean` of
/// its share of `samples`, i.e. the mean plus both extremes' deviations from
/// it. A ramp or a full cycle deviates evenly both ways and keeps its mean, so
/// steep tones do not alias or flicker with sub-sample shifts, while a short
/// one-sided click keeps most of its height. Empty shares (more points than
/// samples) repeat the nearest sample.
pub fn decimate(samples: &[f32], out: &mut [f32]) {
    let points = out.len();
    for (index, point) in out.iter_mut().enumerate() {
        let start = index * samples.len() / points;
        let end = ((index + 1) * samples.len() / points).max(start + 1);
        let share = samples
            .get(start..end.min(samples.len()))
            .unwrap_or_default();
        if share.is_empty() {
            *point = 0.0;
            continue;
        }
        let (low, high, sum) = share.iter().fold(
            (f32::INFINITY, f32::NEG_INFINITY, 0.0),
            |(low, high, sum), &sample| (low.min(sample), high.max(sample), sum + sample),
        );
        *point = high + low - sum / share.len() as f32;
    }
}

/// Wait-free single-writer publication of the latest [`ScopeSnapshot`].
pub struct SharedScope {
    points: [AtomicU32; POINTS],
    onsets: AtomicU32,
}

impl Default for SharedScope {
    fn default() -> Self {
        Self {
            points: std::array::from_fn(|_| AtomicU32::new(0)),
            onsets: AtomicU32::new(0),
        }
    }
}

impl SharedScope {
    pub fn store(&self, snapshot: &ScopeSnapshot) {
        for (slot, point) in self.points.iter().zip(snapshot.points) {
            slot.store(point.to_bits(), Ordering::Relaxed);
        }
        self.onsets.store(snapshot.onsets, Ordering::Release);
    }

    pub fn load(&self) -> ScopeSnapshot {
        let onsets = self.onsets.load(Ordering::Acquire);
        ScopeSnapshot {
            points: std::array::from_fn(|index| {
                f32::from_bits(self.points[index].load(Ordering::Relaxed))
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

    #[test]
    fn decimation_keeps_shape_and_short_clicks() {
        let mut out = [0.0; 8];
        let mut samples = vec![0.1; 64];
        samples[37] = -0.9;
        decimate(&samples, &mut out);
        for (index, point) in out.into_iter().enumerate() {
            let expected = if index == 4 { -0.775 } else { 0.1 };
            assert!((point - expected).abs() < 1e-6, "{index}: {point}");
        }
        // A steep ramp keeps its mean instead of jumping to an end.
        let ramp: Vec<f32> = (0..8).map(|n| n as f32 - 3.5).collect();
        decimate(&ramp, &mut out[..1]);
        assert!(out[0].abs() < 1e-6, "{}", out[0]);

        // Uneven and short inputs still fill every point.
        let mut out = [9.0; 5];
        decimate(&[1.0, -2.0, 3.0], &mut out);
        assert_eq!(out, [1.0, 1.0, -2.0, -2.0, 3.0]);
        let mut out = [9.0; 4];
        decimate(&[], &mut out);
        assert_eq!(out, [0.0; 4]);
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
        let first = shots[5].points;
        for shot in &shots[6..] {
            let drift = first
                .iter()
                .zip(shot.points)
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

    #[test]
    fn trigger_falls_back_to_the_newest_span() {
        let flat = vec![0.25; HISTORY];
        assert_eq!(trigger(&flat), SEARCH);
        let mut edge = vec![-1.0; HISTORY];
        edge[100..].fill(1.0);
        assert_eq!(trigger(&edge), 100);
    }

    #[test]
    fn silence_flattens_and_hiss_is_not_amplified() {
        let mut tap = ScopeTap::new(RATE);
        tap.push(&sine(4800, 220.0, 0.8, 0.0));
        tap.push_silence(HISTORY);
        assert_eq!(tap.snapshot(3).points, [0.0; POINTS]);
        // Long silence lets the peak fall to the floor; quiet hiss stays small.
        tap.push_silence(10 * RATE as usize);
        tap.push(&sine(HISTORY, 3000.0, 0.002, 0.0));
        let hiss = tap.snapshot(0).points;
        assert!(hiss.iter().all(|point| point.abs() <= 0.11), "{hiss:?}");
        // Non-finite samples are treated as silence.
        tap.push(&[f32::NAN; HISTORY]);
        assert_eq!(tap.snapshot(0).points, [0.0; POINTS]);
    }

    #[test]
    fn shared_scope_round_trips_without_blocking() {
        let shared = SharedScope::default();
        assert_eq!(shared.load(), ScopeSnapshot::default());
        let mut snapshot = ScopeSnapshot {
            onsets: 42,
            ..ScopeSnapshot::default()
        };
        for (index, point) in snapshot.points.iter_mut().enumerate() {
            *point = index as f32 / POINTS as f32 - 0.5;
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
                        points: [onsets as f32; POINTS],
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
            assert!(snapshot.points.iter().all(|&point| point >= floor));
            assert!(snapshot.onsets >= last);
            last = snapshot.onsets;
            reads.fetch_add(1, Ordering::Release);
        }
        let written = writer.join().unwrap();
        assert!(reads.load(Ordering::Acquire) >= MIN_READS);
        assert_eq!(shared.load().onsets, written);
    }
}
