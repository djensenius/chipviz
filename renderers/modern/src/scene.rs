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

pub fn build_uniforms(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    aspect: f32,
) -> Uniforms {
    let energy = frame.energy as f32 / 255.0;
    let beat = if frame.beat_onset() { 1.0 } else { 0.0 };
    let phase = frame.beat_phase as f32 / 255.0;
    let tempo = frame.bpm as f32 / 255.0;
    let beat_drift = (frame.beat_count as f32 * 0.015).sin() * tempo;
    let angle = elapsed * profile.camera_speed + phase * std::f32::consts::TAU + beat_drift;
    let radius = profile.camera_radius - energy * 1.8;
    let eye = [
        angle.cos() * radius,
        1.8 + energy * 2.2,
        angle.sin() * radius,
    ];
    let target = [0.0, beat_drift * 0.5, 0.0];
    let view = look_at(eye, target, [0.0, 1.0, 0.0]);
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
            let uniforms = build_uniforms(&profile, frame, 0.0, aspect);
            // At zero phase/time/drift the eye is (radius, 1.8, 0),
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
                    let uniforms = build_uniforms(&profile, frame, 0.0, aspect);
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
    fn uniforms_project_orbiting_camera_depth_and_beat_drift() {
        let frame = ControlFrame {
            energy: 255,
            bpm: 180,
            beat_phase: 32,
            beat_count: 37,
            ..ControlFrame::procedural(0)
        };
        let drift = (f32::from(frame.beat_count) * 0.015).sin() * f32::from(frame.bpm) / 255.0;
        let target_y = drift * 0.5;
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            // Choose elapsed time so the orbit is a quarter turn: eye = (0, 4, radius).
            let elapsed = (std::f32::consts::FRAC_PI_2
                - f32::from(frame.beat_phase) / 255.0 * std::f32::consts::TAU
                - drift)
                / profile.camera_speed;
            let radius = profile.camera_radius - 1.8;
            let height = 4.0 - target_y;
            let distance = radius.hypot(height);
            let aspect = 4.0 / 3.0;
            let uniforms = build_uniforms(&profile, frame, elapsed, aspect);
            assert_projection(
                &uniforms,
                [0.0, target_y, 0.0],
                [0.0, 0.0, distance],
                aspect,
            );
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
