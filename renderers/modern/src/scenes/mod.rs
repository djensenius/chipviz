//! Scene registry for the modern renderer.
//!
//! Scene byte 7 is the colour field ([`colour_field`]), checked before the
//! registry mapping on every frame (see [`dispatch`]); the colour field is
//! stateful, so it is not a registry entry. Every other byte keeps its
//! registry scene.
//!
//! Registry scenes that need memory between frames (Lines and Static)
//! keep it in [`SceneState`], which the renderer steps once per new frame
//! and passes to [`SceneBuild::Stateful`] builders; other scenes ignore it.
//!
//! Each scene lives in its own module and exposes one [`SceneEntry`]. The
//! control frame's `scene` byte selects `REGISTRY[scene % REGISTRY.len()]`, so
//! with the current seven entries scene 0/14/21... is SignalField, 1/8/15... is
//! SpectrumVolume, 2/9/16... is PulseTunnel, 3/10/17... is Lines, 4/11/18...
//! is Static, 5/12/19... is Object and 6/13/20... is Starfield. Byte 7 is
//! separately reserved for ColourField. Adding a scene is one new module plus
//! one `REGISTRY` entry. Appending keeps every index below the old length on
//! the same scene; wrapped indices (>= length) move with the new length.
//! Starfield is appended at byte 6, leaving the ColourField override at byte
//! 7 untouched.

pub mod colour_field;
mod lines;
pub mod mic_layers;
mod object;
mod pulse_tunnel;
#[cfg(test)]
mod reference;
mod signal_field;
mod spectrum_volume;
pub mod starfield;
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

/// A [`SceneBuilder`] that also reads the state its scene keeps between
/// frames (see [`SceneState`]).
pub type StatefulBuilder =
    fn(&RenderProfile, ControlFrame, f32, f32, &SceneState, &mut Vec<Instance>);

/// A stateful scene that also uses the output viewport in pixels.
pub type ViewportStatefulBuilder =
    fn(&RenderProfile, ControlFrame, f32, f32, (u32, u32), &SceneState, &mut Vec<Instance>);

/// How a registry scene builds its instances. Most scenes derive everything
/// from the control frame and elapsed time; a stateful one also reads the
/// state [`SceneState::observe`] kept from earlier frames.
#[derive(Clone, Copy)]
pub enum SceneBuild {
    Stateless(SceneBuilder),
    Stateful(StatefulBuilder),
    ViewportStateful(ViewportStatefulBuilder),
}

pub struct SceneEntry {
    pub name: &'static str,
    pub camera: CameraMode,
    pub build: SceneBuild,
}

/// Opt-in scene behaviour chosen at start-up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneConfig {
    /// Lines cuts straight to its other shape on a drop flag (at most once
    /// per bar); see [`lines`].
    pub lines_drop_cuts: bool,
}

/// State registry scenes keep between frames. [`observe`](Self::observe)
/// steps every stateful scene once per new frame, whichever scene is shown,
/// so a scene resumes in time when it is selected again; builders only read
/// it, so a held redraw repeats the last step. Stateless scenes ignore it.
#[derive(Clone, Debug, Default)]
pub struct SceneState {
    config: SceneConfig,
    pub(crate) lines: lines::LinesState,
    pub(crate) static_noise: static_noise::StaticState,
    starfield: Option<starfield::StarfieldConfig>,
}

impl SceneState {
    pub fn new(config: SceneConfig) -> Self {
        Self {
            config,
            lines: lines::LinesState::new(config.lines_drop_cuts),
            static_noise: static_noise::StaticState::default(),
            starfield: None,
        }
    }

    pub fn config(&self) -> SceneConfig {
        self.config
    }

    pub fn configure_starfield(&mut self, config: Option<starfield::StarfieldConfig>) {
        self.starfield = config;
    }

    pub(crate) fn starfield(&self) -> Option<&starfield::StarfieldConfig> {
        self.starfield.as_ref()
    }

    /// Feeds one new (not held) frame to every stateful scene, counted as
    /// 1/60 s where a scene needs time (tests).
    #[cfg(test)]
    pub fn observe(&mut self, frame: &ControlFrame) {
        self.lines.observe(frame);
        self.static_noise.observe(frame);
    }

    /// As [`observe`](Self::observe) at `elapsed` seconds on the renderer's
    /// clock, so time-limited changes (Static's energy slew) hold at any
    /// frame rate.
    pub fn observe_at(&mut self, frame: &ControlFrame, elapsed: f64) {
        self.lines.observe(frame);
        self.static_noise.observe_at(frame, elapsed);
    }
}

pub const REGISTRY: &[SceneEntry] = &[
    signal_field::ENTRY,
    spectrum_volume::ENTRY,
    pulse_tunnel::ENTRY,
    lines::ENTRY,
    static_noise::ENTRY,
    object::ENTRY,
    starfield::ENTRY,
];

/// The registry mapping for native scenes. Callers check
/// [`colour_field::SCENE`] first (see [`dispatch`]).
pub fn select(scene: u8) -> &'static SceneEntry {
    &REGISTRY[usize::from(scene) % REGISTRY.len()]
}

/// What a scene byte selects: 7 is the colour field on every target,
/// whatever the registry length; anything else is `REGISTRY[scene % len]`.
#[derive(Clone, Copy)]
pub enum Selected {
    ColourField,
    Registry(&'static SceneEntry),
}

impl Selected {
    pub fn name(self) -> &'static str {
        match self {
            Self::ColourField => colour_field::NAME,
            Self::Registry(entry) => entry.name,
        }
    }

