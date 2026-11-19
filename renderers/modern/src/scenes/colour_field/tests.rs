//! Normative fixtures F0 to F24 from `shared/specs/colour-field-v0.md`,
//! driven straight through the step function. F23 is a host-side check and
//! belongs to the host bridges, not to this renderer.

use super::state::{ColourField, Config, Controls, Display, PaletteSource, Trace};
use super::*;
use crate::control_frame::{ControlFrame, Note};

const STEPS: usize = 600;
const HELD: [u8; 2] = [0, 4];
const SPECTRAL: [u8; 2] = [1, 5];
const CUTS: [u8; 2] = [2, 6];
const FLASH: [u8; 2] = [3, 7];
/// Palettes whose armed fixtures flash: 3 and 7 with `unmeasured-flash`,
/// none in default builds, where armed 3 and 7 equal 2 and 6.
#[cfg(feature = "unmeasured-flash")]
const ARMED: [u8; 2] = FLASH;
#[cfg(not(feature = "unmeasured-flash"))]
const ARMED: [u8; 0] = [];

/// Unlisted fields are 0; `scene = 7` and `frame = step`.
fn frame(step: usize) -> ControlFrame {
    ControlFrame {
        frame: step as u16,
        bpm: 0,
        beat_phase: 0,
        beat_count: 0,
        energy: 0,
        bass: 0,
        mid: 0,
        treble: 0,
        spectrum: [0; 8],
        scene: SCENE,
        palette: 0,
        flags: 0,
        note_count: 0,
        notes: [Note::default(); 4],
    }
}

/// `e`, bands and spectrum all `value`.
fn flat(step: usize, value: u8) -> ControlFrame {
    ControlFrame {
        energy: value,
        bass: value,
        mid: value,
        treble: value,
        spectrum: [value; 8],
        ..frame(step)
    }
}

fn with_note(mut frame: ControlFrame, notes: &[(u8, u8)]) -> ControlFrame {
    for (slot, &(note, velocity)) in frame.notes.iter_mut().zip(notes) {
        *slot = Note { note, velocity };
    }
    frame.note_count = notes.len() as u8;
    frame
}

/// The fixtures' local `p`, pre-armed by operator arming for 3 and 7.
fn local(p: u8, armed: bool) -> Config {
    Config {
        allow_flash: true,
        palette: PaletteSource::Local { p, armed },
        pin: None,
    }
}

/// The default `data` role: wire palettes, no local arming.
fn wire() -> Config {
    Config::default()
}

/// Steps once per entry of `frames(step)` batch (resampling when a batch
/// holds several frames).
fn run_frames(
    config: Config,
    mut frames: impl FnMut(usize) -> Vec<ControlFrame>,
    mut controls: impl FnMut(usize) -> Controls,
) -> Vec<Option<Trace>> {
    let mut field = ColourField::new(config);
    (0..STEPS)
        .map(|step| {
            for frame in frames(step) {
                field.observe(&frame);
            }
            field.step(controls(step))
        })
        .collect()
}

fn run(config: Config, input: impl Fn(usize) -> ControlFrame) -> Vec<Trace> {
    run_frames(config, |step| vec![input(step)], |_| Controls::default())
        .into_iter()
        .map(|trace| trace.expect("scene 7 is shown"))
        .collect()
}

/// Traces for local `p = 0..7`; also checks that unarmed 3 and 7 equal 2
/// and 6, and that default builds never flash, even when armed.
fn all_palettes(input: impl Fn(usize) -> ControlFrame + Copy) -> Vec<Vec<Trace>> {
    let traces: Vec<_> = (0..8).map(|p| run(local(p, p & 3 == 3), input)).collect();
    for p in FLASH {
        assert_eq!(
            run(local(p, false), input),
            traces[usize::from(p - 1)],
            "unarmed {p}"
        );
        if !FLASH_BUILD {
            assert_eq!(
                traces[usize::from(p)],
                traces[usize::from(p - 1)],
                "armed {p}"
            );
        }
    }
    traces
}

fn changes(traces: &[Trace]) -> Vec<usize> {
    (1..traces.len())
        .filter(|&step| traces[step] != traces[step - 1])
        .collect()
}

fn pulse_starts(traces: &[Trace]) -> Vec<usize> {
    (0..traces.len())
        .filter(|&step| traces[step].lvl[0] == 3 && (step == 0 || traces[step - 1].lvl[0] != 3))
        .collect()
}

fn max_changes_per_60(traces: &[Trace]) -> usize {
    let steps = changes(traces);
    steps
        .iter()
        .map(|&start| {
            steps
                .iter()
                .filter(|&&step| (start..start + 60).contains(&step))
                .count()
        })
        .max()
        .unwrap_or(0)
}

fn levels(traces: &[Trace], step: usize) -> [u8; 3] {
    traces[step].lvl
}

/// One field per call, in wire order: `x = (x * 25173 + 13849) & 0xFFFF`.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = (self.0 * 25173 + 13849) & 0xFFFF;
        self.0
    }

    /// Fills energy, bass, mid, treble, then spectrum 0..7.
    fn fields(&mut self, frame: ControlFrame, mut value: impl FnMut(u32) -> u8) -> ControlFrame {
        let mut fields = [0; 12];
        for field in &mut fields {
            *field = value(self.next());
        }
        let mut spectrum = [0; 8];
        spectrum.copy_from_slice(&fields[4..]);
        ControlFrame {
            energy: fields[0],
            bass: fields[1],
            mid: fields[2],
            treble: fields[3],
            spectrum,
            ..frame
        }
    }
}

