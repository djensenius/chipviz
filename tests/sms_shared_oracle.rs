//! Small host harness for the project's actual colour-field reference.
//! It verifies fixture expectations, not execution of the SMS assembly.

#![allow(dead_code)]

#[path = "../renderers/modern/src/control_frame.rs"]
mod control_frame;
#[path = "../renderers/modern/src/scenes/colour_field/state.rs"]
mod state;

use control_frame::{ControlFrame, Note};
use state::{ColourField, Config, Controls, PaletteSource};
use std::io::{self, Read, Write};

fn frame(step: u16, energy: u8, bands: [u8; 3], spectrum: [u8; 8]) -> ControlFrame {
    ControlFrame {
        frame: step,
        energy,
        bass: bands[0],
        mid: bands[1],
        treble: bands[2],
        spectrum,
        scene: 7,
        palette: 0,
        flags: 0,
        note_count: 0,
        notes: [Note::default(); 4],
        ..ControlFrame::procedural(0)
    }
}

fn local(p: u8) -> ColourField {
    ColourField::new(Config {
        palette: PaletteSource::Local { p, armed: false },
        ..Config::default()
    })
}

fn f18() {
    let mut field = local(2);
    for step in 0..600 {
        let mut input = frame(step, 100, [100; 3], [100; 8]);
        if step < 10 {
            input.flags = 1;
            input.note_count = 1;
            input.notes[0] = Note { note: 60, velocity: 100 };
        } else if step == 300 {
            input.flags = 1;
        }
        field.observe(&input);
        let trace = field.step(Controls::default()).unwrap();
        assert_eq!(trace.rot, u8::from(step >= 300), "F18 step {step}");
    }
}

fn f19() {
    let mut field = local(1);
    for step in 0..600 {
        let energy = if (320..380).contains(&step) { 200 } else { 64 };
        let mut input = frame(step, energy, [energy; 3], [0; 8]);
        input.flags = u8::from(step >= 50 && (step - 50) % 60 == 0);
        field.observe(&input);
        let trace = field.step(Controls::default()).unwrap();
        let level = match step {
            320..330 | 380..390 => 2,
            330..380 => 3,
            _ => 1,
        };
        assert_eq!(trace.lvl, [level, level, 0], "F19 step {step}");
    }
}

fn f20() {
    let mut field = local(1);
    for step in 0..600 {
        let band = if step < 200 || step >= 400 { 250 } else { 10 };
        let input = frame(step, 100, [band; 3], [0; 8]);
        field.observe(&input);
        let trace = field.step(Controls::default()).unwrap();
        let level = match step {
            200..210 | 410..420 => 2,
            210..220 | 400..410 => 1,
            220..400 => 0,
            _ => 3,
        };
        assert_eq!(trace.lvl, [level, level, 0], "F20 step {step}");
    }
}

fn f21() {
    let mut field = local(0);
    for step in 0..600 {
        let mut input = frame(step, 100, [100; 3], [100; 8]);
        input.scene = match step {
            0..100 | 300..320 | 450..600 => 6,
            100..200 if step % 2 != 0 => 6,
            _ => 7,
        };
        field.observe(&input);
        let trace = field.step(Controls::default());
        assert_eq!(trace.is_some(), (230..480).contains(&step), "F21 step {step}");
    }
}

fn f24() {
    let mut field = ColourField::new(Config::default());
    for step in 0..600 {
        let mut input = frame(step, 64, [64; 3], [0; 8]);
        input.palette = match step {
            0..10 => 0,
            10..20 => 1,
            20..30 | 130..600 => 2,
            _ => 3,
        };
        field.observe(&input);
        let controls = Controls {
            relative_palette: true,
            relative_scene: true,
            ..Controls::default()
        };
        let trace = field.step(controls).unwrap();
        assert_eq!(trace.mode, if step < 60 { 0 } else { 2 }, "F24 step {step}");
    }
}

fn f22() {
    let mut field = ColourField::new(Config::default());
    for step in 0..600 {
        let mut input = frame(step, 100, [100; 3], [100; 8]);
        input.scene = if step < 60 || step >= 300 {
            7
        } else if ((step - 60) / 20) % 2 == 0 {
            5
        } else {
            6
        };
        input.palette = if step < 60 { 0 } else { 3 };
        field.observe(&input);
        let trace = field.step(Controls::default()).unwrap();
        assert_eq!(trace.mode, if step < 330 { 0 } else { 2 }, "F22 step {step}");
    }
}