    pub fn camera(self) -> CameraMode {
        match self {
            Self::ColourField => CameraMode::ScreenSpace,
            Self::Registry(entry) => entry.camera,
        }
    }
}

pub fn dispatch(scene: u8) -> Selected {
    if scene == colour_field::SCENE {
        Selected::ColourField
    } else {
        Selected::Registry(select(scene))
    }
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

/// `(x.sin(), x.cos())`, computed identically in debug and release builds.
///
/// With optimisation, LLVM merges a `sin` and `cos` of the same value into
/// one `sincos` libm call (`__sincosf_stret` on Apple), which differs from
/// separate `sinf`/`cosf` in the last ulp for some inputs, so instance data
/// (and the recorded goldens) would depend on the build profile.
/// Scene and camera code that feeds rendered output uses this instead of
/// `f32::sin_cos` or a `sin`/`cos` pair on the same value.
///
/// `clippy.toml` bans `f32::sin_cos` and `f32::cos`
/// (`clippy::disallowed_methods`). Every fused pair needs a `cos`, so banning
/// it catches a same-value `x.sin()`/`x.cos()` pair as well as `sin_cos`; a
/// lone cosine uses [`cos`] (see #112, #114).
pub(crate) fn sin_cos(x: f32) -> (f32, f32) {
    (out_of_line_sin(x), out_of_line_cos(x))
}

/// `x.cos()`, for a lone cosine now that `f32::cos` is banned (see
/// [`sin_cos`]). Out of line, so it can never fuse with a `sin`.
pub(crate) fn cos(x: f32) -> f32 {
    out_of_line_cos(x)
}

// Out of line so LLVM never sees both calls on the same value, and so does
// not fuse them (or constant-fold them differently from the runtime libm).
#[inline(never)]
fn out_of_line_sin(x: f32) -> f32 {
    x.sin()
}

#[inline(never)]
#[allow(clippy::disallowed_methods)]
fn out_of_line_cos(x: f32) -> f32 {
    x.cos()
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
                "Object",
                "Starfield",
                "SignalField",
                "SpectrumVolume",
                "PulseTunnel",
            ]
        );
        assert_eq!(select(255).name, REGISTRY[255 % REGISTRY.len()].name);
        let cameras: Vec<_> = REGISTRY.iter().map(|entry| entry.camera).collect();
        let orbit = CameraMode::Orbit3D;
        let flat = CameraMode::ScreenSpace;
        assert_eq!(cameras, [orbit, orbit, orbit, flat, flat, flat, flat]);
    }

    #[test]
    fn scene_seven_dispatches_to_colour_field_before_the_registry() {
        let names: Vec<_> = (0..=13).map(|scene| dispatch(scene).name()).collect();
        let mut expected: Vec<_> = (0..=13).map(|scene| select(scene).name).collect();
        expected[7] = "ColourField";
        assert_eq!(names, expected);
        for scene in [15, 23, 71, 255] {
            assert_eq!(dispatch(scene).name(), select(scene).name);
        }
        assert_eq!(dispatch(7).camera(), CameraMode::ScreenSpace);
        assert!(REGISTRY.iter().all(|entry| entry.name != "ColourField"));
        assert_eq!(dispatch(6).name(), "Starfield");
    }

    #[test]
    fn starfield_dispatch_and_profile_instance_budgets() {
        let selected = dispatch(starfield::SCENE);
        assert_eq!(selected.name(), "Starfield");
        assert_eq!(selected.camera(), CameraMode::ScreenSpace);

        let mut csv = String::from("ra,dec,mag\n");
        for index in 0..20_000 {
            let magnitude = 6.5 - (index % 8_000) as f64 / 1_000.0;
            csv.push_str(&format!("0,90,{magnitude}\n"));
        }
        let catalog = starfield::StarCatalog::from_reader(csv.as_bytes()).expect("catalog");
        let mut state = SceneState::default();
        state.configure_starfield(Some(starfield::StarfieldConfig {
            catalog: std::sync::Arc::new(catalog),
            latitude_degrees: 45.0,
            longitude_degrees: 0.0,
            unix_seconds: 946_728_000.0,
            follow_elapsed: false,
        }));
        for (profile, budget) in [(RenderProfile::m1(), 16_384), (RenderProfile::pi5(), 4_096)] {
            let frame = ControlFrame {
                scene: starfield::SCENE,
                ..ControlFrame::procedural(0)
            };
            let instances = build_instances(&profile, frame, 0.0, 16.0 / 9.0, &state);
            assert!(!instances.is_empty());
            assert!(instances.len() <= budget);
            assert!(instances.len() <= 400);
            assert!(instances[0].color[0] <= instances.last().unwrap().color[0]);
            assert!(instances.iter().all(|instance| {
                instance.center[0].is_finite()
                    && instance.center[1].is_finite()
                    && (-1.1..=1.1).contains(&instance.center[0])
                    && (-1.0..=1.0).contains(&instance.center[1])
            }));
        }
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
                    build_instances(profile, frame, elapsed, 16.0 / 9.0, &SceneState::default())
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
                    build_instances(profile, frame, elapsed, 16.0 / 9.0, &SceneState::default())
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
