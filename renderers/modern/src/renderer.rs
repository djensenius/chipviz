use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;

use crate::audio::scope::ScopeSnapshot;
use crate::control_frame::ControlFrame;
use crate::phosphor;
use crate::profile::RenderProfile;
use crate::scene::{
    Camera, Instance, QUAD_VERTICES, Uniforms, Vertex, build_instances_for_viewport, scene_uniforms,
};
use crate::scenes::{self, CameraMode, SceneConfig, SceneState, colour_field, mic_layers};

const MAX_STARFIELD_CROSSINGS_PER_SECOND: usize = 2;
const STARFIELD_CROSSING_WINDOW_SECONDS: f64 = 1.0;

/// Offscreen capture format; sRGB like the window's preferred surface format.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Pipeline, buffers, camera and colour-field state shared by the window and
/// offscreen targets.
pub struct ScenePipeline {
    profile: RenderProfile,
    camera: Camera,
    colour_field: colour_field::ColourField,
    colour_controls: colour_field::Controls,
    /// One-frame scene fallback requested after displayed-scene validation.
    next_native_fallback: Option<u8>,
    /// Wall-clock step cap for live rendering; `None` steps every new frame.
    step_clock: Option<(colour_field::StepClock, Instant)>,
    /// Waveform/onset overlay over native scenes; see `scenes::mic_layers`.
    mic_layers: mic_layers::MicLayers,
    /// The mic side channel's latest snapshot while mic frames are shown;
    /// `None` makes the layers fall back to the control frame.
    live_scope: Option<ScopeSnapshot>,
    /// State registry scenes keep between observed frames.
    scene_state: SceneState,
    /// The colour attachment encodes sRGB on write, so flat colours are
    /// given in linear light; otherwise they are written as sRGB bytes.
    srgb_target: bool,
    device: wgpu::Device,
    queue: wgpu::Queue,
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    /// Scene instances; the overlay's slots follow them in the same buffer.
    instance_capacity: usize,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    overlay_uniform_buffer: wgpu::Buffer,
    overlay_bind_group: wgpu::BindGroup,
    /// Draws the mic layers into the phosphor's emitted target.
    overlay_pipeline: wgpu::RenderPipeline,
    phosphor_pipelines: phosphor::Pipelines,
    /// The mic layers' afterglow; made when they first draw, see `phosphor`.
    phosphor: Option<phosphor::Phosphor>,
    /// Holds off remaking the phosphor while the target is being resized.
    settle: phosphor::Settle,
    /// Rate-limits only actual transitions into and out of Starfield.
    crossing_gate: StarfieldCrossingGate,
}

pub struct Renderer {
    window: Arc<winit::window::Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: ScenePipeline,
}

/// Renders the same pipeline into a texture and reads it back as RGB8.
pub struct OffscreenRenderer {
    pipeline: ScenePipeline,
    width: u32,
    height: u32,
    padded_row: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

#[derive(Debug)]
pub enum RendererError {
    CreateSurface(wgpu::CreateSurfaceError),
    NoAdapter,
    RequestDevice(wgpu::RequestDeviceError),
    NoSurfaceFormat,
    Surface(wgpu::SurfaceError),
    Readback(wgpu::BufferAsyncError),
    TargetTooLarge {
        width: u32,
        height: u32,
        max_dimension: u32,
    },
}

impl std::fmt::Display for RendererError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateSurface(error) => write!(formatter, "failed to create surface: {error}"),
            Self::NoAdapter => formatter.write_str("no compatible GPU adapter found"),
            Self::RequestDevice(error) => {
                write!(formatter, "failed to request GPU device: {error}")
            }
            Self::NoSurfaceFormat => {
                formatter.write_str("surface does not expose any supported formats")
            }
            Self::Surface(error) => write!(formatter, "surface error: {error}"),
            Self::Readback(error) => write!(formatter, "texture readback failed: {error}"),
            Self::TargetTooLarge {
                width,
                height,
                max_dimension,
            } => write!(
                formatter,
                "capture size {width}x{height} exceeds the GPU limit \
                 ({max_dimension} px per side, or the readback buffer limit)"
            ),
        }
    }
}

impl std::error::Error for RendererError {}

#[derive(Debug)]
struct StarfieldCrossingGate {
    displayed: colour_field::Display,
    recent_crossings: VecDeque<f64>,
    last_elapsed: f64,
}

impl Default for StarfieldCrossingGate {
    fn default() -> Self {
        Self {
            displayed: colour_field::Display::Native(0),
            recent_crossings: VecDeque::new(),
            last_elapsed: 0.0,
        }
    }
}

