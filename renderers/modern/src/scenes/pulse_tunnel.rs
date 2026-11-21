//! PulseTunnel: beat-driven planes and bars moving through depth.

use super::{
    CameraMode, SceneBuild, SceneEntry, SceneState, TunnelVariant, house, lines, note_emphasis,
    palette, planar_hallway, sin_cos,
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

/// Slow audio and projected-look colour envelopes, advanced once per draw or
/// capture replay step. Existing tunnel builders continue to use wire colour.
#[derive(Clone, Debug, Default)]
pub(crate) struct TunnelState {
    levels: [f32; 3],
    projected_color: Option<[f32; 4]>,
    last_elapsed: Option<f64>,
}

impl TunnelState {
    pub(crate) fn advance(&mut self, frame: &ControlFrame, elapsed: f64) {
        let previous = self.last_elapsed.unwrap_or(elapsed);
        // Discard excess time after a stall: applying it in one draw could
        // otherwise jump the geometry straight to a new audio level.
        let delta = (elapsed - previous).clamp(0.0, 1.0 / 30.0);
        let blend = (1.0 - (-delta / 0.8).exp()) as f32;
        for (level, input) in self
            .levels
            .iter_mut()
            .zip([frame.energy, frame.bass, frame.treble])
        {
            *level += (f32::from(input) / 255.0 - *level) * blend;
        }
        let target = palette(frame.palette, 0.42, 0.5);
        let color = self.projected_color.get_or_insert(target);
        for (channel, target) in color[..3].iter_mut().zip(target) {
            *channel += (target - *channel) * blend;
        }
        self.last_elapsed = Some(previous.max(elapsed));
    }
}

fn segment(
    instances: &mut Vec<Instance>,
    budget: usize,
    a: [f32; 2],
    b: [f32; 2],
    aspect: f32,
    width: f32,
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
        size: [length, lines::ndc_width(direction, width, aspect.max(0.01))],
        color,
        rotation: delta[1].atan2(delta[0]),
        scene_mix: 2.0,
        pattern: [0.0; 2],
    });
}

