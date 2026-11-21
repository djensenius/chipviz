//! Live microphone input: a third frame source next to procedural and UDP.
//!
//! `capture` owns the cpal stream and an analysis thread; `analysis` is the
//! pure, deterministic signal processing. The render thread only reads the
//! latest [`AudioFeatures`] from [`SharedFeatures`] (two atomic loads) and
//! never waits on audio. `scope` is the renderer-internal waveform and onset
//! side channel for the mic layers; it is not part of the control frame.

pub mod analysis;
mod capture;
pub mod scope;

use std::sync::atomic::{AtomicU64, Ordering};

pub use analysis::AudioFeatures;
pub use capture::{InputDevices, MicSource};

use crate::control_frame::{ControlFrame, MAX_NOTES, Note};

pub const BEAT_FLAG: u8 = 0x01;
pub const BAR_FLAG: u8 = 0x02;
pub const DROP_FLAG: u8 = 0x04;
const BEATS_PER_BAR: u16 = 4;
const MAX_BEAT_STEP: u16 = 4;

/// Latest analysed features, published without locks. Only the transport
/// fields are guaranteed consistent with each other: `bpm`, `beat_phase`, and
/// `beat_count` share one word (with `energy`, `bass`, `mid`, and `treble`).
/// `spectrum` is a separate word, so a reader may see it one analysis step
/// apart from the levels; that is harmless for rendering.
#[derive(Default)]
pub struct SharedFeatures {
    transport: AtomicU64,
    spectrum: AtomicU64,
}

impl SharedFeatures {
    pub fn store(&self, features: AudioFeatures) {
        let [count_low, count_high] = features.beat_count.to_le_bytes();
        let transport = [
            features.bpm,
            features.beat_phase,
            count_low,
            count_high,
            features.energy,
            features.bass,
            features.mid,
            features.treble,
        ];
        self.spectrum
            .store(u64::from_le_bytes(features.spectrum), Ordering::Relaxed);
        self.transport
            .store(u64::from_le_bytes(transport), Ordering::Release);
    }

    pub fn load(&self) -> AudioFeatures {
        let transport = self.transport.load(Ordering::Acquire).to_le_bytes();
        AudioFeatures {
            bpm: transport[0],
            beat_phase: transport[1],
            beat_count: u16::from_le_bytes([transport[2], transport[3]]),
            energy: transport[4],
            bass: transport[5],
            mid: transport[6],
            treble: transport[7],
            spectrum: self.spectrum.load(Ordering::Relaxed).to_le_bytes(),
        }
    }
}

/// Builds a mic-driven frame. `frame`, `scene`, and `palette` come from
/// `base` (the procedural timeline), so scene selection is unchanged; the
/// audio fields come from `features`. Up to four forward beats, including
/// counter wrap, produce one beat flag; a bar flag is emitted only when the
/// current count is a bar boundary. Missed bars are not reconstructed.
/// Larger jumps resynchronize without an event. A small forward reset is
/// indistinguishable from progress without a source epoch.
pub fn mic_frame(
    base: ControlFrame,
    features: AudioFeatures,
    previous_beat_count: Option<u16>,
) -> ControlFrame {
    let beats =
        previous_beat_count.map_or(0, |previous| features.beat_count.wrapping_sub(previous));
    let beat = (1..=MAX_BEAT_STEP).contains(&beats);
    let mut flags = 0;
    if beat {
        flags |= BEAT_FLAG;
        if features.beat_count.is_multiple_of(BEATS_PER_BAR) {
            flags |= BAR_FLAG;
        }
    }
    ControlFrame {
        frame: base.frame,
        bpm: features.bpm,
        beat_phase: features.beat_phase,
        beat_count: features.beat_count,
        energy: features.energy,
        bass: features.bass,
        mid: features.mid,
        treble: features.treble,
        spectrum: features.spectrum,
        scene: base.scene,
        palette: base.palette,
        flags,
        note_count: 0,
        notes: [Note::default(); MAX_NOTES],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AudioFeatures {
        AudioFeatures {
            bpm: 128,
            beat_phase: 200,
            beat_count: 0xBEEF,
            energy: 1,
            bass: 2,
            mid: 3,
            treble: 4,
            spectrum: [10, 20, 30, 40, 50, 60, 70, 80],
        }
    }

    #[test]
    fn shared_features_round_trip() {
        let shared = SharedFeatures::default();
        assert_eq!(shared.load(), AudioFeatures::default());
        shared.store(sample());
        assert_eq!(shared.load(), sample());
    }

    #[test]
    fn mic_frame_keeps_procedural_scene_and_takes_audio_fields() {
        let base = ControlFrame::procedural(500);
        let frame = mic_frame(base, sample(), None);
        assert_eq!(
            (frame.frame, frame.scene, frame.palette),
            (base.frame, base.scene, base.palette)
        );
        assert_eq!(
            (frame.bpm, frame.beat_phase, frame.beat_count),
            (128, 200, 0xBEEF)
        );
        assert_eq!(
            (frame.energy, frame.bass, frame.mid, frame.treble),
            (1, 2, 3, 4)
        );
        assert_eq!(frame.spectrum, sample().spectrum);
        assert_eq!(frame.note_count, 0);
        assert_eq!(frame.flags, 0);
        assert!(!frame.beat_onset());
    }

    #[test]
    fn mic_frame_flags_bounded_beats_and_current_bar() {
        let base = ControlFrame::procedural(0);
        let mut features = sample();
        for (previous, current, expected) in [
            (None, 7, 0),
            (Some(7), 7, 0),
            (Some(8), 8, 0),
            (Some(6), 7, BEAT_FLAG),
            (Some(7), 8, BEAT_FLAG | BAR_FLAG),
            (Some(7), 9, BEAT_FLAG),
            (Some(7), 11, BEAT_FLAG),
            (Some(9), 12, BEAT_FLAG | BAR_FLAG),
            (Some(8), 10, BEAT_FLAG),
            (Some(1), 5, BEAT_FLAG),
            (Some(u16::MAX), 0, BEAT_FLAG | BAR_FLAG),
            (Some(u16::MAX - 1), 1, BEAT_FLAG),
            (Some(u16::MAX - 3), 0, BEAT_FLAG | BAR_FLAG),
            (Some(1000), 0, 0),
            (Some(0), 1, BEAT_FLAG),
            (Some(3), 2, 0),
            (Some(0), 5, 0),
            (Some(u16::MAX - 4), 0, 0),
        ] {
            features.beat_count = current;
            assert_eq!(
                mic_frame(base, features, previous).flags,
                expected,
                "previous {previous:?}, current {current}"
            );
        }
    }
}