fn precomputed(frames: Vec<ControlFrame>) -> impl Fn(usize) -> ControlFrame + Copy {
    let frames: &'static [ControlFrame] = Box::leak(frames.into_boxed_slice());
    move |step| frames[step]
}

#[test]
fn f0_silence() {
    let traces = all_palettes(frame);
    for p in 0..8 {
        let traces = &traces[usize::from(p)];
        assert!(changes(traces).is_empty(), "p {p}");
        let expected = if SPECTRAL.contains(&p) || ARMED.contains(&p) {
            [0; 3]
        } else {
            [3, 3, 0]
        };
        assert_eq!((traces[0].layout, traces[0].combo), (0, 0), "p {p}");
        assert_eq!(traces[0].lvl, expected, "p {p}");
    }
}

fn f1(step: usize) -> ControlFrame {
    ControlFrame {
        energy: 160,
        bass: 40,
        mid: 200,
        treble: 40,
        spectrum: [40, 40, 120, 200, 200, 120, 40, 40],
        flags: if (300..=305).contains(&step) { 0x04 } else { 0 },
        ..frame(step)
    }
}

#[test]
fn f1_tone_and_drop() {
    let traces = all_palettes(f1);
    for p in HELD {
        let traces = &traces[usize::from(p)];
        assert_eq!(traces[0].combo, 1);
        assert_eq!(changes(traces), [300]);
        assert_eq!((traces[300].layout, traces[300].combo), (1, 1));
    }
    for p in SPECTRAL {
        let traces = &traces[usize::from(p)];
        assert_eq!(levels(traces, 0), [0, 3, 0]);
        assert_eq!(changes(traces), [300]);
        assert_eq!((traces[300].layout, traces[300].combo), (1, 1));
        assert_eq!(levels(traces, 599), [0, 3, 0]);
    }
    for p in CUTS {
        let traces = &traces[usize::from(p)];
        assert_eq!(changes(traces), [300]);
        assert_eq!((traces[300].layout, traces[300].combo), (1, 1));
    }
    for p in ARMED {
        let traces = &traces[usize::from(p)];
        assert_eq!(pulse_starts(traces), [300]);
        assert_eq!(changes(traces), [300, 304]);
        let pulse = traces[300];
        assert_eq!((pulse.layout, pulse.combo, pulse.rot), (0, 1, 1));
        assert_eq!(levels(traces, 299), [1, 1, 0]);
    }
}

#[test]
fn f2_noise() {
    let mut lcg = Lcg(1);
    let input = precomputed(
        (0..STEPS)
            .map(|step| lcg.fields(frame(step), |x| (150 + (x >> 8) % 25 - 12) as u8))
            .collect(),
    );
    let traces = all_palettes(input);
    for (p, traces) in traces.iter().enumerate() {
        assert!(changes(traces).is_empty(), "p {p}: {:?}", changes(traces));
    }
    for p in SPECTRAL {
        assert_eq!(levels(&traces[usize::from(p)], 0), [2, 2, 0]);
    }
    for p in ARMED {
        assert_eq!(levels(&traces[usize::from(p)], 0), [1, 1, 0]);
    }
}

#[test]
fn f3_aperiodic() {
    let spikes = [37, 101, 113, 250, 400, 523];
    let traces = all_palettes(|step| {
        let spiking = spikes
            .iter()
            .any(|&spike| (spike..spike + 2).contains(&step));
        flat(step, if spiking { 220 } else { 40 })
    });
    for p in HELD {
        assert!(changes(&traces[usize::from(p)]).is_empty());
    }
    for p in CUTS {
        assert_eq!(changes(&traces[usize::from(p)]), [37, 101, 250, 400, 523]);
    }
    for p in ARMED {
        assert_eq!(pulse_starts(&traces[usize::from(p)]), spikes);
    }
}

#[test]
fn f4_notes() {
    let traces = all_palettes(|step| {
        let notes: &[(u8, u8)] = match step {
            45 | 46 | 130 | 352 => &[(60, 100)],
            200 => &[(60, 40)],
            _ => &[],
        };
        with_note(flat(step, 80), notes)
    });
    for p in CUTS {
        assert_eq!(changes(&traces[usize::from(p)]), [45, 130, 352]);
    }
    for p in ARMED {
        assert_eq!(pulse_starts(&traces[usize::from(p)]), [45, 130, 352]);
    }
}

fn f5(step: usize) -> ControlFrame {
    let beat = step > 0 && step.is_multiple_of(30);
    let loud = (1..=19).any(|k| (30 * k..30 * k + 4).contains(&step));
    let mut spectrum = [60; 8];
    if loud {
        spectrum[..2].fill(200);
    }
    ControlFrame {
        energy: if loud { 180 } else { 60 },
        bass: if loud { 200 } else { 60 },
        mid: 60,
        treble: 60,
        spectrum,
        flags: u8::from(beat)
            | if beat && step.is_multiple_of(120) {
                0x02
            } else {
                0
            },
        ..frame(step)
    }
}

#[test]
fn f5_rhythmic() {
    let beats: Vec<_> = (1..=19).map(|k| 30 * k).collect();
    let traces = all_palettes(f5);
    for p in HELD {
        assert!(changes(&traces[usize::from(p)]).is_empty());
    }
    for p in CUTS {
        let traces = &traces[usize::from(p)];
        assert_eq!(changes(traces), beats);
        let layouts: Vec<_> = changes(traces)
            .into_iter()
            .filter(|&step| traces[step].layout != traces[step - 1].layout)
            .map(|step| (step, traces[step].layout))
            .collect();
        assert_eq!(layouts, [(120, 1), (240, 2), (360, 3), (480, 0)]);
    }
    for p in ARMED {
        assert_eq!(pulse_starts(&traces[usize::from(p)]), beats);
    }
}