pub(crate) fn build_projected(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    aspect: f32,
    state: &SceneState,
    instances: &mut Vec<Instance>,
    is_house: bool,
) {
    let remaining = profile.instance_count.saturating_sub(instances.len());
    let mut draw = |start, end, width, intensity: f32| {
        let mut color = state
            .tunnel
            .projected_color
            .unwrap_or_else(|| palette(frame.palette, 0.42, 0.5));
        color[3] = intensity.clamp(0.0, 1.0);
        segment(
            instances,
            profile.instance_count,
            start,
            end,
            aspect,
            width,
            color,
        );
    };
    if is_house {
        for line in house::project(elapsed, aspect, state.tunnel.levels, remaining)
            .into_iter()
            .take(remaining)
        {
            draw(line.start, line.end, line.width, line.intensity);
        }
    } else {
        for line in planar_hallway::project(elapsed, aspect, state.tunnel.levels, remaining)
            .into_iter()
            .take(remaining)
        {
            draw(line.start, line.end, line.width, line.intensity);
        }
    }
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
        segment(
            instances,
            profile.instance_count,
            a,
            b,
            aspect,
            0.004,
            color,
        );
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
        TunnelVariant::Original | TunnelVariant::SurfaceHallway => {
            unreachable!("uses a separate builder")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::build_instances;
    use crate::scenes::SceneConfig;

    fn audio(level: u8) -> ControlFrame {
        ControlFrame {
            scene: 2,
            energy: level,
            bass: level,
            treble: level,
            ..ControlFrame::procedural(0)
        }
    }

    fn state() -> SceneState {
        SceneState::new(SceneConfig {
            tunnel_variant: TunnelVariant::Hallway,
            ..SceneConfig::default()
        })
    }

    #[test]
    fn projected_palette_changes_smoothly_on_live_and_replay_steps() {
        let profile = RenderProfile::pi5();
        for house in [false, true] {
            let mut live = SceneState::new(SceneConfig {
                tunnel_variant: if house {
                    TunnelVariant::Original
                } else {
                    TunnelVariant::SurfaceHallway
                },
                ..SceneConfig::default()
            });
            live.set_local_house(house);
            let mut replay = live.clone();
            let mut previous: Option<[f32; 4]> = None;
            for i in 0..=420 {
                let frame = ControlFrame {
                    scene: 2,
                    palette: if i < 120 { (i % 2) as u8 * 3 } else { 2 },
                    ..audio(180)
                };
                let time = f64::from(i) / 60.0;
                // Fresh observations do not advance the envelope; live and
                // replay each advance once on the displayed frame clock.
                replay.observe_at(&frame, time);
                live.observe_live_at(&frame, time);
                replay.advance_tunnel(&frame, time);
                live.advance_tunnel(&frame, time);
                let color = live.tunnel.projected_color.unwrap();
                assert_eq!(color, replay.tunnel.projected_color.unwrap());
                if let Some(previous) = previous {
                    for channel in 0..3 {
                        assert!((color[channel] - previous[channel]).abs() < 0.01);
                    }
                }
                previous = Some(color);
                if i % 60 == 0 {
                    let live_instances =
                        build_instances(&profile, frame, time as f32, 16.0 / 9.0, &live);
                    let replay_instances =
                        build_instances(&profile, frame, time as f32, 16.0 / 9.0, &replay);
                    assert!(!live_instances.is_empty());
                    assert_eq!(
                        bytemuck::cast_slice::<Instance, u8>(&live_instances),
                        bytemuck::cast_slice::<Instance, u8>(&replay_instances)
                    );
                    assert_eq!(&live_instances[0].color[..3], &color[..3]);
                }
            }
            let target = palette(2, 0.42, 0.5);
            let before = live.tunnel.projected_color.unwrap();
            for channel in 0..3 {
                assert!((before[channel] - target[channel]).abs() < 0.001);
            }
            let changed = ControlFrame {
                palette: 0,
                ..audio(180)
            };
            live.advance_tunnel(&changed, 67.0);
            let after_stall = live.tunnel.projected_color.unwrap();
            for channel in 0..3 {
                assert!((after_stall[channel] - before[channel]).abs() < 0.02);
            }
            live.advance_tunnel(&changed, 66.0);
            assert_eq!(live.tunnel.projected_color.unwrap(), after_stall);
        }
    }

    #[test]
    fn existing_flat_tunnels_keep_immediate_palette_changes() {
        let profile = RenderProfile::pi5();
        for variant in [TunnelVariant::Hallway, TunnelVariant::Abstract] {
            let mut state = SceneState::new(SceneConfig {
                tunnel_variant: variant,
                ..SceneConfig::default()
            });
            let first = ControlFrame {
                palette: 0,
                ..audio(180)
            };
            let changed = ControlFrame {
                palette: 3,
                ..first
            };
            state.advance_tunnel(&first, 0.0);
            state.advance_tunnel(&changed, 1.0 / 60.0);
            let instances = build_instances(&profile, changed, 1.0, 16.0 / 9.0, &state);
            assert!(!instances.is_empty());
            assert_eq!(&instances[0].color[..3], &palette(3, 0.42, 0.5)[..3]);
        }
    }

    #[test]
    fn single_packet_converges_on_held_redraws() {
        let frame = audio(255);
        let mut state = state();
        state.observe_at(&frame, 0.0);
        state.advance_tunnel(&frame, 0.0);
        assert_eq!(state.tunnel.levels, [0.0; 3]);
        let mut previous = 0.0;
        // No more observations: a UDP packet is held on each redraw.
        for i in 1..=240 {
            state.advance_tunnel(&frame, f64::from(i) / 60.0);
            let level = state.tunnel.levels[0];
            assert!(level > previous && level <= 1.0);
            assert!(level - previous < 0.021);
            previous = level;
        }
        assert!(previous > 0.99);
    }

    #[test]
    fn batched_packets_use_displayed_newest_input_once() {
        let quiet = audio(0);
        let loud = audio(255);
        let mut state = state();
        state.observe_at(&quiet, 0.0);
        state.advance_tunnel(&quiet, 0.0);

        // Both packet observations share a timestamp; neither may consume
        // the interval before the newest displayed frame is known.
        state.observe_at(&quiet, 1.0 / 60.0);
        state.observe_at(&loud, 1.0 / 60.0);
        assert_eq!(state.tunnel.levels, [0.0; 3]);
        state.advance_tunnel(&loud, 1.0 / 60.0);
        let expected = (1.0 - (-1.0_f64 / 60.0 / 0.8).exp()) as f32;
        assert!((state.tunnel.levels[0] - expected).abs() < 0.000001);
        let before = state.tunnel.levels;
        // An identical timestamp cannot double-smooth, even if the frame
        // changes. The next redraw uses the newest held input.
        state.advance_tunnel(&quiet, 1.0 / 60.0);
        assert_eq!(state.tunnel.levels, before);
        state.advance_tunnel(&quiet, 2.0 / 60.0);
        assert!(state.tunnel.levels[0] < before[0]);
    }

    #[test]
    fn long_gap_is_bounded_and_capture_replay_matches_redraw() {
        let loud = audio(255);
        let mut stalled = state();
        stalled.advance_tunnel(&loud, 0.0);
        stalled.advance_tunnel(&loud, 60.0);
        let maximum = (1.0 - (-1.0_f64 / 30.0 / 0.8).exp()) as f32;
        assert!((stalled.tunnel.levels[0] - maximum).abs() < 0.000001);
        let before = stalled.tunnel.levels;
        stalled.advance_tunnel(&audio(0), 59.0);
        stalled.advance_tunnel(&audio(0), 60.0);
        assert_eq!(stalled.tunnel.levels, before);
        stalled.advance_tunnel(&loud, 60.0 + 1.0 / 60.0);
        assert!(stalled.tunnel.levels[0] > before[0]);

        let mut replay = state();
        let mut live = state();
        for i in 0..180 {
            let frame = if i < 80 { loud } else { audio(0) };
            let time = f64::from(i) / 60.0;
            // Capture observes every frame; a live held packet observes only
            // once per change. Both advance once per displayed/replayed step.
            replay.observe_at(&frame, time);
            if i == 0 || i == 80 {
                live.observe_at(&frame, time);
            }
            replay.advance_tunnel(&frame, time);
            live.advance_tunnel(&frame, time);
            assert_eq!(replay.tunnel.levels, live.tunnel.levels);
        }
        let profile = RenderProfile::pi5();
        let frame = audio(0);
        let a = build_instances(&profile, frame, 3.0, 16.0 / 9.0, &replay);
        let b = build_instances(&profile, frame, 3.0, 16.0 / 9.0, &live);
        assert_eq!(
            bytemuck::cast_slice::<Instance, u8>(&a),
            bytemuck::cast_slice::<Instance, u8>(&b)
        );
    }

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
            sixty.advance(&frame, f64::from(i) / 60.0);
        }
        for i in 0..=30 {
            thirty.advance(&frame, f64::from(i) / 30.0);
        }
        assert!((sixty.levels[0] - thirty.levels[0]).abs() < 0.00001);
        let before = sixty.levels;
        sixty.advance(
            &ControlFrame {
                energy: 0,
                bass: 0,
                treble: 0,
                ..frame
            },
            0.5,
        );
        assert_eq!(sixty.levels, before);
        sixty.advance(
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
