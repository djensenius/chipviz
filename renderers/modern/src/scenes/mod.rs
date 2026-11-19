//! Scene registry for the modern renderer.
//!
//! Each scene lives in its own module and exposes one [`SceneEntry`]. The
//! control frame's `scene` byte selects `REGISTRY[scene % REGISTRY.len()]`, so
//! with the current five entries scene 0/5/10... is SignalField, 1/6/11... is
//! SpectrumVolume, 2/7/12... is PulseTunnel, 3/8/13... is Lines and
//! 4/9/14... is Static. Adding a scene is one new module plus one `REGISTRY`
//! entry. Appending keeps every index below the old length on the same scene;
//! wrapped indices (>= length) move with the new length. Adding Lines shifted
//! bytes 3, 4 and 5 to Lines, SignalField and SpectrumVolume; adding Static
//! shifted bytes 4..7 (formerly SignalField, SpectrumVolume, PulseTunnel and
//! Lines) to Static, SignalField, SpectrumVolume and PulseTunnel.

mod lines;
mod pulse_tunnel;
#[cfg(test)]
mod reference;
mod signal_field;
mod spectrum_volume;
mod static_noise;
#[cfg(test)]
pub(crate) use static_noise::gpu_reference as static_gpu_reference;

use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scene::Instance;

/// How a scene's instance positions are transformed to the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode {
    /// World-space instances viewed through the smoothed orbit camera.
    Orbit3D,
    /// Flat instances whose `center.xy` are NDC (-1..1 spans the viewport,
    /// +y up) and whose depth is ignored; draw order is instance order. The
    /// shader draws their colour as-is: no quad-edge darkening or beat pulse.
    ScreenSpace,
}

/// Appends at most `profile.instance_count` instances for one frame, given
/// elapsed seconds and the viewport aspect (width / height).
pub type SceneBuilder = fn(&RenderProfile, ControlFrame, f32, f32, &mut Vec<Instance>);

pub struct SceneEntry {
    pub name: &'static str,
    pub camera: CameraMode,
    pub build: SceneBuilder,
}

pub const REGISTRY: &[SceneEntry] = &[
    signal_field::ENTRY,
    spectrum_volume::ENTRY,
    pulse_tunnel::ENTRY,
    lines::ENTRY,
    static_noise::ENTRY,
];

pub fn select(scene: u8) -> &'static SceneEntry {
    &REGISTRY[usize::from(scene) % REGISTRY.len()]
}

