//! SignalField: thousands of thin high-contrast data planes in a rotating 3D field.

use super::{CameraMode, SceneEntry, hash, note_emphasis, palette};
use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scene::Instance;

pub const ENTRY: SceneEntry = SceneEntry {
    name: "SignalField",
    camera: CameraMode::Orbit3D,
    build,
};

fn build(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    _aspect: f32,
    instances: &mut Vec<Instance>,
) {
    let columns = 128usize;
    let rows = (profile.instance_count / columns).max(1);
    let layers = profile.layer_count.max(1);
    let energy = frame.energy as f32 / 255.0;
    let low = frame.bass as f32 / 255.0;
    let high = frame.treble as f32 / 255.0;
    let notes = note_emphasis(frame);

    for index in 0..profile.instance_count {
        let column = index % columns;
        let row = (index / columns) % rows;
        let layer = index % layers;
        let band = frame.spectrum[column % frame.spectrum.len()] as f32 / 255.0;
        let gate = hash(index as u32 + frame.frame as u32) > 0.28 + band * 0.5;
        let brightness = if gate { 0.92 } else { 0.04 + band * 0.16 };
        let x = (column as f32 / columns as f32 - 0.5) * 12.0;
        let y = (row as f32 / rows as f32 - 0.5) * 6.5;
        let z = (layer as f32 / layers as f32 - 0.5) * (9.0 + notes * 2.0)
            + (elapsed * (0.6 + energy + low)).sin() * 0.35;
        let thin = 0.012 + band * 0.015 + high * 0.006;
        let tall = 0.06 + band * 0.34 + notes * 0.08;

        instances.push(Instance {
            center: [x, y, z],
            size: [thin, tall],
            color: palette(frame.palette, brightness, energy),
            rotation: 0.0,
            scene_mix: 0.0,
            _padding: [0.0; 2],
        });
    }
}