impl StarfieldCrossingGate {
    fn resolve(
        &mut self,
        requested: colour_field::Display,
        elapsed: f64,
        commit: bool,
    ) -> colour_field::Display {
        let now = if elapsed.is_finite() {
            elapsed.max(self.last_elapsed)
        } else {
            self.last_elapsed
        };
        let crossing = is_starfield_display(self.displayed) != is_starfield_display(requested);
        let recent_count = self
            .recent_crossings
            .iter()
            .filter(|&&time| now - time <= STARFIELD_CROSSING_WINDOW_SECONDS)
            .count();
        let allowed = !crossing || recent_count < MAX_STARFIELD_CROSSINGS_PER_SECOND;
        let displayed = if allowed { requested } else { self.displayed };
        if commit {
            if crossing && allowed {
                while self
                    .recent_crossings
                    .front()
                    .is_some_and(|&time| now - time > STARFIELD_CROSSING_WINDOW_SECONDS)
                {
                    self.recent_crossings.pop_front();
                }
                self.recent_crossings.push_back(now);
            }
            if allowed {
                self.displayed = requested;
            }
            self.last_elapsed = now;
        }
        displayed
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

fn is_starfield_display(display: colour_field::Display) -> bool {
    matches!(
        display,
        colour_field::Display::Native(scene) if scenes::select(scene).name == "Starfield"
    )
}

fn mic_layers_shown(mode: mic_layers::Mode, display: colour_field::Display) -> bool {
    mic_layers::shown(mode, display) && !is_starfield_display(display)
}

fn displayed_scene_byte(display: colour_field::Display) -> u8 {
    match display {
        colour_field::Display::Native(scene) => scene,
        colour_field::Display::ColourField(_) => colour_field::SCENE,
    }
}

fn prepare_starfield_attachment_colors(
    instances: &mut [Instance],
    srgb_target: bool,
    is_starfield: bool,
) {
    if srgb_target || !is_starfield {
        return;
    }
    for instance in instances {
        for channel in &mut instance.color[..3] {
            *channel = if *channel <= 0.003_130_8 {
                *channel * 12.92
            } else {
                1.055 * channel.powf(1.0 / 2.4) - 0.055
            };
        }
    }
}

fn resolve_dispatch_frame(
    mut frame: ControlFrame,
    display: colour_field::Display,
    fallback: Option<u8>,
) -> (ControlFrame, colour_field::Display, bool) {
    match display {
        colour_field::Display::Native(scene) => {
            let fallback_applied = scenes::select(scene).name == "Starfield" && fallback.is_some();
            let scene = if fallback_applied {
                fallback.unwrap_or(scene)
            } else {
                scene
            };
            frame.scene = scene;
            (
                frame,
                colour_field::Display::Native(scene),
                fallback_applied,
            )
        }
        colour_field::Display::ColourField(trace) => {
            (frame, colour_field::Display::ColourField(trace), false)
        }
    }
}

impl From<wgpu::CreateSurfaceError> for RendererError {
    fn from(error: wgpu::CreateSurfaceError) -> Self {
        Self::CreateSurface(error)
    }
}

impl From<wgpu::RequestDeviceError> for RendererError {
    fn from(error: wgpu::RequestDeviceError) -> Self {
        Self::RequestDevice(error)
    }
}

impl From<wgpu::SurfaceError> for RendererError {
    fn from(error: wgpu::SurfaceError) -> Self {
        Self::Surface(error)
    }
}

async fn request_device(
    instance: &wgpu::Instance,
    profile: &RenderProfile,
    compatible_surface: Option<&wgpu::Surface<'_>>,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), RendererError> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: profile.power_preference,
            compatible_surface,
            force_fallback_adapter: false,
        })
        .await
        .ok_or(RendererError::NoAdapter)?;
    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                label: Some("chipviz device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        )
        .await?;
    Ok((adapter, device, queue))
}

fn gpu_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    })
}