pub(crate) fn palette(index: u8, brightness: f32, energy: f32) -> [f32; 4] {
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

pub(crate) fn note_emphasis(frame: ControlFrame) -> f32 {
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

/// Keep in sync with `mix` in `static_noise/mod.rs` and `mix32` in
/// `shader.wgsl`.
pub(crate) fn hash(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value as f32 / u32::MAX as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Camera, SCREEN_SPACE_TRANSFORM, build_instances, scene_uniforms};

    const INDICES: [u64; 9] = [0, 1, 17, 95, 96, 200, 511, 1000, 65_600];
    const ELAPSED: [f32; 4] = [0.0, 0.5, 3.25, 120.0];

    /// FNV-1a over the instance bytes for every profile/frame/elapsed sample,
    /// with the scene byte forced to `scene` or its wrapped `scene + REGISTRY.len()`.
    fn fingerprint(
        scene: u8,
        build: fn(&RenderProfile, ControlFrame, f32) -> Vec<Instance>,
    ) -> u64 {
        let mut fingerprint: u64 = 0xcbf2_9ce4_8422_2325;
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            for index in INDICES {
                for elapsed in ELAPSED {
                    let frame = ControlFrame {
                        scene: scene + REGISTRY.len() as u8 * (index as u8 % 2),
                        ..ControlFrame::procedural(index)
                    };
                    let instances = build(&profile, frame, elapsed);
                    for byte in bytemuck::cast_slice::<Instance, u8>(&instances) {
                        fingerprint ^= u64::from(*byte);
                        fingerprint = fingerprint.wrapping_mul(0x0100_0000_01b3);
                    }
                }
            }
        }
        fingerprint
    }

    #[test]
    fn registry_maps_scene_byte_modulo_length() {
        let names: Vec<_> = (0..=9).map(|scene| select(scene).name).collect();
        assert_eq!(
            names,
            [
                "SignalField",
                "SpectrumVolume",
                "PulseTunnel",
                "Lines",
                "Static",
                "SignalField",
                "SpectrumVolume",
                "PulseTunnel",
                "Lines",
                "Static",
            ]
        );
        assert_eq!(select(255).name, REGISTRY[255 % REGISTRY.len()].name);
        let cameras: Vec<_> = REGISTRY.iter().map(|entry| entry.camera).collect();
        let orbit = CameraMode::Orbit3D;
        let flat = CameraMode::ScreenSpace;
        assert_eq!(cameras, [orbit, orbit, orbit, flat, flat]);
    }

    #[test]
    fn registry_names_are_unique() {
        for (index, entry) in REGISTRY.iter().enumerate() {
            assert!(
                REGISTRY[..index]
                    .iter()
                    .all(|other| other.name != entry.name)
            );
        }
    }

    #[test]
    fn registry_scenes_match_pre_registry_code_byte_for_byte() {
        for scene in 0..3 {
            assert_eq!(
                fingerprint(scene, |profile, frame, elapsed| {
                    build_instances(profile, frame, elapsed, 16.0 / 9.0)
                }),
                fingerprint(scene, |profile, frame, elapsed| {
                    let scene = frame.scene % REGISTRY.len() as u8;
                    reference::build_instances(profile, ControlFrame { scene, ..frame }, elapsed)
                }),
                "scene {scene}"
            );
        }
    }

    // Recorded from the original scene.rs (commit 559ec5d) on Apple Silicon.
    // Gated to Apple Silicon macOS because platform libm sin/cos may differ in the last ulp;
    // the reference comparison above covers every platform.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn registry_scenes_match_recorded_pre_registry_golden() {
        let golden = [
            0x15ab_93e6_2324_545d,
            0xabfa_8536_4088_5675,
            0xcbbe_f57a_5e83_f9f9,
        ];
        for (scene, expected) in golden.into_iter().enumerate() {
            assert_eq!(
                fingerprint(scene as u8, |profile, frame, elapsed| {
                    build_instances(profile, frame, elapsed, 16.0 / 9.0)
                }),
                expected,
                "scene {scene}"
            );
        }
    }

    #[test]
    fn screen_space_transform_passes_xy_and_flattens_depth() {
        // Shader: world = (center.xy + rotated quad, center.z + beat * 0.12, 1).
        for world in [[0.0, 0.0, 0.0], [0.5, -0.5, 9.0], [-1.0, 1.0, -40.12]] {
            let clip: [f32; 4] = std::array::from_fn(|row| {
                (0..3)
                    .map(|column| SCREEN_SPACE_TRANSFORM[column][row] * world[column])
                    .sum::<f32>()
                    + SCREEN_SPACE_TRANSFORM[3][row]
            });
            assert_eq!(clip, [world[0], world[1], 0.5, 1.0]);
        }
    }

    #[test]
    fn scene_uniforms_follow_camera_mode() {
        let profile = RenderProfile::m1();
        let frame = ControlFrame::procedural(0);
        let expected = Camera::default().build_uniforms(&profile, frame, 1.0, 1.5);
        let orbit = CameraMode::Orbit3D;
        let actual = scene_uniforms(orbit, &mut Camera::default(), &profile, frame, 1.0, 1.5);
        assert_eq!(expected.view_projection, actual.view_projection);

        let flat = CameraMode::ScreenSpace;
        let actual = scene_uniforms(flat, &mut Camera::default(), &profile, frame, 1.0, 1.5);
        assert_eq!(actual.view_projection, SCREEN_SPACE_TRANSFORM);
        assert_eq!((expected.screen_space, actual.screen_space), (0.0, 1.0));
        assert_eq!((actual.time, actual.beat), (expected.time, expected.beat));
    }
}
