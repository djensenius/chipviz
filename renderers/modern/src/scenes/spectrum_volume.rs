//! SpectrumVolume: layered spectrum slices forming an audio volume.

use super::{CameraMode, SceneEntry, note_emphasis, palette};
use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scene::Instance;

pub const ENTRY: SceneEntry = SceneEntry {
    name: "SpectrumVolume",
    camera: CameraMode::Orbit3D,
    build,
};

fn build(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    instances: &mut Vec<Instance>,
) {
    let bands = frame.spectrum.len();
    let slices = profile.layer_count;
    let bars_per_slice = (profile.instance_count / slices.max(1)).max(bands);
    let energy = frame.energy as f32 / 255.0;
    let mids = frame.mid as f32 / 255.0;
    let notes = note_emphasis(frame);

    for slice in 0..slices {
        for bar in 0..bars_per_slice {
            if instances.len() >= profile.instance_count {
                return;
            }
            let band_index = bar % bands;
            let band = frame.spectrum[band_index] as f32 / 255.0;
            let x = (band_index as f32 / (bands - 1) as f32 - 0.5) * 9.5;
            let repeat = bar / bands;
            let y = (repeat as f32 / (bars_per_slice / bands).max(1) as f32 - 0.5) * 5.0;
            let z = (slice as f32 / slices.max(1) as f32 - 0.5) * 12.0;
            let pulse = (elapsed * 2.4 + slice as f32 * 0.13).sin() * 0.5 + 0.5;
            let height = 0.05 + band * (0.35 + energy * 0.55) + mids * notes * 0.25;

            instances.push(Instance {
                center: [x, y + band * 1.5, z],
                size: [0.08 + pulse * 0.03, height],
                color: palette(
                    frame.palette.wrapping_add(band_index as u8),
                    0.2 + band * 0.8,
                    energy,
                ),
                rotation: 0.0,
                scene_mix: 1.0,
                _padding: [0.0; 2],
            });
        }
    }
}