// Stream the exact mailbox fixture rows through the reference. The
// independently constructed inputs below also guard against fixture setup
// drift (including a wrong F18 mode), rather than validating only the trace.
fn stream_trace(name: &str) {
    let mut bytes = Vec::new();
    io::stdin().read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes.len(), 600 * 18, "{name}: expected 600 mailbox rows");
    let mut field = ColourField::new(Config::default());
    let mut output = Vec::with_capacity(600 * 9);
    for (step, row) in bytes.chunks_exact(18).enumerate() {
        let mut expected = match name {
            "f18" | "f21" | "f22" => frame(step as u16, 100, [100; 3], [100; 8]),
            "f19" => {
                let e = if (320..380).contains(&step) { 200 } else { 64 };
                frame(step as u16, e, [e; 3], [0; 8])
            }
            "f20" => {
                let v = if step < 200 || step >= 400 { 250 } else { 10 };
                frame(step as u16, 100, [v; 3], [0; 8])
            }
            "f24" => frame(step as u16, 64, [64; 3], [0; 8]),
            _ => panic!("unknown fixture {name}"),
        };
        match name {
            "f18" => {
                expected.palette = 2;
                if step < 10 {
                    expected.flags = 1;
                    expected.note_count = 1;
                    expected.notes[0] = Note { note: 60, velocity: 100 };
                } else if step == 300 {
                    expected.flags = 1;
                }
            }
            "f19" => {
                expected.palette = 1;
                expected.flags = u8::from(step >= 50 && (step - 50) % 60 == 0);
            }
            "f20" => expected.palette = 1,
            "f21" => {
                expected.scene = match step {
                    0..100 | 300..320 | 450..600 => 6,
                    100..200 if step % 2 != 0 => 6,
                    _ => 7,
                };
            }
            "f22" => {
                expected.scene = if step < 60 || step >= 300 {
                    7
                } else if ((step - 60) / 20) % 2 == 0 { 5 } else { 6 };
                expected.palette = if step < 60 { 0 } else { 3 };
            }
            "f24" => {
                expected.palette = match step {
                    0..10 => 0, 10..20 => 1, 20..30 | 130..600 => 2, _ => 3,
                };
            }
            _ => unreachable!(),
        }
        let payload = [
            expected.energy, expected.bass, expected.mid, expected.treble,
            expected.spectrum[0], expected.spectrum[1], expected.spectrum[2], expected.spectrum[3],
            expected.spectrum[4], expected.spectrum[5], expected.spectrum[6], expected.spectrum[7],
            expected.flags, expected.note_count, expected.notes[0].note, expected.notes[0].velocity,
        ];
        assert_eq!((row[0], row[1], &row[2..]), (expected.scene, expected.palette, &payload[..]),
                   "{name} input step {step}");
        field.observe(&expected);
        let relative = name == "f24";
        let trace = field.step(Controls {
            relative_palette: relative,
            relative_scene: relative,
            ..Controls::default()
        });
        if let Some(trace) = trace {
            if name == "f18" {
                assert_eq!(trace.mode, 2, "F18 must exercise hard cuts at step {step}");
                assert_eq!(trace.rot, u8::from(step >= 300), "F18 step {step}");
            }
            output.extend_from_slice(&[
                1, trace.mode, trace.family, trace.layout, trace.combo, trace.rot,
                trace.lvl[0], trace.lvl[1], trace.lvl[2],
            ]);
        } else {
            output.extend_from_slice(&[0; 9]);
        }
    }
    io::stdout().write_all(&output).unwrap();
}

fn pad_trace(detailed: bool) {
    let mut pad = Vec::new();
    io::stdin().read_to_end(&mut pad).unwrap();
    let mut previous = 0u8;
    let mut scene = 0u8;
    let mut palette = 0u8;
    let mut digital = false;
    let mut field = ColourField::new(Config::default());
    let mut output = Vec::with_capacity(pad.len() * if detailed { 27 } else { 9 });
    for (step, &buttons) in pad.iter().enumerate() {
        assert_eq!(buttons & !0x3f, 0);
        let edge = buttons & !previous;
        if edge & 3 != 0 && buttons & 3 != 3 { scene ^= 7; }
        if edge & 4 != 0 && buttons & 12 != 12 { palette = palette.wrapping_sub(1) & 7; }
        if edge & 8 != 0 && buttons & 12 != 12 { palette = (palette + 1) & 7; }
        previous = buttons;
        digital |= buttons & 0x30 != 0;
        let mut input = if digital {
            let mut input = frame(step as u16, 64, [64; 3], [0; 8]);
            input.flags = (buttons >> 4) & 3;
            input
        } else {
            // SMS uses the shared C generator's wrapping 16-bit counter and
            // full-width beat count for notes, unlike Rust's u8 note cast.
            let counter = step as u16;
            let mut input = ControlFrame::procedural(u64::from(counter));
            if input.note_count != 0 {
                input.notes[0].note = 48 + ((counter / 16) % 24) as u8;
            }
            input
        };
        input.scene = scene;
        input.palette = palette;
        if detailed {
            output.extend_from_slice(&[
                scene, palette, input.energy, input.bass, input.mid, input.treble,
                input.spectrum[0], input.spectrum[1], input.spectrum[2], input.spectrum[3],
                input.spectrum[4], input.spectrum[5], input.spectrum[6], input.spectrum[7],
                input.flags, input.note_count, input.notes[0].note, input.notes[0].velocity,
            ]);
        }
        field.observe(&input);
        if let Some(trace) = field.step(Controls {
            relative_palette: true,
            relative_scene: true,
            ..Controls::default()
        }) {
            output.extend_from_slice(&[
                1, trace.mode, trace.family, trace.layout, trace.combo, trace.rot,
                trace.lvl[0], trace.lvl[1], trace.lvl[2],
            ]);
        } else {
            output.extend_from_slice(&[0; 9]);
        }
    }
    io::stdout().write_all(&output).unwrap();
}

fn main() {
    assert!(!state::FLASH_BUILD, "unmeasured flash must remain disabled");
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--stream" {
        stream_trace(&args[2]);
        return;
    }
    if args.len() == 2 && args[1] == "--pad-stream" {
        pad_trace(false);
        return;
    }
    if args.len() == 2 && args[1] == "--pad-input-stream" {
        pad_trace(true);
        return;
    }
    assert_eq!(args.len(), 1, "usage: sms-shared-oracle [--stream NAME|--pad-stream|--pad-input-stream]");
    f18();
    f19();
    f20();
    f21();
    f22();
    f24();
    println!("sms shared oracle: F18/F19/F20/F21/F22/F24 passed");
}
