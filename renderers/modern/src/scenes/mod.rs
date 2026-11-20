//! Scene registry for the modern renderer.
//!
//! Scene byte 7 is the colour field ([`colour_field`]), checked before the
//! registry mapping on every frame (see [`dispatch`]); the colour field is
//! stateful, so it is not a registry entry. Every other byte keeps its
//! registry scene.
//!
//! Registry scenes that need memory between frames (Lines and Static)
//! keep it in [`SceneState`], which the renderer steps once per new frame
//! and passes to [`SceneBuild::Stateful`] builders. PulseTunnel variants also
//! smooth audio on each redraw or replay step, including held frames.
//!
//! Each scene lives in its own module and exposes one [`SceneEntry`]. The
//! first six entries preserve the original wire-byte mapping for every byte:
//! `scene % 6`. Byte 7 is checked first and remains reserved for ColourField.
//! Starfield is opt-in and is selected only for explicit scene byte 6; all
//! other bytes retain their original mapping.

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
use std::cell::{Ref, RefCell};

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
    pub tunnel_variant: TunnelVariant,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TunnelVariant {
    #[default]
    Original,
    Hallway,
    Abstract,
}

impl TunnelVariant {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "original" => Some(Self::Original),
            "hallway" => Some(Self::Hallway),
            "abstract" => Some(Self::Abstract),
            _ => None,
        }
    }
}

/// State registry scenes keep between frames. [`observe`](Self::observe)
/// steps Lines and Static once per new frame, whichever scene is shown.
/// PulseTunnel's audio envelope is advanced separately per redraw or replay
/// step using the displayed frame, including on held redraws. Builders read
/// the resulting state without advancing it.
#[derive(Clone, Debug, Default)]
pub struct SceneState {
    config: SceneConfig,
    pub(crate) lines: lines::LinesState,
    pub(crate) static_noise: static_noise::StaticState,
    pub(crate) tunnel: pulse_tunnel::TunnelState,
    starfield: Option<starfield::StarfieldConfig>,
    starfield_dispatch_enabled: bool,
    starfield_projection: RefCell<Option<starfield::ProjectionCache>>,
}

impl SceneState {
    pub fn new(config: SceneConfig) -> Self {
        Self {
            config,
            lines: lines::LinesState::new(config.lines_drop_cuts),
            static_noise: static_noise::StaticState::default(),
            tunnel: pulse_tunnel::TunnelState::default(),
            starfield: None,
            starfield_dispatch_enabled: false,
            starfield_projection: RefCell::new(None),
        }
    }

    pub fn config(&self) -> SceneConfig {
        self.config
    }

    pub fn configure_starfield(&mut self, config: Option<starfield::StarfieldConfig>) {
        self.starfield = config;
        *self.starfield_projection.get_mut() = None;
    }

    pub(crate) fn configure_starfield_dispatch(&mut self, enabled: bool) {
        self.starfield_dispatch_enabled = enabled;
    }

    pub(crate) fn starfield_dispatch_enabled(&self) -> bool {
        self.starfield_dispatch_enabled
    }

    pub(crate) fn starfield(&self) -> Option<&starfield::StarfieldConfig> {
        self.starfield.as_ref()
    }

    pub(crate) fn starfield_projection(&self) -> Ref<'_, Option<starfield::ProjectionCache>> {
        self.starfield_projection.borrow()
    }

    pub(crate) fn starfield_projection_mut(
        &self,
    ) -> std::cell::RefMut<'_, Option<starfield::ProjectionCache>> {
        self.starfield_projection.borrow_mut()
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

    /// Smooth the displayed frame once per redraw or capture replay step.
    /// Fresh packet observations do not advance this envelope.
    pub(crate) fn advance_tunnel(&mut self, frame: &ControlFrame, elapsed: f64) {
        self.tunnel.advance(frame, elapsed);
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
const LEGACY_SCENE_COUNT: usize = 6;

/// The legacy mapping for native scenes. Callers check
/// [`colour_field::SCENE`] first (see [`dispatch`]); Starfield is deliberately
/// not selected by this mapping.
pub fn select(scene: u8) -> &'static SceneEntry {
    &REGISTRY[usize::from(scene) % LEGACY_SCENE_COUNT]
}