#[test]
fn f6_floor() {
    let mut lcg = Lcg(1);
    let input = precomputed(
        (0..STEPS)
            .map(|step| match step {
                60..=419 => lcg.fields(frame(step), |x| ((x >> 8) % 13) as u8),
                480 | 481 => flat(step, 220),
                _ => flat(step, 120),
            })
            .collect(),
    );
    let traces = all_palettes(input);
    for p in SPECTRAL {
        let traces = &traces[usize::from(p)];
        assert_eq!(changes(traces), [60, 70, 420, 430, 480, 490]);
        let at = |step| levels(traces, step);
        assert_eq!([at(59), at(60), at(70)], [[2, 2, 0], [1, 1, 0], [0; 3]]);
        assert_eq!(
            [at(420), at(430), at(480), at(490)],
            [[1, 1, 0], [2, 2, 0], [3, 3, 0], [2, 2, 0]]
        );
    }
    for p in CUTS {
        assert_eq!(changes(&traces[usize::from(p)]), [420, 480]);
    }
    for p in ARMED {
        let traces = &traces[usize::from(p)];
        assert_eq!(pulse_starts(traces), [420, 480]);
        // The dim rest goes dark when silence starts, before step 130.
        let dark = (0..STEPS).find(|&step| traces[step].lvl[0] == 0).unwrap();
        assert!((60..130).contains(&dark), "{dark}");
        assert!(
            changes(traces)
                .iter()
                .all(|&step| step == dark || step >= 420)
        );
    }
}

#[test]
fn f7_sweep() {
    let traces = all_palettes(|step| {
        let (bands, lit) = if step < 300 {
            ([200, 60, 60], 0..4)
        } else {
            ([60, 60, 200], 4..8)
        };
        let mut spectrum = [60; 8];
        spectrum[lit].fill(200);
        ControlFrame {
            energy: 150,
            bass: bands[0],
            mid: bands[1],
            treble: bands[2],
            spectrum,
            ..frame(step)
        }
    });
    for p in HELD {
        let traces = &traces[usize::from(p)];
        let steps = changes(traces);
        assert_eq!(steps.len(), 1);
        assert!((330..=345).contains(&steps[0]), "{steps:?}");
        assert_eq!((traces[0].combo, traces[599].combo), (0, 2));
        assert_eq!(traces[599].layout, 0);
    }
    for p in SPECTRAL {
        let traces = &traces[usize::from(p)];
        assert_eq!(changes(traces), [300, 310]);
        let at = |step| levels(traces, step);
        assert_eq!([at(0), at(300), at(310)], [[3, 1, 0], [2, 2, 0], [1, 3, 0]]);
    }
    for p in CUTS {
        assert_eq!(changes(&traces[usize::from(p)]), [300]);
    }
    for p in ARMED {
        assert_eq!(pulse_starts(&traces[usize::from(p)]), [300]);
    }
}

#[test]
fn f8_held_note() {
    let traces = all_palettes(|step| {
        let notes: &[(u8, u8)] = match step {
            100..=199 => &[(60, 100)],
            200..=299 => &[(60, 100), (64, 100)],
            _ => &[],
        };
        with_note(flat(step, 80), notes)
    });
    for p in CUTS {
        assert_eq!(changes(&traces[usize::from(p)]), [100, 200]);
    }
    for p in ARMED {
        assert_eq!(pulse_starts(&traces[usize::from(p)]), [100, 200]);
    }
}

#[test]
fn f9_beat_and_bar() {
    let beat = |step: usize| step >= 20 && (step - 20).is_multiple_of(40) && step <= 580;
    let bar = |step: usize| step >= 40 && (step - 40).is_multiple_of(120) && step <= 520;
    let traces = all_palettes(|step| ControlFrame {
        flags: u8::from(beat(step)) | u8::from(bar(step)) << 1,
        ..flat(step, 100)
    });
    let expected: Vec<_> = (0..STEPS).filter(|&step| beat(step) || bar(step)).collect();
    assert_eq!(expected.len(), 20);
    for p in HELD {
        assert!(changes(&traces[usize::from(p)]).is_empty());
    }
    for p in CUTS {
        assert_eq!(changes(&traces[usize::from(p)]), expected);
    }
    for p in ARMED {
        assert_eq!(pulse_starts(&traces[usize::from(p)]), expected);
    }
}

fn f10(step: usize) -> ControlFrame {
    let palette = match step {
        100..=109 => 2,
        200.. => 3,
        _ => 0,
    };
    ControlFrame {
        flags: 0,
        palette,
        ..f1(step)
    }
}

#[test]
fn f10_wire_gate() {
    let traces = run(wire(), f10);
    assert!(traces[..230].iter().all(|trace| trace.mode == 0));
    if FLASH_BUILD {
        assert!(traces[230..].iter().all(|trace| trace.mode == 3));
        assert_eq!(levels(&traces, 230), [1, 1, 0]);
    } else {
        // The armed request runs unarmed: hard cuts from the same step.
        assert!(traces[230..].iter().all(|trace| trace.mode == 2));
        assert_eq!(levels(&traces, 230), [3, 3, 0]);
    }
    assert_eq!(changes(&traces), [230]);
}