impl ScenePipeline {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        profile: RenderProfile,
        aspect: f32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chipviz modern shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        // Buffer initialization must not seed the live camera before its first frame.
        let uniform =
            Camera::default().build_uniforms(&profile, ControlFrame::procedural(0), 0.0, aspect);
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chipviz uniforms"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("chipviz uniform bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chipviz uniform bind group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let overlay_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chipviz overlay uniforms"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let overlay_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chipviz overlay bind group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: overlay_uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("chipviz pipeline layout"),
            bind_group_layouts: &[&uniform_bind_group_layout],
            push_constant_ranges: &[],
        });
        let instance_pipeline = |label, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    buffers: &[Vertex::layout(), Instance::layout()],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            })
        };
        let render_pipeline = instance_pipeline("chipviz render pipeline", format);
        let overlay_pipeline = instance_pipeline("chipviz mic layers pipeline", phosphor::FORMAT);
        let phosphor_pipelines = phosphor::Pipelines::new(&device, format);
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chipviz quad vertices"),
            contents: bytemuck::cast_slice(&QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instance_capacity = profile.instance_count.max(1);
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chipviz instances"),
            size: ((instance_capacity + mic_layers::MAX_INSTANCES)
                * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            profile,
            camera: Camera::default(),
            colour_field: colour_field::ColourField::new(colour_field::Config::default()),
            colour_controls: colour_field::Controls::default(),
            next_native_fallback: None,
            step_clock: None,
            mic_layers: mic_layers::MicLayers::new(mic_layers::Mode::Off),
            live_scope: None,
            scene_state: SceneState::default(),
            srgb_target: format.is_srgb(),
            device,
            queue,
            render_pipeline,
            vertex_buffer,
            instance_buffer,
            instance_capacity,
            uniform_buffer,
            uniform_bind_group,
            overlay_uniform_buffer,
            overlay_bind_group,
            overlay_pipeline,
            phosphor_pipelines,
            phosphor: None,
            settle: phosphor::Settle::default(),
            crossing_gate: StarfieldCrossingGate::default(),
        }
    }

    /// Restarts camera smoothing, the colour field and scene state, as at
    /// renderer start-up.
    pub fn reset_state(&mut self) {
        let starfield = self.scene_state.starfield().cloned();
        self.camera = Camera::default();
        self.colour_field = colour_field::ColourField::new(self.colour_field.config());
        self.mic_layers = mic_layers::MicLayers::new(self.mic_layers.mode());
        self.scene_state = SceneState::new(self.scene_state.config());
        self.scene_state.configure_starfield(starfield);
        self.next_native_fallback = None;
        self.phosphor = None;
        self.settle = phosphor::Settle::default();
        self.crossing_gate.reset();
    }

    /// Steps camera smoothing, the colour field and the mic layers for a
    /// frame that is not drawn, at a target of `size` pixels.
    ///
    /// Advances scene, camera, debounce, crossing-gate and mic-layer state
    /// without drawing a frame. Capture replay uses this for stateful scenes.
    ///
    /// `elapsed` is f64 seconds: the mic layers time bursts with it exactly
    /// however long the renderer runs; scenes take it as f32.
    pub fn advance_state(&mut self, frame: ControlFrame, elapsed: f64, size: (u32, u32)) {
        self.observe(&frame, elapsed);
        let (frame, display) = self.dispatch(frame, true, elapsed);
        let (width, height) = size;
        let aspect = size_aspect(size);
        let (mut instances, camera) = match display {
            colour_field::Display::ColourField(trace) => {
                let mut instances = Vec::new();
                colour_field::build(&trace, width, height, self.srgb_target, &mut instances);
                (instances, CameraMode::ScreenSpace)
            }
            colour_field::Display::Native(_) => (
                build_instances_for_viewport(
                    &self.profile,
                    frame,
                    elapsed as f32,
                    aspect,
                    size,
                    &self.scene_state,
                ),
                scenes::select(frame.scene).camera,
            ),
        };
        prepare_starfield_attachment_colors(
            &mut instances,
            self.srgb_target,
            is_starfield_display(display),
        );
        let uniforms = scene_uniforms(
            camera,
            &mut self.camera,
            &self.profile,
            frame,
            elapsed as f32,
            aspect,
        );
        if let Some(count) = self.prepare_mic_layers(frame, display, elapsed, size) {
            let mut encoder = self.command_encoder();
            let phosphor = self.phosphor.take().expect("prepared above");
            self.encode_layers(&mut encoder, phosphor.scene(), &instances, &uniforms);
            self.phosphor = Some(phosphor);
            self.encode_phosphor(&mut encoder, None, count, elapsed);
            self.queue.submit(std::iter::once(encoder.finish()));
        }
    }

    /// Sets the mic layers' mode without applying the build gate. Callers
    /// outside tests must filter the mode through `mic_layers::allowed()`
    /// first, as `app` and `capture` do: default builds must draw no layers.
    pub fn set_mic_layers(&mut self, mode: mic_layers::Mode) {
        self.mic_layers.set_mode(mode);
    }

    /// Operator control (`M`): cycles off, scope, bursts, both. Ungated,
    /// like `set_mic_layers`; callers check the build gate first.
    pub fn cycle_mic_layers(&mut self) -> mic_layers::Mode {
        let mode = self.mic_layers.mode().next();
        self.mic_layers.set_mode(mode);
        mode
    }

    /// The mic side channel's snapshot for the next drawn or advanced frame,
    /// or `None` when the frame does not come from the microphone.
    pub fn set_live_scope(&mut self, scope: Option<ScopeSnapshot>) {
        self.live_scope = scope;
    }

    /// Replaces the colour-field configuration and restarts it from power-on.
    pub fn configure_colour_field(&mut self, config: colour_field::Config) {
        self.colour_field = colour_field::ColourField::new(config);
    }

    /// Replaces the registry scenes' options and restarts their state.
    pub fn configure_scenes(&mut self, config: SceneConfig) {
        self.scene_state = SceneState::new(config);
    }

    /// Supplies the optional catalog and observer setup for registry scene 6.
    pub fn configure_starfield(&mut self, config: Option<scenes::starfield::StarfieldConfig>) {
        self.scene_state.configure_starfield(config);
    }

    /// Caps colour-field steps at 60 in any wall-clock second (live window).
    pub fn limit_colour_field_steps(&mut self) {
        self.step_clock = Some((colour_field::StepClock::default(), Instant::now()));
    }

    /// Operator pin control (`L`): cycles the layout pin on the next step.
    pub fn cycle_layout_pin(&mut self) {
        self.colour_controls.pin_cycle = true;
    }

    /// Feeds one received or generated frame to the colour field (onset
    /// detection, and the newest frame for its next step) and the stateful
    /// registry scenes (see `SceneState`). Call once per new
    /// frame, never for a held display repeat; `encode` steps on what was
    /// observed. `elapsed` is the clock `encode` gets (seconds).
    pub fn observe(&mut self, frame: &ControlFrame, elapsed: f64) {
        self.colour_field.observe(frame);
        self.scene_state.observe_at(frame, elapsed);
    }

    /// Previews dispatch without consuming a fallback or recording a
    /// Starfield crossing. Surface acquisition can fail after this query.
    pub fn displayed_scene(&mut self, frame: ControlFrame, elapsed: f64) -> u8 {
        let (_, display) = self.dispatch(frame, false, elapsed);
        displayed_scene_byte(display)
    }

    /// Scene byte of the last successfully encoded/replayed frame.
    pub fn committed_displayed_scene(&self) -> u8 {
        displayed_scene_byte(self.crossing_gate.displayed)
    }

    /// Uses a safe native scene for the next encode if scene dispatch selects
    /// Starfield without a configured catalog.
    pub fn fallback_next_missing_starfield(&mut self) {
        self.next_native_fallback = Some(0);
    }

    /// Scene dispatch: the colour field steps once per new observed frame
    /// and owns scene 7; otherwise the native scene goes through the
    /// registry unchanged. The crossing gate commits only when this frame
    /// will be encoded; `frame` is never observed here.
    fn dispatch(
        &mut self,
        frame: ControlFrame,
        commit: bool,
        elapsed: f64,
    ) -> (ControlFrame, colour_field::Display) {
        let (clock, now) = match &mut self.step_clock {
            Some((clock, start)) => (Some(clock), start.elapsed()),
            None => (None, Duration::ZERO),
        };
        let display = colour_field::step_capped(
            &mut self.colour_field,
            clock,
            now,
            &mut self.colour_controls,
        );
        let (frame, requested, fallback_applied) =
            resolve_dispatch_frame(frame, display, self.next_native_fallback);
        let display = self.crossing_gate.resolve(requested, elapsed, commit);
        let mut frame = frame;
        if let colour_field::Display::Native(scene) = display {
            frame.scene = scene;
        }
        if commit && fallback_applied && display == requested {
            self.next_native_fallback = None;
        }
        (frame, display)
    }

    /// Uploads this frame's instances and uniforms, then records its render pass.
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        frame: ControlFrame,
        elapsed: f64,
        size: (u32, u32),
    ) {
        self.encode_frame(encoder, view, frame, elapsed, size, true);
    }

    /// `encode` as it was before the mic layers: neither steps nor draws
    /// them. The baseline that layers `off` must match.
    #[cfg(test)]
    pub(crate) fn encode_without_mic_layers(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        frame: ControlFrame,
        elapsed: f64,
        size: (u32, u32),
    ) {
        self.encode_frame(encoder, view, frame, elapsed, size, false);
    }

    fn encode_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        frame: ControlFrame,
        elapsed: f64,
        size: (u32, u32),
        mic_layers: bool,
    ) {
        let overlay_elapsed = elapsed;
        let elapsed = elapsed as f32;
        let (width, height) = size;
        let aspect = size_aspect(size);
        let (frame, display) = self.dispatch(frame, true, overlay_elapsed);
        let (mut instances, camera) = match display {
            colour_field::Display::ColourField(trace) => {
                let mut instances = Vec::new();
                colour_field::build(&trace, width, height, self.srgb_target, &mut instances);
                (instances, CameraMode::ScreenSpace)
            }
            colour_field::Display::Native(_) => (
                build_instances_for_viewport(
                    &self.profile,
                    frame,
                    elapsed,
                    aspect,
                    size,
                    &self.scene_state,
                ),
                scenes::select(frame.scene).camera,
            ),
        };
        prepare_starfield_attachment_colors(
            &mut instances,
            self.srgb_target,
            is_starfield_display(display),
        );
        let uniforms = scene_uniforms(
            camera,
            &mut self.camera,
            &self.profile,
            frame,
            elapsed,
            aspect,
        );
        let glow = if mic_layers {
            self.prepare_mic_layers(frame, display, overlay_elapsed, size)
        } else {
            None
        };
        match glow {
            // The scene goes into the phosphor's copy, which the composite
            // writes to the acquired surface with the layers added.
            Some(count) => {
                let phosphor = self.phosphor.take().expect("prepared above");
                self.encode_layers(encoder, phosphor.scene(), &instances, &uniforms);
                self.phosphor = Some(phosphor);
                self.encode_phosphor(encoder, Some(view), count, overlay_elapsed);
            }
            None => self.encode_layers(encoder, view, &instances, &uniforms),
        }
    }

    /// Steps the mic layers and uploads their instances. Returns the
    /// instance count when the phosphor passes should run this frame: while
    /// the layers draw or the phosphor still glows. With nothing drawn and a
    /// dark phosphor (always, for layers `off`) no pass runs and the
    /// phosphor's textures are freed. Entering the colour field or a resize
    /// drops it at once and holds the layers dark while the dropped light
    /// would have decayed (and, for a resize, while the target settles; see
    /// `phosphor::Settle::plan`).
    fn prepare_mic_layers(
        &mut self,
        frame: ControlFrame,
        display: colour_field::Display,
        elapsed: f64,
        size: (u32, u32),
    ) -> Option<u32> {
        let aspect = size_aspect(size);
        // While a resize settles or a dropped afterglow is held dark, nothing
        // drawn would reach the screen, so the layers step as hidden: no
        // burst starts that would first appear part way through its rise.
        let drawable = self.settle.can_draw(
            self.phosphor.as_ref().map(phosphor::Phosphor::size),
            size,
            elapsed,
        );
        let shown = mic_layers_shown(self.mic_layers.mode(), display) && drawable;
        self.mic_layers
            .step(frame, self.live_scope.as_ref(), elapsed, shown);
        // The colour field must follow its trace alone, and Starfield keeps
        // its per-pixel luminance bound without an overlay or lingering
        // afterglow. The drop holds layers dark as a resize's does.
        if matches!(display, colour_field::Display::ColourField(_)) || is_starfield_display(display)
        {
            if let Some(glow) = self.phosphor.take() {
                self.settle.drop_lit(glow.fading(), elapsed);
            }
            return None;
        }
        let mut overlay = Vec::new();
        if shown {
            self.mic_layers.build(frame, aspect, &mut overlay);
        }
        let count = overlay.len().min(mic_layers::MAX_INSTANCES);
        let held = self
            .phosphor
            .as_ref()
            .map(|glow| (glow.size(), glow.fading()));
        match self.settle.plan(held, count, size, elapsed) {
            phosphor::Plan::Idle => {
                self.phosphor = None;
                return None;
            }
            phosphor::Plan::Make => {
                self.phosphor = Some(phosphor::Phosphor::new(
                    &self.device,
                    &self.phosphor_pipelines,
                    size,
                ));
            }
            phosphor::Plan::Draw => {}
        }
        if count > 0 {
            self.queue.write_buffer(
                &self.instance_buffer,
                self.overlay_offset(),
                bytemuck::cast_slice(&overlay[..count]),
            );
            // Flat screen-space instances use no other uniform.
            let flat = Uniforms {
                view_projection: mic_layers::transform(aspect),
                time: 0.0,
                energy: 0.0,
                beat: 0.0,
                screen_space: 1.0,
            };
            self.queue
                .write_buffer(&self.overlay_uniform_buffer, 0, bytemuck::bytes_of(&flat));
        }
        Some(count as u32)
    }

    /// Records the phosphor passes for `count` prepared overlay instances,
    /// compositing onto `view` (with the scene drawn into the phosphor's
    /// copy first) when given.
    fn encode_phosphor(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: Option<&wgpu::TextureView>,
        count: u32,
        elapsed: f64,
    ) {
        let overlay = phosphor::Overlay {
            pipeline: &self.overlay_pipeline,
            uniforms: &self.overlay_bind_group,
            quad: &self.vertex_buffer,
            instances: self.instance_buffer.slice(self.overlay_offset()..),
            count,
        };
        self.phosphor.as_mut().expect("prepared").encode(
            encoder,
            &self.queue,
            &self.phosphor_pipelines,
            overlay,
            elapsed,
            view,
        );
    }

    /// Where the overlay's instances start in the instance buffer.
    fn overlay_offset(&self) -> u64 {
        (self.instance_capacity * std::mem::size_of::<Instance>()) as u64
    }

    /// What the latest colour-field step shows.
    pub fn colour_field_display(&self) -> colour_field::Display {
        self.colour_field.display()
    }

    /// Uploads explicit instances and uniforms, then records the render pass.
    #[cfg(test)]
    pub(crate) fn encode_instances(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        instances: &[Instance],
        uniforms: &Uniforms,
    ) {
        self.encode_layers(encoder, view, instances, uniforms);
    }

    /// Uploads the scene's instances and uniforms and records their pass.
    fn encode_layers(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        instances: &[Instance],
        uniforms: &Uniforms,
    ) {
        let instance_count = instances.len().min(self.instance_capacity);
        self.queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&instances[..instance_count]),
        );
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("chipviz render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.draw(0..QUAD_VERTICES.len() as u32, 0..instance_count as u32);
    }

    fn command_encoder(&self) -> wgpu::CommandEncoder {
        self.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("chipviz render encoder"),
            })
    }
}

