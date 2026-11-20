//! PulseTunnel: beat-driven planes and bars moving through depth.

use super::{
    CameraMode, SceneBuild, SceneEntry, SceneState, TunnelVariant, lines, note_emphasis, palette,
    sin_cos,
};
use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scene::Instance;

pub const ENTRY: SceneEntry = SceneEntry {
    name: "PulseTunnel",
    camera: CameraMode::Orbit3D,
    build: SceneBuild::Stateless(build),
};

fn build(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    _aspect: f32,
    instances: &mut Vec<Instance>,
) {
    let rings = profile.layer_count.max(1);
    let bars_per_ring = (profile.instance_count / rings).max(12);
    let phase = frame.beat_phase as f32 / 255.0;
    let energy = frame.energy as f32 / 255.0;
    let bass = frame.bass as f32 / 255.0;
    let treble = frame.treble as f32 / 255.0;
    let notes = note_emphasis(frame);

    for ring in 0..rings {
        let z = (ring as f32 / rings as f32 - 0.5) * 16.0 + phase * 2.0;
        let radius = 1.0 + ring as f32 * 0.045 + energy * 1.5 + bass * 0.4 + notes * 0.5;

        for bar in 0..bars_per_ring {
            if instances.len() >= profile.instance_count {
                return;
            }
            let angle = bar as f32 / bars_per_ring as f32 * std::f32::consts::TAU;
            let band = frame.spectrum[bar % frame.spectrum.len()] as f32 / 255.0;
            let (sin, cos) = sin_cos(angle);
            let x = cos * radius * (2.4 + band);
            let y = sin * radius * (1.3 + band);

            instances.push(Instance {
                center: [x, y, z],
                size: [0.035 + energy * 0.03 + treble * 0.015, 0.22 + band * 0.65],
                color: palette(frame.palette, 0.18 + band * 0.82, energy),
                rotation: angle + elapsed * 0.12,
                scene_mix: 2.0,
                pattern: [0.0; 2],
            });
        }
    }
}

/// Slow audio envelopes; only fresh frames update them, using the renderer clock.
#[derive(Clone, Debug, Default)]
pub(crate) struct TunnelState {
    levels: [f32; 3],
    last_elapsed: Option<f64>,
}

impl TunnelState {
    pub(crate) fn observe_at(&mut self, frame: &ControlFrame, elapsed: f64) {
        let previous = self.last_elapsed.unwrap_or(elapsed);
        let delta = (elapsed - previous).max(0.0);
        let blend = (1.0 - (-delta / 0.8).exp()) as f32;
        for (level, input) in self
            .levels
            .iter_mut()
            .zip([frame.energy, frame.bass, frame.treble])
        {
            *level += (f32::from(input) / 255.0 - *level) * blend;
        }
        self.last_elapsed = Some(previous.max(elapsed));
    }

    #[cfg(test)]
    pub(crate) fn next_test_time(&self) -> f64 {
        self.last_elapsed.map_or(0.0, |time| time + 1.0 / 60.0)
    }
}

fn segment(
    instances: &mut Vec<Instance>,
    budget: usize,
    a: [f32; 2],
    b: [f32; 2],
    aspect: f32,
    color: [f32; 4],
) {
    if instances.len() >= budget {
        return;
    }
    let delta = [b[0] - a[0], b[1] - a[1]];
    let length = delta[0].hypot(delta[1]);
    if length <= f32::EPSILON {
        return;
    }
    let direction = [delta[0] / length, delta[1] / length];
    instances.push(Instance {
        center: [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, 0.0],
        size: [length, lines::ndc_width(direction, 0.004, aspect.max(0.01))],
        color,
        rotation: delta[1].atan2(delta[0]),
        scene_mix: 2.0,
        pattern: [0.0; 2],
    });
}

pub(crate) fn build_variant(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    aspect: f32,
    state: &SceneState,
    instances: &mut Vec<Instance>,
) {
    let [energy, bass, treble] = state.tunnel.levels;
    // Thin, dim lines only: no beat-triggered cuts or full-screen flashes.
    let base = palette(frame.palette, 0.42, 0.5);
    let mut draw = |a, b, fade: f32| {
        let mut color = base;
        color[3] = fade.clamp(0.0, 1.0);
        segment(instances, profile.instance_count, a, b, aspect, color);
    };
    match state.config().tunnel_variant {
        TunnelVariant::Hallway => {
            let focal = [0.10 * bass * (elapsed * 0.19).sin(), 0.06 * energy];
            let corners = [[-1.1, -0.85], [1.1, -0.85], [1.1, 0.85], [-1.1, 0.85]];
            for corner in corners {
                draw(focal, corner, 0.55);
            }
            // Equal depth intervals are projected to nested rectangles; fade
            // at both ends hides the cyclic depth wrap.
            for ring in 0..20 {
                let depth = (ring as f32 / 20.0 + elapsed * 0.04).fract();
                let scale = 0.08 + depth * depth * (1.05 + 0.1 * energy);
                let fade = (depth * 8.0).min((1.0 - depth) * 8.0).clamp(0.0, 1.0);
                let point = |corner: [f32; 2]| {
                    [
                        focal[0] + (corner[0] - focal[0]) * scale,
                        focal[1] + (corner[1] - focal[1]) * scale,
                    ]
                };
                for side in 0..4 {
                    draw(point(corners[side]), point(corners[(side + 1) % 4]), fade);
                }
            }
        }
        TunnelVariant::Abstract => {
            // Independent radial dashes spiral through depth, rather than a
            // folded plane or the hallway's rectangular cross-sections.
            for spoke in 0..20 {
                let angle = spoke as f32 * std::f32::consts::TAU / 20.0
                    + 0.25 * (elapsed * 0.12).sin()
                    + bass * 0.22;
                let (s, c) = sin_cos(angle);
                for dash in 0..4 {
                    let depth = (dash as f32 / 4.0 + elapsed * 0.03 + spoke as f32 * 0.013).fract();
                    let radius = 0.15 + depth * (1.0 + 0.15 * energy);
                    let length = 0.12 + treble * 0.12;
                    let tangent = 0.08 + depth * 0.20;
                    let point = |r: f32, t: f32| [(c * r - s * t) / aspect.max(0.5), s * r + c * t];
                    let fade = (depth * 8.0).min((1.0 - depth) * 8.0).clamp(0.0, 1.0);
                    draw(
                        point(radius, tangent),
                        point(radius + length, tangent + 0.08),
                        fade,
                    );
                }
            }
        }
        TunnelVariant::Original => unreachable!("original uses its unchanged builder"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_envelope_is_time_based_and_ignores_backward_timestamps() {
        let frame = ControlFrame {
            energy: 255,
            bass: 255,
            treble: 255,
            ..ControlFrame::procedural(0)
        };
        let mut sixty = TunnelState::default();
        let mut thirty = TunnelState::default();
        for i in 0..=60 {
            sixty.observe_at(&frame, f64::from(i) / 60.0);
        }
        for i in 0..=30 {
            thirty.observe_at(&frame, f64::from(i) / 30.0);
        }
        assert!((sixty.levels[0] - thirty.levels[0]).abs() < 0.00001);
        let before = sixty.levels;
        sixty.observe_at(
            &ControlFrame {
                energy: 0,
                bass: 0,
                treble: 0,
                ..frame
            },
            0.5,
        );
        assert_eq!(sixty.levels, before);
        sixty.observe_at(
            &ControlFrame {
                energy: 0,
                bass: 0,
                treble: 0,
                ..frame
            },
            1.0 + 1.0 / 60.0,
        );
        assert!((before[0] - sixty.levels[0]) < 0.021);
    }
}