#[test]
fn wire_selection_arms_without_the_renderer_flag() {
    // Route (b): the host's `--allow-flash` is the opt-in, so the renderer's
    // own flag changes nothing on the wire path.
    assert!(!Config::default().allow_flash);
    let flagged = Config {
        allow_flash: true,
        ..Config::default()
    };
    for input in [f10 as fn(usize) -> ControlFrame, f22] {
        let traces = run(Config::default(), input);
        assert_eq!(traces.iter().any(|trace| trace.mode == 3), FLASH_BUILD);
        assert_eq!(traces, run(flagged, input));
    }
}

#[test]
fn f11_pinned() {
    for p in CUTS {
        let config = Config {
            pin: Some(3),
            ..local(p, false)
        };
        let traces = run(config, f5);
        assert!(traces.iter().all(|trace| trace.layout == 3));
        let cuts = changes(&traces);
        assert_eq!(cuts.len(), 19);
        for (index, &step) in cuts.iter().enumerate() {
            let rotated = traces[step].rot != traces[step - 1].rot;
            let combo = traces[step].combo != traces[step - 1].combo;
            // Actions cycle rotate, combo, rotate, rotate (pinned layout).
            assert_eq!((rotated, combo), (index % 4 != 1, index % 4 == 1), "{step}");
        }
    }
}

#[test]
fn f12_mode_toggle() {
    let pulses = |step: usize| {
        if !(60..=594).contains(&step) {
            None
        } else if (step - 60).is_multiple_of(12) {
            Some(1)
        } else if step >= 66 && (step - 66).is_multiple_of(12) {
            Some(0)
        } else {
            None
        }
    };
    let traces: Vec<_> = run_frames(
        local(0, false),
        |step| vec![frame(step)],
        |step| Controls {
            operator_palette: pulses(step),
            ..Controls::default()
        },
    )
    .into_iter()
    .map(Option::unwrap)
    .collect();
    let steps = changes(&traces);
    assert_eq!(steps[..7], [61, 81, 101, 127, 147, 167, 193]);
    for pair in steps.windows(2) {
        assert!([20, 26].contains(&(pair[1] - pair[0])), "{steps:?}");
    }
    for &step in &steps {
        let trace = traces[step];
        let expected = if trace.mode == 0 { [3, 3, 0] } else { [0; 3] };
        assert_eq!(trace.lvl, expected);
        assert_ne!(trace.mode, traces[step - 1].mode);
    }
    assert!(max_changes_per_60(&traces) <= 3);
}

#[test]
fn f13_silence_spikes() {
    let traces = all_palettes(|step| ControlFrame {
        energy: if step > 0 && step % 31 == 0 { 30 } else { 0 },
        bass: 250,
        mid: 250,
        treble: 250,
        ..frame(step)
    });
    let expected: Vec<_> = (0..)
        .map(|cycle| 30 + 62 * cycle)
        .take_while(|&start| start < STEPS)
        .flat_map(|start| [start, start + 10, start + 20, start + 30])
        .filter(|&step| step < STEPS)
        .collect();
    for p in SPECTRAL {
        let traces = &traces[usize::from(p)];
        assert_eq!(changes(traces), expected);
        let at = |step| levels(traces, step);
        assert_eq!(
            [at(30), at(40), at(50), at(60), at(61), at(92)],
            [[0; 3], [1, 1, 0], [2, 2, 0], [3, 3, 0], [3, 3, 0], [0; 3]]
        );
        assert!(max_changes_per_60(traces) <= 4);
    }
    for p in HELD.into_iter().chain(CUTS).chain(FLASH) {
        assert!(changes(&traces[usize::from(p)]).is_empty(), "p {p}");
    }
}

#[test]
fn f14_dim_rest_attack() {
    let traces = all_palettes(|step| ControlFrame {
        energy: if step % 5 == 0 { 255 } else { 0 },
        ..frame(step)
    });
    for p in ARMED {
        let traces = &traces[usize::from(p)];
        let expected: Vec<_> = (5..STEPS).step_by(10).collect();
        assert_eq!(pulse_starts(traces), expected);
        assert_eq!(expected.len(), 60);
        for step in 0..STEPS {
            let lit = step >= 5 && (step - 5) % 10 < 4;
            let rest = if step <= 28 { 1 } else { 0 };
            let level = if lit { 3 } else { rest };
            assert_eq!(levels(traces, step), [level, level, 0], "step {step}");
        }
        assert!(max_changes_per_60(traces) <= 12);
    }
    for p in SPECTRAL.into_iter().chain(HELD) {
        assert!(changes(&traces[usize::from(p)]).is_empty(), "p {p}");
    }
    for p in CUTS {
        let expected: Vec<_> = (5..STEPS).step_by(20).collect();
        assert_eq!(changes(&traces[usize::from(p)]), expected);
        assert_eq!(expected.len(), 30);
    }
}

#[test]
fn f15_resampled_onsets() {
    let source = |index: usize| ControlFrame {
        frame: index as u16,
        flags: (index % 2) as u8,
        ..flat(index, 100)
    };
    let run = |p: u8| -> Vec<Trace> {
        run_frames(
            local(p, p & 3 == 3),
            |step| vec![source(2 * step), source(2 * step + 1)],
            |_| Controls::default(),
        )
        .into_iter()
        .map(Option::unwrap)
        .collect()
    };
    for p in CUTS {
        let expected: Vec<_> = (1..STEPS).step_by(20).collect();
        assert_eq!(expected.len(), 30);
        assert_eq!(changes(&run(p)), expected);
    }
    for p in ARMED {
        let expected: Vec<_> = (1..STEPS).step_by(8).collect();
        assert_eq!(expected.len(), 75);
        assert_eq!(pulse_starts(&run(p)), expected);
    }
}