impl Renderer {
    pub async fn new(
        window: Arc<winit::window::Window>,
        profile: RenderProfile,
    ) -> Result<Self, RendererError> {
        let size = window.inner_size();
        let instance = gpu_instance();
        let surface = instance.create_surface(window.clone())?;
        let (adapter, device, queue) = request_device(&instance, &profile, Some(&surface)).await?;
        let surface_capabilities = surface.get_capabilities(&adapter);
        let format = surface_capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| surface_capabilities.formats.first().copied())
            .ok_or(RendererError::NoSurfaceFormat)?;
        let present_mode = surface_capabilities
            .present_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::PresentMode::Mailbox)
            .unwrap_or(wgpu::PresentMode::Fifo);
        let config = surface_config(format, present_mode, &surface_capabilities, size);
        surface.configure(&device, &config);
        let pipeline = ScenePipeline::new(device, queue, format, profile, aspect(size));

        Ok(Self {
            window,
            surface,
            config,
            pipeline,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.pipeline.device, &self.config);
    }

    pub fn pipeline_mut(&mut self) -> &mut ScenePipeline {
        &mut self.pipeline
    }

    pub fn resize_to_current_window(&mut self) {
        let size = self.window.inner_size();
        self.resize(size.width, size.height);
    }

    /// Draws `frame`. The caller feeds new frames to the colour field with
    /// [`ScenePipeline::observe`] first, so held redraws repeat its last step.
    pub fn render(&mut self, frame: ControlFrame, elapsed: f64) -> Result<(), RendererError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let size = (self.config.width, self.config.height);
        let mut encoder = self.pipeline.command_encoder();
        self.pipeline
            .encode(&mut encoder, &view, frame, elapsed, size);
        self.pipeline
            .queue
            .submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }
}