/// Resolves an input byte to the actual native registry index. The explicit
/// Starfield opt-in applies only to exact scene byte 6.
pub fn normalize_scene_byte(scene: u8, starfield_enabled: bool) -> u8 {
    if starfield_enabled && scene == starfield::SCENE {
        starfield::SCENE
    } else {
        (usize::from(scene) % LEGACY_SCENE_COUNT) as u8
    }
}

/// Selects Starfield only when the caller explicitly enabled it and the
/// displayed byte is exactly 6. Wrapped wire bytes always keep legacy mapping.
pub fn select_configured(scene: u8, starfield_enabled: bool) -> &'static SceneEntry {
    &REGISTRY[usize::from(normalize_scene_byte(scene, starfield_enabled))]
}

/// Variant selection affects only PulseTunnel, including its wrapped bytes.
pub fn selected_camera(scene: u8, config: SceneConfig) -> CameraMode {
    if select(scene).name == pulse_tunnel::ENTRY.name
        && config.tunnel_variant != TunnelVariant::Original
    {
        CameraMode::ScreenSpace
    } else {
        select(scene).camera
    }
}

pub(crate) fn selected_camera_configured(
    scene: u8,
    starfield_enabled: bool,
    config: SceneConfig,
) -> CameraMode {
    let selected = dispatch_configured(scene, starfield_enabled);
    if selected.name() == pulse_tunnel::ENTRY.name
        && config.tunnel_variant != TunnelVariant::Original
    {
        CameraMode::ScreenSpace
    } else {
        selected.camera()
    }
}

pub(crate) fn build_tunnel_variant(
    profile: &RenderProfile,
    frame: ControlFrame,
    elapsed: f32,
    aspect: f32,
    state: &SceneState,
    instances: &mut Vec<Instance>,
) -> bool {
    if select(frame.scene).name != pulse_tunnel::ENTRY.name
        || state.config.tunnel_variant == TunnelVariant::Original
    {
        return false;
    }
    pulse_tunnel::build_variant(profile, frame, elapsed, aspect, state, instances);
    true
}

/// What a legacy scene byte selects: 7 is the colour field on every target;
/// all other bytes use the original modulo-six native-scene mapping.
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
    dispatch_configured(scene, false)
}