#[test]
fn f16_armed_toggle_keeps_pulse_spacing() {
    let request = |step: usize| match step {
        98.. if (step - 98).is_multiple_of(60) => Some(7),
        128.. if (step - 128).is_multiple_of(60) => Some(3),
        _ => None,
    };
    let traces: Vec<_> = run_frames(
        local(3, true),
        |step| {
            vec![ControlFrame {
                flags: (step % 2) as u8,
                ..flat(step, 100)
            }]
        },
        |step| Controls {
            operator_palette: request(step),
            ..Controls::default()
        },
    )
    .into_iter()
    .map(Option::unwrap)
    .collect();
    let family_changes: Vec<_> = (1..STEPS)
        .filter(|&step| traces[step].family != traces[step - 1].family)
        .collect();
    if FLASH_BUILD {
        let expected: Vec<_> = (99..STEPS).step_by(30).collect();
        assert_eq!(family_changes, expected);
        assert!(traces.iter().all(|trace| trace.mode == 3));
        let starts: Vec<_> = (1..STEPS).step_by(8).collect();
        assert_eq!(starts.len(), 75);
        assert_eq!(pulse_starts(&traces), starts);
    } else {
        // Operator arming with `--allow-flash` still never latches.
        assert!(!family_changes.is_empty());
        assert!(traces.iter().all(|trace| trace.mode == 2));
    }
}

#[test]
fn f17_silence_cut_off() {
    let input = |step: usize| ControlFrame {
        energy: if step > 0 && step.is_multiple_of(31) {
            24
        } else {
            0
        },
        flags: (step % 2) as u8,
        ..frame(step)
    };
    for p in ARMED {
        let mut field = ColourField::new(local(p, true));
        let mut traces = Vec::new();
        let mut entries = Vec::new();
        for step in 0..STEPS {
            let was_silent = field.is_silent();
            field.observe(&input(step));
            traces.push(field.step(Controls::default()).unwrap());
            if field.is_silent() && !was_silent {
                entries.push(step);
            }
        }
        let expected: Vec<_> = (1..=19).map(|k| 31 * k - 1).collect();
        assert_eq!(entries, expected);
        // Entries that end a lit pulse early (including `pulse = 1`).
        let cut_off: Vec<_> = entries
            .iter()
            .copied()
            .filter(|&step| traces[step - 1].lvl[0] == 3)
            .collect();
        assert_eq!(
            cut_off,
            [61, 92, 123, 154, 247, 278, 309, 340, 433, 464, 495, 526]
        );
        assert!(cut_off.iter().all(|&step| traces[step].lvl[0] == 0));
        let starts = pulse_starts(&traces);
        assert_eq!(starts.len(), 75);
        let early: Vec<_> = (0..23).map(|k| 1 + 8 * k).collect();
        assert_eq!(starts[..23], early[..]);
        assert_eq!(starts[23..25], [187, 195]);
        assert!(!starts.contains(&127));
        assert!(starts.windows(2).all(|pair| pair[1] - pair[0] >= 8));
        assert!(max_changes_per_60(&traces) <= 16);
    }
    // Unarmed 3 and 7 run as 2 and 6.
    for p in ARMED {
        assert_eq!(run(local(p, false), input), run(local(p - 1, false), input));
    }
}

#[test]
fn f18_held_across_entry() {
    let input = |step: usize| {
        let held = step <= 9;
        let frame = ControlFrame {
            flags: u8::from(held || step == 300),
            ..flat(step, 100)
        };
        if held {
            with_note(frame, &[(60, 100)])
        } else {
            frame
        }
    };
    for p in 0..8 {
        let config = local(p, p & 3 == 3);
        let full = run_frames(config, |step| vec![input(step)], |_| Controls::default());
        let resampled = run_frames(
            config,
            |step| {
                let frame = input(step);
                let doubled = |index: usize| ControlFrame {
                    frame: index as u16,
                    ..frame
                };
                vec![doubled(2 * step), doubled(2 * step + 1)]
            },
            |_| Controls::default(),
        );
        assert_eq!(full, resampled, "p {p}");
        let traces: Vec<_> = full.into_iter().map(Option::unwrap).collect();
        match p {
            0 | 4 => assert!(changes(&traces).is_empty()),
            2 | 6 => assert_eq!(changes(&traces), [300]),
            3 | 7 if FLASH_BUILD => assert_eq!(pulse_starts(&traces), [300]),
            // Default builds: armed 3 and 7 are 2 and 6 (`all_palettes`).
            3 | 7 => assert_eq!(changes(&traces), [300]),
            _ => {}
        }
    }
}