impl OffscreenRenderer {
    /// Uses a headless adapter: no window, surface or screen-recording access.
    pub async fn new(
        profile: RenderProfile,
        width: u32,
        height: u32,
    ) -> Result<Self, RendererError> {
        Self::with_format(profile, width, height, OFFSCREEN_FORMAT).await
    }

    /// As [`OffscreenRenderer::new`], drawing into a target of `format` (4
    /// bytes per pixel, RGBA order), as a surface of that format would.
    pub(crate) async fn with_format(
        profile: RenderProfile,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<Self, RendererError> {
        let (width, height) = (width.max(1), height.max(1));
        let (_, device, queue) = request_device(&gpu_instance(), &profile, None).await?;
        check_target_size(width, height, &device.limits())?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("chipviz offscreen target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let padded_row = padded_row_bytes(width);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chipviz offscreen readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let aspect = width as f32 / height as f32;
        let pipeline = ScenePipeline::new(device, queue, format, profile, aspect);

        Ok(Self {
            pipeline,
            width,
            height,
            padded_row,
            texture,
            view,
            readback,
        })
    }

    #[cfg(test)]
    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn pipeline_mut(&mut self) -> &mut ScenePipeline {
        &mut self.pipeline
    }

    /// Draws one new frame (observed by the colour field first) and returns
    /// tightly packed RGB8 rows, top row first.
    ///
    /// The colour field steps once per new `frame` counter value. A frame
    /// whose counter equals the previously rendered one does not step, so
    /// its scene and colour-field state stay those of the earlier frame;
    /// give each frame a distinct counter (or call
    /// [`ScenePipeline::reset_state`]) when rendering unrelated frames.
    pub fn render(&mut self, frame: ControlFrame, elapsed: f64) -> Result<Vec<u8>, RendererError> {
        self.pipeline.observe(&frame, elapsed);
        let size = (self.width, self.height);
        self.render_with(|pipeline, encoder, view| {
            pipeline.encode(encoder, view, frame, elapsed, size);
        })
    }

    /// Records `draw` into the offscreen target, then reads back RGB8 rows.
    pub(crate) fn render_with(
        &mut self,
        draw: impl FnOnce(&mut ScenePipeline, &mut wgpu::CommandEncoder, &wgpu::TextureView),
    ) -> Result<Vec<u8>, RendererError> {
        let mut encoder = self.pipeline.command_encoder();
        draw(&mut self.pipeline, &mut encoder, &self.view);
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self.readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.pipeline
            .queue
            .submit(std::iter::once(encoder.finish()));

        let slice = self.readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.pipeline.device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .unwrap_or(Err(wgpu::BufferAsyncError))
            .map_err(RendererError::Readback)?;
        let mut rgb = Vec::with_capacity(self.width as usize * self.height as usize * 3);
        {
            let data = slice.get_mapped_range();
            for row in data.chunks_exact(self.padded_row as usize) {
                for pixel in row[..self.width as usize * 4].chunks_exact(4) {
                    rgb.extend_from_slice(&pixel[..3]);
                }
            }
        }
        self.readback.unmap();
        Ok(rgb)
    }
}

fn padded_row_bytes(width: u32) -> u32 {
    (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// Rejects capture sizes the device cannot allocate, instead of letting wgpu
/// validation panic.
fn check_target_size(width: u32, height: u32, limits: &wgpu::Limits) -> Result<(), RendererError> {
    let max_dimension = limits.max_texture_dimension_2d;
    let fits = width <= max_dimension
        && height <= max_dimension
        && u64::from(padded_row_bytes(width)) * u64::from(height) <= limits.max_buffer_size;
    if fits {
        Ok(())
    } else {
        Err(RendererError::TargetTooLarge {
            width,
            height,
            max_dimension,
        })
    }
}

fn surface_config(
    format: wgpu::TextureFormat,
    present_mode: wgpu::PresentMode,
    capabilities: &wgpu::SurfaceCapabilities,
    size: PhysicalSize<u32>,
) -> wgpu::SurfaceConfiguration {
    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode,
        alpha_mode: capabilities
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto),
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    }
}

fn aspect(size: PhysicalSize<u32>) -> f32 {
    size_aspect((size.width, size.height))
}

fn size_aspect((width, height): (u32, u32)) -> f32 {
    width.max(1) as f32 / height.max(1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_size_check_follows_device_limits() {
        let limits = wgpu::Limits::default();
        let max = limits.max_texture_dimension_2d;
        for (width, height) in [(1, 1), (1920, 1080), (3840, 2160), (max, max)] {
            assert!(
                check_target_size(width, height, &limits).is_ok(),
                "{width}x{height}"
            );
        }
        for (width, height) in [(max + 1, 10), (9000, 10), (10, max + 1)] {
            let error = check_target_size(width, height, &limits).unwrap_err();
            assert!(error.to_string().contains(&format!("{width}x{height}")));
        }
        let tight = wgpu::Limits {
            max_buffer_size: 1024,
            ..limits
        };
        assert!(check_target_size(64, 4, &tight).is_ok());
        assert!(check_target_size(64, 5, &tight).is_err());
    }

    #[test]
    fn crossing_gate_limits_starfield_reversals_and_leaves_native_changes_alone() {
        let mut gate = StarfieldCrossingGate::default();
        let starfield = colour_field::Display::Native(13);
        let native = colour_field::Display::Native(0);
        assert_eq!(gate.resolve(starfield, 0.0, true), starfield);
        assert_eq!(gate.resolve(native, 0.1, true), native);

        // A third transition inside the rolling second is denied. Preview is
        // non-consuming, and changes between native scenes remain immediate.
        assert_eq!(gate.resolve(starfield, 0.5, true), native);
        assert_eq!(gate.resolve(starfield, 0.6, false), native);
        assert_eq!(gate.displayed, native);
        assert_eq!(
            gate.resolve(colour_field::Display::Native(4), 0.7, true),
            colour_field::Display::Native(4)
        );
        assert_eq!(
            gate.resolve(starfield, 1.100_01, true),
            starfield,
            "old crossings expire only after the closed one-second boundary"
        );

        let colour = colour_field::Display::ColourField(colour_field::Trace {
            mode: 0,
            family: 0,
            layout: 0,
            combo: 0,
            rot: 0,
            lvl: [0; 3],
        });
        assert_eq!(gate.resolve(colour, 1.2, true), colour);
        assert_eq!(gate.resolve(starfield, 1.21, true), colour);
        assert_eq!(gate.resolve(starfield, 2.200_02, true), starfield);
    }

    #[test]
    fn optional_mic_layers_are_not_composited_over_starfield() {
        let mode = mic_layers::Mode::Both;
        assert!(mic_layers_shown(mode, colour_field::Display::Native(4)));
        assert!(!mic_layers_shown(mode, colour_field::Display::Native(6)));
        assert!(!mic_layers_shown(mode, colour_field::Display::Native(13)));
        assert!(!mic_layers_shown(
            mode,
            colour_field::Display::ColourField(colour_field::Trace {
                mode: 0,
                family: 0,
                layout: 0,
                combo: 0,
                rot: 0,
                lvl: [0; 3],
            })
        ));
    }

    #[test]
    fn fallback_is_not_counted_as_a_starfield_display_and_preserves_frame_events() {
        let mut field = colour_field::ColourField::new(colour_field::Config::default());
        let frame = ControlFrame {
            scene: 13,
            flags: 0x05,
            beat_count: 7,
            ..ControlFrame::procedural(9)
        };
        // Match live dispatch: observe before previewing the frame.
        field.observe(&frame);
        if field.has_new_frame() {
            field.step(colour_field::Controls::default());
        }
        let fallback = Some(0);
        let (preview_frame, preview, fallback_applied) =
            resolve_dispatch_frame(frame, field.display(), fallback);
        assert!(fallback_applied);
        assert_eq!(preview_frame.flags, frame.flags);
        assert_eq!(preview_frame.beat_count, frame.beat_count);
        assert_eq!(preview, colour_field::Display::Native(0));
        assert!(!is_starfield_display(preview));

        let mut gate = StarfieldCrossingGate::default();
        assert_eq!(gate.resolve(preview, 0.0, false), preview);
        assert_eq!(gate.displayed, colour_field::Display::Native(0));
        // A surface-acquisition failure commits neither the fallback nor a
        // crossing. The successful retry observes the same input first.
        assert_eq!(fallback, Some(0));
        field.observe(&frame);
        if field.has_new_frame() {
            field.step(colour_field::Controls::default());
        }
        let (_, retry, applied) = resolve_dispatch_frame(frame, field.display(), fallback);
        let committed = gate.resolve(retry, 0.0, true);
        assert!(applied);
        assert_eq!(committed, colour_field::Display::Native(0));

        assert!(is_starfield_display(colour_field::Display::Native(13)));
        assert!(!is_starfield_display(colour_field::Display::Native(0)));
        assert_eq!(scenes::dispatch(7).name(), "ColourField");
    }

    #[test]
    fn faint_star_unorm_is_encoded_before_attachment_quantization() {
        let linear = [0.01, 0.02, 0.05];
        let make_instance = || Instance {
            center: [0.0; 3],
            size: [0.0; 2],
            color: [linear[0], linear[1], linear[2], 1.0],
            rotation: 0.0,
            scene_mix: 0.0,
            pattern: [0.0; 2],
        };
        let mut instance = make_instance();
        prepare_starfield_attachment_colors(std::slice::from_mut(&mut instance), false, true);
        for (encoded, expected_linear) in instance.color[..3].iter().zip(linear) {
            let code = (encoded * 255.0).round() as u8;
            assert!(code > 0, "faint channel was rounded to black");
            let readback = f32::from(code) / 255.0;
            let decoded = if readback <= 0.04045 {
                readback / 12.92
            } else {
                ((readback + 0.055) / 1.055).powf(2.4)
            };
            assert!((decoded - expected_linear).abs() < 0.002);
        }

        let mut srgb_instance = make_instance();
        prepare_starfield_attachment_colors(std::slice::from_mut(&mut srgb_instance), true, true);
        assert_eq!(&srgb_instance.color[..3], &linear);

        let mut unrelated = make_instance();
        prepare_starfield_attachment_colors(std::slice::from_mut(&mut unrelated), false, false);
        assert_eq!(&unrelated.color[..3], &linear);
    }

    #[test]
    fn readback_rows_are_padded_to_copy_alignment() {
        assert_eq!(padded_row_bytes(64), 256);
        assert_eq!(padded_row_bytes(100), 512);
        assert_eq!(padded_row_bytes(1920), 7680);
    }

    /// The GPU composite matches its CPU model (`phosphor::composite_pixel`,
    /// in linear light) within 8-bit rounding for a grid of scene and
    /// display colours, on an sRGB target and on one that is not sRGB
    /// (shown as sRGB-coded, so the composite decodes and encodes itself),
    /// and leaves the scene byte for byte where the display is black.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn phosphor_composite_matches_its_model_on_either_target() {
        let decode = |byte: u8| {
            let value = f32::from(byte) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        let encode = |linear: f32| {
            let value = if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            (value * 255.0).round()
        };
        let levels = [0_u8, 30, 90, 160, 220, 255];
        let colours: Vec<[u8; 3]> = (0..216)
            .map(|index| [levels[index % 6], levels[index / 6 % 6], levels[index / 36]])
            .collect();
        let side = colours.len() as u32;
        let (mut scene, mut display) = (Vec::new(), Vec::new());
        for row in &colours {
            for column in &colours {
                scene.extend_from_slice(&[column[0], column[1], column[2], 255]);
                display.extend_from_slice(&[row[0], row[1], row[2], 255]);
            }
        }
        for format in [
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Rgba8Unorm,
        ] {
            let mut renderer = pollster::block_on(OffscreenRenderer::with_format(
                RenderProfile::pi5(),
                side,
                side,
                format,
            ))
            .unwrap();
            let pipeline = renderer.pipeline_mut();
            let glow = phosphor::Phosphor::new(
                &pipeline.device,
                &pipeline.phosphor_pipelines,
                (side, side),
            );
            glow.fill(&pipeline.queue, &scene, &display);
            let drawn = renderer
                .render_with(|pipeline, encoder, view| {
                    glow.composite_only(
                        encoder,
                        &pipeline.queue,
                        &pipeline.phosphor_pipelines,
                        view,
                    );
                })
                .unwrap();
            for (pixel, got) in drawn.chunks_exact(3).enumerate() {
                let stored = &scene[pixel * 4..pixel * 4 + 3];
                let lit = &display[pixel * 4..pixel * 4 + 3];
                if lit == [0, 0, 0] {
                    assert_eq!(got, stored, "{format:?} pixel {pixel}: scene changed");
                    continue;
                }
                let out = phosphor::composite_pixel(
                    std::array::from_fn(|channel| decode(stored[channel])),
                    std::array::from_fn(|channel| f32::from(lit[channel]) / 255.0),
                );
                for (channel, &value) in got.iter().enumerate() {
                    let error = (f32::from(value) - encode(out[channel])).abs();
                    assert!(
                        error <= 1.0,
                        "{format:?} scene {stored:?} display {lit:?}: got {got:?}, model {out:?}"
                    );
                }
            }
        }
    }
}