/// Dispatches a scene byte with the explicit local Starfield opt-in applied.
pub fn dispatch_configured(scene: u8, starfield_enabled: bool) -> Selected {
    if scene == colour_field::SCENE {
        Selected::ColourField
    } else {
        Selected::Registry(select_configured(scene, starfield_enabled))
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

    #[test]
    fn tunnel_variants_are_selected_only_for_tunnel_and_follow_audio() {
        let frame = ControlFrame {
            scene: 2,
            palette: 1,
            ..ControlFrame::procedural(0)
        };
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            for variant in [TunnelVariant::Hallway, TunnelVariant::Abstract] {
                let config = SceneConfig {
                    tunnel_variant: variant,
                    ..SceneConfig::default()
                };
                let mut quiet = SceneState::new(config);
                let mut loud = SceneState::new(config);
                let silent = ControlFrame {
                    energy: 0,
                    bass: 0,
                    treble: 0,
                    ..frame
                };
                let strong = ControlFrame {
                    energy: 255,
                    bass: 255,
                    treble: 255,
                    ..frame
                };
                for i in 0..120 {
                    quiet.advance_tunnel(&silent, f64::from(i) / 60.0);
                    loud.advance_tunnel(&strong, f64::from(i) / 60.0);
                }
                for aspect in [16.0 / 9.0, 4.0 / 3.0] {
                    let a = build_instances(&profile, frame, 2.0, aspect, &quiet);
                    let b = build_instances(&profile, frame, 2.0, aspect, &loud);
                    let again = build_instances(&profile, frame, 2.0, aspect, &quiet);
                    assert!(!a.is_empty() && a.len() <= profile.instance_count);
                    assert_eq!(
                        bytemuck::cast_slice::<Instance, u8>(&a),
                        bytemuck::cast_slice::<Instance, u8>(&again)
                    );
                    assert_ne!(
                        bytemuck::cast_slice::<Instance, u8>(&a),
                        bytemuck::cast_slice::<Instance, u8>(&b)
                    );
                    let later = build_instances(&profile, frame, 3.0, aspect, &quiet);
                    assert_ne!(
                        bytemuck::cast_slice::<Instance, u8>(&a),
                        bytemuck::cast_slice::<Instance, u8>(&later)
                    );
                    assert_eq!(
                        selected_camera(frame.scene, config),
                        CameraMode::ScreenSpace
                    );
                    assert_eq!(selected_camera(8, config), CameraMode::ScreenSpace);
                    let other = ControlFrame { scene: 0, ..frame };
                    assert_eq!(selected_camera(other.scene, config), CameraMode::Orbit3D);
                    assert_eq!(
                        bytemuck::cast_slice::<Instance, u8>(&build_instances(
                            &profile, other, 2.0, aspect, &quiet
                        )),
                        bytemuck::cast_slice::<Instance, u8>(&build_instances(
                            &profile,
                            other,
                            2.0,
                            aspect,
                            &SceneState::default()
                        ))
                    );
                }
                let hallway = SceneState::new(SceneConfig {
                    tunnel_variant: TunnelVariant::Hallway,
                    ..SceneConfig::default()
                });
                let abstract_space = SceneState::new(SceneConfig {
                    tunnel_variant: TunnelVariant::Abstract,
                    ..SceneConfig::default()
                });
                assert_ne!(
                    bytemuck::cast_slice::<Instance, u8>(&build_instances(
                        &profile, frame, 2.0, 1.5, &hallway
                    )),
                    bytemuck::cast_slice::<Instance, u8>(&build_instances(
                        &profile,
                        frame,
                        2.0,
                        1.5,
                        &abstract_space
                    ))
                );
            }
        }
    }

    /// FNV-1a over the instance bytes for every profile/frame/elapsed sample,
    /// with the scene byte forced to `scene` or its wrapped `scene + 6`.
    fn fingerprint(
        scene: u8,
        build: fn(&RenderProfile, ControlFrame, f32) -> Vec<Instance>,
    ) -> u64 {
        let mut fingerprint: u64 = 0xcbf2_9ce4_8422_2325;
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            for index in INDICES {
                for elapsed in ELAPSED {
                    let frame = ControlFrame {
                        scene: scene + LEGACY_SCENE_COUNT as u8 * (index as u8 % 2),
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
    fn legacy_registry_mapping_is_unchanged_for_every_scene_byte() {
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
                "SignalField",
                "SpectrumVolume",
                "PulseTunnel",
                "Lines",
            ]
        );
        for scene in 0..=u8::MAX {
            assert_eq!(
                select(scene).name,
                REGISTRY[usize::from(scene) % LEGACY_SCENE_COUNT].name
            );
            assert_eq!(normalize_scene_byte(scene, false), scene % 6);
            assert_eq!(select_configured(scene, false).name, select(scene).name);
            if scene != starfield::SCENE {
                assert_eq!(normalize_scene_byte(scene, true), scene % 6);
                assert_eq!(select_configured(scene, true).name, select(scene).name);
            }
        }
        assert_eq!(normalize_scene_byte(6, true), 6);
        assert_eq!(select_configured(6, true).name, "Starfield");
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
        assert_eq!(dispatch(6).name(), "SignalField");
        assert_eq!(select_configured(6, true).name, "Starfield");
        assert_eq!(
            selected_camera_configured(6, true, SceneConfig::default()),
            CameraMode::ScreenSpace
        );
        assert_eq!(
            selected_camera_configured(6, false, SceneConfig::default()),
            CameraMode::Orbit3D
        );
        assert_eq!(
            selected_camera_configured(
                9,
                true,
                SceneConfig {
                    tunnel_variant: TunnelVariant::Hallway,
                    ..SceneConfig::default()
                }
            ),
            CameraMode::ScreenSpace
        );
    }

    #[test]
    fn starfield_dispatch_and_profile_instance_budgets() {
        let selected = dispatch_configured(starfield::SCENE, true);
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
        state.configure_starfield_dispatch(true);
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
                    let scene = frame.scene % LEGACY_SCENE_COUNT as u8;
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