#[test]
fn f19_hid_digital_only() {
    let input = |step: usize| {
        let energy = if (320..380).contains(&step) { 200 } else { 64 };
        ControlFrame {
            energy,
            bass: energy,
            mid: energy,
            treble: energy,
            flags: u8::from(step >= 50 && (step - 50).is_multiple_of(60)),
            ..frame(step)
        }
    };
    let traces = all_palettes(input);
    let cuts = [50, 110, 170, 230, 290, 320, 350, 410, 470, 530, 590];
    for p in HELD {
        assert!(changes(&traces[usize::from(p)]).is_empty());
    }
    for p in SPECTRAL {
        let traces = &traces[usize::from(p)];
        assert_eq!(changes(traces), [320, 330, 380, 390]);
        let at = |step| levels(traces, step);
        assert_eq!(
            [at(0), at(320), at(330), at(380), at(390)],
            [[1, 1, 0], [2, 2, 0], [3, 3, 0], [2, 2, 0], [1, 1, 0]]
        );
    }
    for p in CUTS {
        assert_eq!(changes(&traces[usize::from(p)]), cuts);
    }
    for p in ARMED {
        let traces = &traces[usize::from(p)];
        assert_eq!(pulse_starts(traces), cuts);
        assert!(
            traces.iter().all(|trace| trace.lvl[0] != 0),
            "never silent or dark"
        );
    }
}

#[test]
fn f20_level_saturation() {
    let traces = all_palettes(|step| {
        let band = if (200..400).contains(&step) { 10 } else { 250 };
        ControlFrame {
            energy: 100,
            bass: band,
            mid: band,
            treble: band,
            ..frame(step)
        }
    });
    for p in SPECTRAL {
        let traces = &traces[usize::from(p)];
        assert_eq!(changes(traces), [200, 210, 220, 400, 410, 420]);
        let at = |step| levels(traces, step);
        assert_eq!(at(0), [3, 3, 0]);
        assert_eq!(
            [at(200), at(210), at(220), at(399)],
            [[2, 2, 0], [1, 1, 0], [0; 3], [0; 3]]
        );
        assert_eq!(
            [at(400), at(410), at(420), at(599)],
            [[1, 1, 0], [2, 2, 0], [3, 3, 0], [3, 3, 0]]
        );
    }
    for p in HELD {
        assert!(changes(&traces[usize::from(p)]).is_empty());
    }
    for p in CUTS {
        assert_eq!(changes(&traces[usize::from(p)]), [400]);
    }
    for p in ARMED {
        let traces = &traces[usize::from(p)];
        assert_eq!(pulse_starts(traces), [400]);
        assert!(
            (0..STEPS).all(|step| (400..404).contains(&step) || levels(traces, step) == [1, 1, 0])
        );
    }
}

#[test]
fn f21_scene_seven_debounce() {
    let scene = |step: usize| match step {
        100..=199 if step.is_multiple_of(2) => 7,
        200..=299 | 320..=449 => 7,
        _ => 6,
    };
    let mut field = ColourField::new(wire());
    let mut traces = Vec::new();
    for step in 0..STEPS {
        field.observe(&ControlFrame {
            scene: scene(step),
            ..flat(step, 100)
        });
        let trace = field.step(Controls::default());
        if trace.is_none() {
            assert_eq!(field.display(), Display::Native(6), "step {step}");
        }
        traces.push(trace);
    }
    assert!(traces[..230].iter().all(Option::is_none));
    assert!(traces[480..].iter().all(Option::is_none));
    let shown: Vec<_> = traces[230..480]
        .iter()
        .map(|trace| trace.unwrap())
        .collect();
    assert!(changes(&shown).is_empty());
    let trace = shown[0];
    assert_eq!(
        (trace.mode, trace.layout, trace.combo, trace.lvl),
        (0, 0, 0, [3, 3, 0])
    );
}

fn f22(step: usize) -> ControlFrame {
    let scene = match step {
        60..=299 if (step - 60) % 40 < 20 => 5,
        60..=299 => 6,
        _ => 7,
    };
    ControlFrame {
        scene,
        palette: if step < 60 { 0 } else { 3 },
        ..flat(step, 100)
    }
}

#[test]
fn f22_deferred_exit_never_arms() {
    let traces = run(wire(), f22);
    assert!(traces[..330].iter().all(|trace| trace.mode == 0));
    let mode = if FLASH_BUILD { 3 } else { 2 };
    assert!(traces[330..].iter().all(|trace| trace.mode == mode));
    if FLASH_BUILD {
        assert_eq!(levels(&traces, 330), [1, 1, 0]);
    }
}

#[test]
fn f24_relative_palette() {
    let ring = |step: usize| match step {
        0..=9 => 0,
        10..=19 => 1,
        20..=29 => 2,
        30..=129 => 3,
        _ => 2,
    };
    let input = |step: usize| ControlFrame {
        palette: ring(step),
        ..flat(step, 64)
    };
    let pulse = |step: usize| [10, 20, 30, 130].contains(&step);
    let relative: Vec<_> = run_frames(
        wire(),
        |step| vec![input(step)],
        |step| Controls {
            relative_palette: pulse(step),
            relative_scene: true,
            ..Controls::default()
        },
    )
    .into_iter()
    .map(Option::unwrap)
    .collect();
    assert!(relative[..60].iter().all(|trace| trace.mode == 0));
    assert!(relative[60..].iter().all(|trace| trace.mode == 2));
    assert_eq!(changes(&relative), [60]);

    // Absolute keys with scene key 8 held arm at 59, then E returns to 2.
    let absolute = run(wire(), |step| ControlFrame {
        palette: if step < 30 { 0 } else { ring(step) },
        ..input(step)
    });
    assert!(absolute[..60].iter().all(|trace| trace.mode == 0));
    if FLASH_BUILD {
        assert!(absolute[60..160].iter().all(|trace| trace.mode == 3));
        assert!(absolute[160..].iter().all(|trace| trace.mode == 2));
    } else {
        // The armed absolute path runs unarmed; E's 2 then changes nothing.
        assert!(absolute[60..].iter().all(|trace| trace.mode == 2));
        assert_eq!(changes(&absolute), [60]);
    }
}

