use bytemuck::{Pod, Zeroable};

use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
}

impl Vertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Instance {
    pub center: [f32; 3],
    pub size: [f32; 2],
    pub color: [f32; 4],
    pub rotation: f32,
    pub scene_mix: f32,
    pub _padding: [f32; 2],
}

impl Instance {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
      1 => Float32x3,
      2 => Float32x2,
      3 => Float32x4,
      4 => Float32,
      5 => Float32,
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Uniforms {
    pub view_projection: [[f32; 4]; 4],
    pub time: f32,
    pub energy: f32,
    pub beat: f32,
    pub _padding: f32,
}

pub const QUAD_VERTICES: [Vertex; 6] = [
    Vertex {
        position: [-0.5, -0.5],
    },
    Vertex {
        position: [0.5, -0.5],
    },
    Vertex {
        position: [0.5, 0.5],
    },
    Vertex {
        position: [-0.5, -0.5],
    },
    Vertex {
        position: [0.5, 0.5],
    },
    Vertex {
        position: [-0.5, 0.5],
    },
];

pub fn build_instances(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
) -> Vec<Instance> {
    let mut instances = Vec::with_capacity(profile.instance_count);
    let scene = frame.scene % 3;

    match scene {
        0 => signal_field(profile, frame, elapsed, &mut instances),
        1 => spectrum_volume(profile, frame, elapsed, &mut instances),
        _ => pulse_tunnel(profile, frame, elapsed, &mut instances),
    }

    instances
}

#[derive(Default)]
pub struct Camera {
    energy: f32,
    drift: f32,
    last_elapsed: Option<f32>,
}

impl Camera {
    pub fn build_uniforms(
        &mut self,
        profile: &RenderProfile,
        frame: ControlFrame,
        elapsed: f32,
        aspect: f32,
    ) -> Uniforms {
        let energy = frame.energy as f32 / 255.0;
        let beat = if frame.beat_onset() { 1.0 } else { 0.0 };
        let previous = self.last_elapsed.unwrap_or(elapsed);
        if self.last_elapsed.is_none() {
            // Seed energy from the first rendered frame; angular drift starts neutral.
            self.energy = energy;
        } else {
            // A 0.5 s exponential response stays within [0, 1] without overshoot.
            // Equal/backward timestamps cannot advance or rewind this envelope.
            let delta = (elapsed - previous).max(0.0);
            let blend = -(-delta / 0.5).exp_m1();
            self.energy += (energy - self.energy) * blend;
            let drift = (frame.beat_count as f32 * 0.015).sin() * (frame.bpm as f32 / 255.0) * 0.04;
            self.drift += (drift - self.drift) * blend;
        }
        self.last_elapsed = Some(previous.max(elapsed));

        // Elapsed time drives the orbit. Drift is at most +/-0.04 rad and changes
        // at most 0.16 rad/s, below either profile's base speed. Phase is not an
        // angle; counter wraps and coupled energy resets are both smoothed.
        let angle = elapsed * profile.camera_speed + self.drift;
        let radius = profile.camera_radius - self.energy * 1.8;
        let eye = [
            angle.cos() * radius,
            1.8 + self.energy * 2.2,
            angle.sin() * radius,
        ];
        let view = look_at(eye, [0.0; 3], [0.0, 1.0, 0.0]);
        let projection = perspective(52.0_f32.to_radians(), aspect, 0.1, 100.0);

        Uniforms {
            // Row-indexed multiplication of column-major storage reverses the order:
            // this uploads projection * view as columns for WGSL.
            view_projection: multiply4(view, projection),
            time: elapsed,
            energy,
            beat,
            _padding: 0.0,
        }
    }
}

fn signal_field(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
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

fn spectrum_volume(
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

fn pulse_tunnel(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
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
            let x = angle.cos() * radius * (2.4 + band);
            let y = angle.sin() * radius * (1.3 + band);

            instances.push(Instance {
                center: [x, y, z],
                size: [0.035 + energy * 0.03 + treble * 0.015, 0.22 + band * 0.65],
                color: palette(frame.palette, 0.18 + band * 0.82, energy),
                rotation: angle + elapsed * 0.12,
                scene_mix: 2.0,
                _padding: [0.0; 2],
            });
        }
    }
}

fn palette(index: u8, brightness: f32, energy: f32) -> [f32; 4] {
    let brightness = brightness.clamp(0.0, 1.0);
    match index % 4 {
        0 => [brightness, brightness, brightness, 1.0],
        1 => [brightness * 0.7, brightness * 0.9, brightness, 1.0],
        2 => [
            brightness,
            brightness * (0.55 + energy * 0.4),
            brightness * 0.25,
            1.0,
        ],
        _ => [
            brightness * 0.35,
            brightness,
            brightness * (0.7 + energy * 0.3),
            1.0,
        ],
    }
}

fn note_emphasis(frame: ControlFrame) -> f32 {
    let count = usize::from(frame.note_count).min(frame.notes.len());
    if count == 0 {
        return 0.0;
    }

    let sum = frame.notes[..count].iter().fold(0.0, |accumulator, note| {
        let pitch = f32::from(note.note.min(127)) / 127.0;
        let velocity = f32::from(note.velocity) / 255.0;
        accumulator + pitch * velocity
    });

    (sum / count as f32).clamp(0.0, 1.0)
}

fn hash(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value as f32 / u32::MAX as f32
}

fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fovy / 2.0).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, (far * near) / (near - far), 0.0],
    ]
}

fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let forward = normalize([target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]]);
    let side = normalize(cross(forward, up));
    let up = cross(side, forward);

    [
        [side[0], up[0], -forward[0], 0.0],
        [side[1], up[1], -forward[1], 0.0],
        [side[2], up[2], -forward[2], 0.0],
        [-dot(side, eye), -dot(up, eye), dot(forward, eye), 1.0],
    ]
}

fn multiply4(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for row in 0..4 {
        for column in 0..4 {
            out[row][column] = left[row][0] * right[0][column]
                + left[row][1] * right[1][column]
                + left[row][2] * right[2][column]
                + left[row][3] * right[3][column];
        }
    }
    out
}

fn normalize(value: [f32; 3]) -> [f32; 3] {
    let length = dot(value, value).sqrt().max(f32::EPSILON);
    [value[0] / length, value[1] / length, value[2] / length]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projected_position(uniforms: &Uniforms, world: [f32; 3]) -> [f32; 2] {
        let columns = uniforms.view_projection;
        let clip: [f32; 4] = std::array::from_fn(|row| {
            columns[0][row] * world[0]
                + columns[1][row] * world[1]
                + columns[2][row] * world[2]
                + columns[3][row]
        });
        assert!(clip.iter().all(|value| value.is_finite()));
        assert!(clip[3] > 0.0 && (0.0..=clip[3]).contains(&clip[2]));
        [clip[0] / clip[3], clip[1] / clip[3]]
    }

    fn projected_motion(before: &Uniforms, after: &Uniforms) -> f32 {
        // Fixed world landmarks isolate the camera from animated instances.
        [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, -0.5, 2.0],
        ]
        .into_iter()
        .map(|world| {
            let before = projected_position(before, world);
            let after = projected_position(after, world);
            (after[0] - before[0]).hypot(after[1] - before[1])
        })
        .fold(0.0, f32::max)
    }

    #[test]
    fn camera_equal_elapsed_has_no_phase_or_initial_counter_jump() {
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            let frame = ControlFrame::procedural(0);
            let reference = Camera::default().build_uniforms(&profile, frame, 3.0, 16.0 / 9.0);
            for beat_phase in [0, 17, 32, 128, 239, 255] {
                for beat_count in [0, 37, u16::MAX] {
                    let frame = ControlFrame {
                        beat_phase,
                        beat_count,
                        ..frame
                    };
                    let mut camera = Camera::default();
                    let actual = camera.build_uniforms(&profile, frame, 3.0, 16.0 / 9.0);
                    let motion = projected_motion(&reference, &actual);
                    assert!(
                        motion < 1.0e-6,
                        "{} phase {beat_phase}, count {beat_count}: {motion} NDC",
                        profile.name
                    );
                    // Phase also has no influence after the envelope has advanced.
                    let unphased = ControlFrame {
                        beat_phase: 0,
                        ..frame
                    };
                    let mut reference = Camera::default();
                    reference.build_uniforms(&profile, unphased, 3.0, 16.0 / 9.0);
                    let expected = reference.build_uniforms(&profile, unphased, 3.1, 16.0 / 9.0);
                    let actual = camera.build_uniforms(&profile, frame, 3.1, 16.0 / 9.0);
                    assert!(projected_motion(&expected, &actual) < 1.0e-6);
                }
            }
        }
    }

    fn assert_camera_sequence(phase_step: u64) {
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            for aspect in [16.0 / 9.0, 9.0 / 16.0] {
                let mut camera = Camera::default();
                let mut previous = None;
                let mut peak_motion = 0.0_f32;
                for index in 0..160 {
                    let phase = (index * phase_step) as u8;
                    let frame = ControlFrame {
                        beat_phase: phase,
                        energy: 80 + phase / 2,
                        beat_count: (u16::MAX - 5).wrapping_add((index / 16) as u16),
                        ..ControlFrame::procedural(index)
                    };
                    let uniforms =
                        camera.build_uniforms(&profile, frame, index as f32 / 60.0, aspect);
                    if let Some(previous) = previous {
                        peak_motion = peak_motion.max(projected_motion(&previous, &uniforms));
                    }
                    previous = Some(uniforms);
                }
                // At 60 Hz no landmark may jump more than 1% of the viewport,
                // including phase/energy resets and the u16 beat-counter wrap.
                assert!(
                    peak_motion < 0.02,
                    "{} step {phase_step}, aspect {aspect}: {peak_motion} NDC",
                    profile.name
                );
            }
        }
    }

    #[test]
    fn camera_procedural_phase_steps_and_wraps_are_readable() {
        assert_camera_sequence(17);
    }

    #[test]
    fn camera_fixture_phase_steps_and_wraps_are_readable() {
        assert_camera_sequence(32);
    }

    #[test]
    fn camera_energy_response_is_smooth_bounded_and_nonzero() {
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            let quiet = ControlFrame {
                energy: 0,
                ..ControlFrame::procedural(0)
            };
            let loud = ControlFrame {
                energy: 255,
                ..quiet
            };
            let aspect = 16.0 / 9.0;
            let mut camera = Camera::default();
            let _initial = camera.build_uniforms(&profile, quiet, 0.0, aspect);
            let response = camera.build_uniforms(&profile, loud, 1.0 / 60.0, aspect);
            let unmodulated = Camera::default().build_uniforms(&profile, quiet, 1.0 / 60.0, aspect);
            let motion = projected_motion(&unmodulated, &response);
            assert!(motion < 0.02, "{} energy step: {motion} NDC", profile.name);
            assert_eq!(response.time, 1.0 / 60.0);
            assert_eq!(response.energy, 1.0);
            assert_eq!(response.beat, 1.0);

            let settled = camera.build_uniforms(&profile, loud, 5.0, aspect);
            let unmodulated = Camera::default().build_uniforms(&profile, quiet, 5.0, aspect);
            let fully_modulated = Camera::default().build_uniforms(&profile, loud, 5.0, aspect);
            let response = projected_motion(&unmodulated, &settled);
            assert!((0.03..0.35).contains(&response), "{response} NDC");
            assert!(projected_motion(&fully_modulated, &settled) < 2.0e-5);
        }
    }

    #[test]
    fn camera_beat_drift_stays_bounded_through_counter_wrap() {
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            let frame = ControlFrame {
                bpm: 255,
                ..ControlFrame::procedural(0)
            };
            let mut camera = Camera::default();
            let mut peak_response = 0.0_f32;
            for index in 0..600 {
                let elapsed = index as f32 / 60.0;
                let modulated = ControlFrame {
                    beat_count: [105, 314, u16::MAX, 0, 157][index / 120],
                    ..frame
                };
                let reference =
                    Camera::default().build_uniforms(&profile, frame, elapsed, 16.0 / 9.0);
                let actual = camera.build_uniforms(&profile, modulated, elapsed, 16.0 / 9.0);
                peak_response = peak_response.max(projected_motion(&reference, &actual));
            }
            // Even extreme tempo/counter changes move landmarks by less than
            // 2% of the viewport relative to the elapsed-only orbit.
            assert!(
                (0.001..0.04).contains(&peak_response),
                "{peak_response} NDC"
            );
        }
    }

    #[test]
    fn camera_response_is_independent_of_render_interval() {
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            let quiet = ControlFrame {
                energy: 0,
                ..ControlFrame::procedural(0)
            };
            let loud = ControlFrame {
                energy: 255,
                bpm: 255,
                beat_count: 105,
                ..quiet
            };
            let mut reference = None;
            for rate in [30, 60, 144] {
                let mut camera = Camera::default();
                let mut actual = camera.build_uniforms(&profile, quiet, 0.0, 16.0 / 9.0);
                for index in 1..=2 * rate {
                    let elapsed = index as f32 / rate as f32;
                    actual = camera.build_uniforms(&profile, loud, elapsed, 16.0 / 9.0);
                    if index == 1 {
                        let unmodulated =
                            Camera::default().build_uniforms(&profile, quiet, elapsed, 16.0 / 9.0);
                        assert!(projected_motion(&unmodulated, &actual) < 0.02);
                    }
                }
                if let Some(reference) = &reference {
                    assert!(projected_motion(reference, &actual) < 2.0e-6);
                }
                reference = Some(actual);
            }
        }
    }

    #[test]
    fn camera_initial_and_repeated_times_do_not_integrate_spurious_motion() {
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            let frame = ControlFrame {
                energy: 255,
                beat_count: u16::MAX,
                beat_phase: 255,
                ..ControlFrame::procedural(0)
            };
            let elapsed = std::f32::consts::FRAC_PI_2 / profile.camera_speed;
            let mut camera = Camera::default();
            let initial = camera.build_uniforms(&profile, frame, elapsed, 16.0 / 9.0);
            assert_projection(
                &initial,
                [0.0; 3],
                [0.0, 0.0, (profile.camera_radius - 1.8).hypot(4.0)],
                16.0 / 9.0,
            );
            let reset = ControlFrame {
                energy: 0,
                bpm: 0,
                beat_count: 0,
                beat_phase: 0,
                flags: 0,
                ..frame
            };
            for time in [elapsed, elapsed - 1.0, elapsed, elapsed] {
                let actual = camera.build_uniforms(&profile, reset, time, 16.0 / 9.0);
                if time == elapsed {
                    assert_eq!(initial.view_projection, actual.view_projection);
                }
                assert_eq!(actual.time, time);
                assert_eq!(actual.energy, 0.0);
                assert_eq!(actual.beat, 0.0);
            }
            let next_time = elapsed + 1.0 / 60.0;
            let next = camera.build_uniforms(&profile, reset, next_time, 16.0 / 9.0);
            let held = Camera::default().build_uniforms(&profile, frame, next_time, 16.0 / 9.0);
            let motion = projected_motion(&held, &next);
            assert!((1.0e-6..0.02).contains(&motion), "{motion} NDC");
        }
    }

    fn assert_projection(uniforms: &Uniforms, world: [f32; 3], camera: [f32; 3], aspect: f32) {
        // Interpret the uploaded columns as WGSL's matrix * world position.
        let columns = uniforms.view_projection;
        let clip: [f32; 4] = std::array::from_fn(|row| {
            columns[0][row] * world[0]
                + columns[1][row] * world[1]
                + columns[2][row] * world[2]
                + columns[3][row]
        });
        // Scalar pinhole projection: positive depth is ahead of the camera,
        // and wgpu maps the near/far planes to NDC depths 0/1.
        let [horizontal, vertical, depth] = camera;
        let half_fov_tan = 26.0_f32.to_radians().tan();
        let expected = [
            horizontal / (half_fov_tan * aspect),
            vertical / half_fov_tan,
            (depth - 0.1) * 100.0 / (100.0 - 0.1),
            depth,
        ];
        for (component, (actual, expected)) in clip.into_iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() < 2.0e-5 * expected.abs().max(1.0),
                "world {world:?}, clip component {component}: got {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn uniforms_project_origin_in_front_of_camera() {
        let frame = ControlFrame {
            energy: 0,
            ..ControlFrame::procedural(0)
        };
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            let aspect = profile.target_width as f32 / profile.target_height as f32;
            let uniforms = Camera::default().build_uniforms(&profile, frame, 0.0, aspect);
            // At zero time/energy the eye is (radius, 1.8, 0),
            // looking directly at the origin.
            let distance = profile.camera_radius.hypot(1.8);
            assert_projection(&uniforms, [0.0; 3], [0.0, 0.0, distance], aspect);
        }
    }

    #[test]
    fn uniforms_project_off_axis_points_with_camera_basis() {
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            for energy in [0, 255] {
                let frame = ControlFrame {
                    energy,
                    ..ControlFrame::procedural(0)
                };
                let radius = profile.camera_radius - f32::from(energy) / 255.0 * 1.8;
                let height = 1.8 + f32::from(energy) / 255.0 * 2.2;
                let distance = radius.hypot(height);
                for aspect in [16.0 / 9.0, 9.0 / 16.0] {
                    let uniforms = Camera::default().build_uniforms(&profile, frame, 0.0, aspect);
                    for [x, y, z] in [
                        [0.0, 0.0, -1.0],
                        [0.0, 0.0, 1.0],
                        [0.0, 1.0, 0.0],
                        [1.0, -0.5, 2.0],
                    ] {
                        // Looking from +X: right is -Z, up is (-height, radius, 0)
                        // normalized, and forward is (-radius, -height, 0) normalized.
                        let camera = [
                            -z,
                            (radius * y - height * x) / distance,
                            distance - (radius * x + height * y) / distance,
                        ];
                        assert_projection(&uniforms, [x, y, z], camera, aspect);
                    }
                }
            }
        }
    }

    #[test]
    fn uniforms_project_orbiting_camera_depth() {
        let frame = ControlFrame {
            energy: 255,
            bpm: 180,
            beat_phase: 32,
            beat_count: 37,
            ..ControlFrame::procedural(0)
        };
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            // Choose elapsed time so the orbit is a quarter turn: eye = (0, 4, radius).
            let elapsed = std::f32::consts::FRAC_PI_2 / profile.camera_speed;
            let radius = profile.camera_radius - 1.8;
            let height = 4.0;
            let distance = radius.hypot(height);
            let aspect = 4.0 / 3.0;
            let uniforms = Camera::default().build_uniforms(&profile, frame, elapsed, aspect);
            assert_projection(&uniforms, [0.0; 3], [0.0, 0.0, distance], aspect);
            for depth in [0.1, 2.0, 100.0, -1.0] {
                // Right is +X; up = (0, radius, -height) / distance;
                // forward = (0, -height, -radius) / distance.
                // Place an off-axis point at near, interior, far, and behind-eye depths.
                let horizontal = 0.02;
                let vertical = -0.03;
                let world = [
                    horizontal,
                    4.0 + (vertical * radius - depth * height) / distance,
                    radius - (vertical * height + depth * radius) / distance,
                ];
                assert_projection(&uniforms, world, [horizontal, vertical, depth], aspect);
            }
        }
    }
}
