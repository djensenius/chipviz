use bytemuck::{Pod, Zeroable};

use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scenes::{self, CameraMode};

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
    /// ScreenSpace fill pattern; `[0, 0]` (every scene but Static) is a solid
    /// quad. Positive values are a 5x8 glyph bitmap (rows 0..3 in `[0]`,
    /// rows 4..7 in `[1]`, 5 bits per row, bit 0 = top-left); a negative `[0]`
    /// is per-pixel noise. See `scenes::static_noise` and `shader.wgsl`.
    pub pattern: [f32; 2],
}

impl Instance {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
      1 => Float32x3,
      2 => Float32x2,
      3 => Float32x4,
      4 => Float32,
      5 => Float32,
      6 => Float32x2,
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
    /// 1.0 for ScreenSpace scenes: the shader skips edge darkening and pulse.
    pub screen_space: f32,
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
    aspect: f32,
) -> Vec<Instance> {
    let mut instances = Vec::with_capacity(profile.instance_count);
    (scenes::select(frame.scene).build)(profile, frame, elapsed, aspect, &mut instances);
    instances
}

/// Flattens screen-space instances: x/y pass through as NDC (-1..1 spans the
/// viewport) and every depth, including the shader's beat push, maps to 0.5.
pub const SCREEN_SPACE_TRANSFORM: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, 0.5, 1.0],
];

/// Uniforms for a scene's camera mode. The orbit camera always advances so
/// switching between camera modes keeps its smoothing continuous.
pub fn scene_uniforms(
    mode: CameraMode,
    camera: &mut Camera,
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    aspect: f32,
) -> Uniforms {
    let mut uniforms = camera.build_uniforms(profile, frame, elapsed, aspect);
    if mode == CameraMode::ScreenSpace {
        uniforms.view_projection = SCREEN_SPACE_TRANSFORM;
        uniforms.screen_space = 1.0;
    }
    uniforms
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
            screen_space: 0.0,
        }
    }
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