#[test]
fn reentry_uses_the_change_budget() {
    // Native 6, then scene 7 from step 10: debounced entry at 40, not power-on.
    let mut field = ColourField::new(local(2, false));
    let mut traces = Vec::new();
    for step in 0..120 {
        field.observe(&ControlFrame {
            scene: if step < 10 { 6 } else { 7 },
            flags: u8::from(step % 2 == 1),
            ..flat(step, 100)
        });
        traces.push(field.step(Controls::default()));
    }
    assert!(traces[..40].iter().all(Option::is_none));
    let shown: Vec<_> = traces[40..].iter().map(|trace| trace.unwrap()).collect();
    // since_cut = 0 at entry 40: the first cut is at 40 + 21.
    assert_eq!(changes(&shown)[0] + 40, 61);
}

#[test]
fn pin_control_cycles_and_applies_under_budget() {
    let mut field = ColourField::new(local(0, false));
    let mut layouts = Vec::new();
    for step in 0..40 {
        field.observe(&flat(step, 100));
        let controls = Controls {
            pin_cycle: [5, 6, 7].contains(&step),
            ..Controls::default()
        };
        layouts.push(field.step(controls).unwrap().layout);
    }
    // V2 is already shown, so it applies for free at 5; H2 uses the
    // power-on budget at the end of 6; V3 waits for `lg` and `since_cut`
    // and applies at the end of 26.
    assert!(layouts[..7].iter().all(|&layout| layout == 0));
    assert!(layouts[7..27].iter().all(|&layout| layout == 1));
    assert!(layouts[27..].iter().all(|&layout| layout == 2));
}

#[test]
fn region_colours_scale_levels_and_rotate() {
    let trace = Trace {
        mode: 1,
        family: 0,
        layout: 2,
        combo: 0,
        rot: 1,
        lvl: [3, 2, 1],
    };
    let [first, second, third] = region_colours(&trace);
    assert_eq!(first, A_YELLOW);
    // (c * L + 1) / 3 for A.B at level 2 and A.R at level 1.
    assert_eq!(second, [11, 32, 128]);
    assert_eq!(third, [75, 5, 11]);
    let two = Trace {
        family: 1,
        layout: 1,
        combo: 1,
        rot: 0,
        lvl: [3, 3, 0],
        ..trace
    };
    assert_eq!(region_colours(&two), [B_GREEN, B_MAGENTA, BLACK]);
}

#[test]
fn boundaries_follow_the_integer_formula() {
    assert_eq!(boundaries(256, 3), [0, 85, 170, 256]);
    assert_eq!(boundaries(1920, 2), [0, 960, 1920]);
    assert_eq!(boundaries(1081, 3), [0, 360, 720, 1081]);
}

#[test]
fn build_covers_the_viewport_with_flat_regions() {
    for layout in 0..4 {
        let trace = Trace {
            mode: 0,
            family: 0,
            layout,
            combo: 0,
            rot: 0,
            lvl: [3, 3, 3],
        };
        let mut instances = Vec::new();
        build(&trace, 1920, 1080, true, &mut instances);
        assert_eq!(instances.len(), usize::from(regions(layout)));
        let axis = usize::from(layout % 2);
        let mut edge = -1.0;
        for instance in &instances {
            assert_eq!(instance.size[1 - axis], 2.0);
            assert!((instance.center[axis] - instance.size[axis] / 2.0 - edge).abs() < 1e-6);
            edge = instance.center[axis] + instance.size[axis] / 2.0;
            assert_eq!(instance.pattern, [0.0; 2]);
        }
        assert!((edge - 1.0).abs() < 1e-6);
    }
}

#[test]
fn step_clock_caps_sixty_steps_per_second() {
    let mut clock = StepClock::default();
    let at = |micros: u64| Duration::from_micros(micros);
    let allowed = (0..240)
        .filter(|&tick| clock.try_step(at(tick * 1_000_000 / 120)))
        .count();
    assert_eq!(allowed, 120);
    let mut clock = StepClock::default();
    assert!((0..600).all(|tick| clock.try_step(at(tick * 1_000_000 / 60))));
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn fingerprint() -> u64 {
    let mut fingerprint: u64 = 0xcbf2_9ce4_8422_2325;
    for p in 0..8 {
        for trace in run(local(p, false), f5).iter().step_by(7) {
            let mut instances = Vec::new();
            build(trace, 1920, 1080, true, &mut instances);
            for byte in bytemuck::cast_slice::<Instance, u8>(&instances) {
                fingerprint ^= u64::from(*byte);
                fingerprint = fingerprint.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    fingerprint
}

// Gated like the registry goldens: platform libm powf may differ in the last
// ulp elsewhere.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn instances_match_recorded_golden() {
    assert_eq!(fingerprint(), 0x0be3_3778_406b_9e09);
}

#[test]
fn build_linearises_only_for_srgb_attachments() {
    // Combo 0 of family 0 in V2: A_RED then A_BLUE.
    let trace = Trace {
        mode: 0,
        family: 0,
        layout: 0,
        combo: 0,
        rot: 0,
        lvl: [3, 3, 0],
    };
    // What each attachment stores for a shader output in [0, 1].
    let unorm = |value: f32| (value * 255.0).round() as u8;
    let srgb = |value: f32| {
        let encoded = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as u8
    };
    for (srgb_target, store) in [(true, &srgb as &dyn Fn(f32) -> u8), (false, &unorm)] {
        let mut instances = Vec::new();
        build(&trace, 1920, 1080, srgb_target, &mut instances);
        let stored: Vec<[u8; 3]> = instances
            .iter()
            .map(|instance| std::array::from_fn(|channel| store(instance.color[channel])))
            .collect();
        assert_eq!(
            stored,
            [[0xE0, 0x10, 0x20], [0x10, 0x30, 0xC0]],
            "sRGB {srgb_target}"
        );
    }
}

#[test]
fn native_scene_selection_ignores_the_step_cap_at_120_hz() {
    // 120 Hz frames with the live 60-steps-per-second cap: the first 60
    // frames use up the first second's steps.
    let mut field = ColourField::new(wire());
    let mut clock = StepClock::default();
    let mut controls = Controls::default();
    let scene = |tick: u64| match tick {
        0..72 => 0,
        72..74 => 2, // A two-frame selection at 600 ms.
        74..96 => 1,
        _ => 7,
    };
    for tick in 0..240 {
        let frame = ControlFrame {
            scene: scene(tick),
            ..flat(tick as usize, 100)
        };
        field.observe(&frame);
        let now = Duration::from_micros(tick * 1_000_000 / 120);
        let display = step_capped(&mut field, Some(&mut clock), now, &mut controls);
        if tick < 96 {
            assert_eq!(display, Display::Native(scene(tick)), "tick {tick}");
        } else if display != Display::Native(1) {
            // Scene 7 from 800 ms still waits for 30 debounce steps.
            assert!(tick >= 96 + 30, "tick {tick}: {display:?}");
        }
    }
    assert!(matches!(field.display(), Display::ColourField(_)));
}

/// Supporting evidence only, not a substitute for PEAT or a Harding check:
/// the most WCAG 2.3.1 general and red flashes in any 60-step (one second)
/// window, at a grid of screen points. A transition is an opposing change
/// of relative luminance by at least 0.1 with the darker state below 0.8,
/// or of red `(R - G - B) * 320` by more than 20 with either state at least
/// 80 % red; a flash is two opposing transitions.
fn wcag_flashes(traces: &[Trace]) -> (usize, usize) {
    let linear = |colour: [u8; 3]| colour.map(|channel| attachment_colour([channel; 3], true)[0]);
    let luminance = |[r, g, b]: [f32; 3]| 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let red = |[r, g, b]: [f32; 3]| ((r - g - b) * 320.0).max(0.0);
    let saturated = |[r, g, b]: [f32; 3]| r + g + b > 0.0 && r / (r + g + b) >= 0.8;
    let mut worst = (0, 0);
    for point in 0..9 {
        let at = |trace: &Trace| {
            let n = u32::from(regions(trace.layout));
            let axis = if trace.layout.is_multiple_of(2) {
                point % 3
            } else {
                point / 3
            };
            linear(region_colours(trace)[((2 * axis + 1) * n / 6) as usize])
        };
        let colours: Vec<_> = traces.iter().map(at).collect();
        let mut general = Vec::new();
        let mut reds = Vec::new();
        for (step, pair) in colours.windows(2).enumerate() {
            let [a, b] = [pair[0], pair[1]];
            let (la, lb) = (luminance(a), luminance(b));
            if (lb - la).abs() >= 0.1 && la.min(lb) < 0.8 {
                general.push((step + 1, lb > la));
            }
            if (red(b) - red(a)).abs() > 20.0 && (saturated(a) || saturated(b)) {
                reds.push((step + 1, red(b) > red(a)));
            }
        }
        let most = |transitions: &[(usize, bool)]| {
            // Keep only opposing transitions; count pairs per window.
            let mut opposing: Vec<usize> = Vec::new();
            let mut last = None;
            for &(step, up) in transitions {
                if last != Some(up) {
                    opposing.push(step);
                    last = Some(up);
                }
            }
            (0..opposing.len())
                .map(|start| {
                    opposing[start..]
                        .iter()
                        .take_while(|&&step| step < opposing[start] + 60)
                        .count()
                        / 2
                })
                .max()
                .unwrap_or(0)
        };
        worst = (worst.0.max(most(&general)), worst.1.max(most(&reds)));
    }
    worst
}

#[test]
fn default_build_rendered_sequences_pass_the_wcag_flash_count() {
    // The last input beats on every other step with wire palette 3.
    let beats = |step: usize| ControlFrame {
        flags: (step % 2) as u8,
        palette: 3,
        ..flat(step, 100)
    };
    let inputs: [fn(usize) -> ControlFrame; 6] = [f1, f5, f10, f22, frame, beats];
    // With `unmeasured-flash`, armed configs flash by design.
    if !FLASH_BUILD {
        for input in inputs {
            let mut configs: Vec<_> = (0..8).map(|p| local(p, p & 3 == 3)).collect();
            configs.push(wire());
            for config in configs {
                let (general, red) = wcag_flashes(&run(config, input));
                assert!(
                    general <= 3 && red <= 3,
                    "{config:?}: {general} general, {red} red"
                );
            }
        }
    } else {
        // The counter does see flash/gate when it can latch.
        let (general, red) = wcag_flashes(&run(local(3, true), beats));
        assert!(general > 3 && red > 3, "{general} general, {red} red");
    }
}

#[test]
fn allow_flash_prints_the_warning_only_in_flash_builds() {
    let expected = if FLASH_BUILD {
        FLASH_WARNING
    } else {
        NO_FLASH_NOTICE
    };
    assert_eq!(allow_flash_notice(), expected);
    assert!(NO_FLASH_NOTICE.contains("unmeasured-flash"));
}
