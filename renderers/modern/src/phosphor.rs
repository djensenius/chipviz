//! Per-pixel phosphor for the mic layers (`scenes::mic_layers`).
//!
//! The layers are drawn into their own target (the emitted light), then a
//! persistent display texture follows it per channel in linear light:
//! `display = max(emitted, previous - amount)`, where the frames' amounts
//! add up to at most [`DECAY_PER_S`] per second. Rises are instant; falls
//! leave an afterglow, like a long-persistence oscilloscope. While the
//! layers glow the scene is drawn into a texture of its own, and a last
//! pass writes `min(scene + display, 1)` per channel, in linear light, to
//! the screen.
//!
//! The layers' added light, `min(scene + display, 1) - scene`, depends on
//! the scene only through that saturation, so it is 1-Lipschitz in the
//! linear scene: a scene change of `d` in a channel alters the added light
//! in that channel by at most `d` (C = 1). There is no ratio or step of
//! the scene anywhere, so no small scene change is amplified.
//!
//! Luminance flashes, for any input, frame rate or geometry: a WCAG 2.3.1
//! flash is a pair of opposing changes, each of at least 0.1 relative
//! luminance. Relative luminance is a convex combination of the linear
//! channels, and over a still scene each channel of the added light falls
//! no faster than the display's, so the added luminance does too. Three
//! flashes in a 1 s window need three falls of 0.1 ending in it, and the
//! last two lie wholly inside it (each starts after the rise that follows
//! the one before): 0.2 of falls. With the decay carried in whole grid
//! steps any span of `t` seconds falls by at most `DECAY_PER_S * t + 1/255`,
//! so those falls take at least `(0.2 - 2/255) / 0.15` = 1.28 s. So the
//! added light makes at most 2 flashes in any 1 s at any point, with 28 %
//! headroom for output rounding and frame timing. (At 0.25 per second the
//! two falls fit in 0.8 s, and a pixel can reach 3.) Where the scene moves,
//! the added light also changes by up to the scene's own change (above).
//!
//! Red flashes are not bounded. They need a saturated red
//! (`R / (R + G + B) >= 0.8`, linear) at one end of a change of
//! `max(R - G - B, 0)` by more than 20/320, and green or blue rising over a
//! red pixel is as much a red change as red falling. The display keeps
//! green plus blue at least a third of its red (a red share of at most
//! [`RED_SHARE`]), so wherever the scene is not a saturated red the output
//! mixes two colours with red shares below 0.8 and is not one either
//! (saturating red only lowers the share; saturating green or blue leaves
//! at most 1/2). But over a scene pixel that is already a saturated red, a
//! layer that is relit as soon as it has faded makes 3 red flashes in 1 s.
//! Masking the layers out over red scene pixels would need a steep
//! function of the scene, which turns small scene changes into large
//! changes of the layers, so the mic layers are built only with the
//! `unmeasured-flash` feature (see `scenes::mic_layers`).
//!
//! Only the composite reads the scene; the display (and so capture replay)
//! never depends on it. With a target that is not sRGB, the composite
//! decodes and encodes sRGB itself, so the decay holds in linear light on
//! either kind of surface.
//!
//! Nothing here runs while the layers draw nothing and the display is
//! dark, so layers `off` render exactly as before them.

use crate::scene::QUAD_VERTICES;

/// Largest total fall of a display channel per second of overlay time.
pub const DECAY_PER_S: f32 = 0.15;
/// The display holds multiples of `1 / GRID` in `0..=1`, which [`FORMAT`]
/// stores exactly, so its falls are exactly the decay amounts.
/// Keep in sync with `GRID` in phosphor.wgsl.
pub const GRID: f32 = 255.0;
/// Longest frame interval counted, so a stall decays no faster than this.
pub const MAX_DT_S: f64 = 0.1;
/// Largest red share (`R / (R + G + B)`) of the display; WCAG's saturated
/// red starts at 0.8. The display's floor (green plus blue at least a
/// third of red) in phosphor.wgsl and `decay_pixel` is this share.
/// Only the tests read it now that the composite has no red mask.
#[cfg_attr(not(test), allow(dead_code))]
pub const RED_SHARE: f32 = 0.75;
/// How long a resized target must hold still before the phosphor is made
/// again at the new size (overlay seconds).
pub const SETTLE_S: f64 = 0.15;
/// Emitted and display textures: linear, blendable, exact on the grid.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Per-frame decay amounts from overlay time, in whole steps of `1 / GRID`,
/// carrying the remainder, so any span of `t` seconds decays by at most
/// `DECAY_PER_S * t + 1 / GRID` and never stalls at high frame rates.
#[derive(Clone, Copy, Debug)]
pub struct DecayClock {
    rate: f32,
    carry: f32,
    last: Option<f64>,
}

impl Default for DecayClock {
    fn default() -> Self {
        Self::with_rate(DECAY_PER_S)
    }
}

impl DecayClock {
    pub fn with_rate(rate: f32) -> Self {
        Self {
            rate,
            carry: 0.0,
            last: None,
        }
    }

    /// The decay, in grid steps, for a frame at overlay time `now` (s).
    pub fn tick(&mut self, now: f64) -> u32 {
        let dt = self
            .last
            .map_or(0.0, |last| (now - last).clamp(0.0, MAX_DT_S));
        self.last = Some(now);
        let steps = dt as f32 * self.rate * GRID + self.carry;
        let whole = steps.floor();
        self.carry = steps - whole;
        whole as u32
    }
}

/// Waits for a resized target to hold still, so an interactive drag does
/// not reallocate the phosphor every frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Settle {
    pending: Option<((u32, u32), f64)>,
}

impl Settle {
    /// Called when no phosphor matches `size` at overlay time `now`;
    /// `resized` when one of another size was just dropped. Returns whether
    /// to make one now: at once the first time, otherwise once the size
    /// has stayed the same for `SETTLE_S`.
    pub fn ready(&mut self, resized: bool, size: (u32, u32), now: f64) -> bool {
        match self.pending {
            None if !resized => true,
            Some((pending, since)) if !resized && pending == size => {
                let ready = now - since >= SETTLE_S;
                if ready {
                    self.pending = None;
                }
                ready
            }
            _ => {
                self.pending = Some((size, now));
                false
            }
        }
    }
}

/// One display pixel's next value from the previous one, the emitted
/// light and the frame's decay steps; the CPU model of `fs_decay` in
/// phosphor.wgsl. Values are linear, `0..=1`.
#[cfg(test)]
pub fn decay_pixel(previous: [f32; 3], emitted: [f32; 3], amount: u32) -> [f32; 3] {
    let mut out = [0.0_f32; 3];
    for ((out, previous), emitted) in out.iter_mut().zip(previous).zip(emitted) {
        let lit = (emitted.clamp(0.0, 1.0) * GRID + 0.5).floor();
        let held = ((previous * GRID).round() - amount as f32).max(0.0);
        *out = lit.max(held);
    }
    let [red, green, blue] = out;
    let deficit = (((red + 2.5) / 3.0).floor() - green - blue).max(0.0);
    [
        red,
        green + (deficit / 2.0).ceil(),
        blue + (deficit / 2.0).floor(),
    ]
    .map(|value| value / GRID)
}

/// The output for a scene pixel and display pixel, both linear; the CPU
/// model of `fs_composite` in phosphor.wgsl.
#[cfg(test)]
pub fn composite_pixel(scene: [f32; 3], display: [f32; 3]) -> [f32; 3] {
    if display == [0.0; 3] {
        return scene;
    }
    std::array::from_fn(|channel| (scene[channel] + display[channel]).min(1.0))
}

/// Pipelines and the bind group layout, created once per [`ScenePipeline`]
/// (cheap; the textures are only made when the layers first draw).
///
/// [`ScenePipeline`]: crate::renderer::ScenePipeline
pub struct Pipelines {
    decay: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// `[decay steps, 1 when the composite encodes sRGB itself, 0, 0]`.
    params: wgpu::Buffer,
    /// The scene target's format, which the scene copy uses too.
    format: wgpu::TextureFormat,
}

impl Pipelines {
    /// `format` is the scene target's, which the composite writes.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chipviz phosphor shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("phosphor.wgsl").into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chipviz phosphor bind group layout"),
            entries: &[
                texture(0),
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("chipviz phosphor pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let fullscreen = |label, entry_point, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_fullscreen",
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point,
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            })
        };
        let decay = fullscreen("chipviz phosphor decay", "fs_decay", FORMAT);
        let composite = fullscreen("chipviz phosphor composite", "fs_composite", format);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chipviz phosphor parameters"),
            size: std::mem::size_of::<[f32; 4]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            decay,
            composite,
            layout,
            params,
            format,
        }
    }

    /// Sets this frame's decay `steps`, with the fixed parameters.
    fn write_params(&self, queue: &wgpu::Queue, steps: u32) {
        // The composite encodes sRGB itself when the target does not.
        let encode = if self.format.is_srgb() { 0.0 } else { 1.0 };
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::cast_slice(&[steps as f32, encode, 0.0, 0.0]),
        );
    }
}

/// The mic layers' draw for one frame: `count` instances, already uploaded
/// to `instances` with their uniforms bound by `uniforms`.
pub struct Overlay<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub uniforms: &'a wgpu::BindGroup,
    pub quad: &'a wgpu::Buffer,
    pub instances: wgpu::BufferSlice<'a>,
    pub count: u32,
}

/// The emitted target, ping-ponged display textures and scene copy for one
/// size.
pub struct Phosphor {
    size: (u32, u32),
    /// Kept for tests that fill the scene copy and display directly.
    #[cfg(test)]
    textures: [wgpu::Texture; 3],
    emitted: wgpu::TextureView,
    scene: wgpu::TextureView,
    /// `decay_groups[i]` reads `emitted` and `displays[i]`.
    decay_groups: [wgpu::BindGroup; 2],
    /// `composite_groups[i]` reads `scene` and `displays[i]`.
    composite_groups: [wgpu::BindGroup; 2],
    displays: [wgpu::TextureView; 2],
    /// The display holding the latest frame.
    current: usize,
    clock: DecayClock,
    /// Decay steps since the layers last drew: from `GRID` the display is
    /// black (the red floor never lifts green or blue above red).
    faded: u32,
}

impl Phosphor {
    /// A dark display at `size` (new textures are zeroed).
    pub fn new(device: &wgpu::Device, pipelines: &Pipelines, size: (u32, u32)) -> Self {
        let texture = |label, format| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };
        let (_, emitted) = texture("chipviz phosphor emitted", FORMAT);
        let (scene_texture, scene) = texture("chipviz phosphor scene", pipelines.format);
        let (display_texture_0, display_0) = texture("chipviz phosphor display 0", FORMAT);
        let (display_texture_1, display_1) = texture("chipviz phosphor display 1", FORMAT);
        #[cfg(not(test))]
        let _ = (scene_texture, display_texture_0, display_texture_1);
        let displays = [display_0, display_1];
        let group = |first: &wgpu::TextureView, display: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("chipviz phosphor bind group"),
                layout: &pipelines.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(first),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(display),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: pipelines.params.as_entire_binding(),
                    },
                ],
            })
        };
        let decay_groups = std::array::from_fn(|index| group(&emitted, &displays[index]));
        let composite_groups = std::array::from_fn(|index| group(&scene, &displays[index]));
        Self {
            size,
            #[cfg(test)]
            textures: [scene_texture, display_texture_0, display_texture_1],
            emitted,
            scene,
            decay_groups,
            composite_groups,
            displays,
            current: 0,
            clock: DecayClock::default(),
            faded: GRID as u32,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Every pixel is black, so skipping the passes draws the same.
    pub fn is_dark(&self) -> bool {
        self.faded >= GRID as u32
    }

    /// Where to draw the scene for a frame that [`Phosphor::encode`] then
    /// composites onto the screen.
    pub fn scene(&self) -> &wgpu::TextureView {
        &self.scene
    }

    /// Records one frame at overlay time `now`: draws `overlay` as the
    /// emitted light and decays the display towards it, then, when `view`
    /// is given, writes the scene (drawn into [`Phosphor::scene`] first)
    /// with the display added onto `view`. A frame that is replayed but not
    /// drawn still advances the display.
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        overlay: Overlay<'_>,
        now: f64,
        view: Option<&wgpu::TextureView>,
    ) {
        let count = overlay.count;
        let amount = self.clock.tick(now);
        self.faded = if count > 0 {
            0
        } else {
            self.faded.saturating_add(amount)
        };
        pipelines.write_params(queue, amount);
        {
            let mut pass = begin(encoder, "chipviz phosphor emit", &self.emitted);
            if count > 0 {
                pass.set_pipeline(overlay.pipeline);
                pass.set_bind_group(0, overlay.uniforms, &[]);
                pass.set_vertex_buffer(0, overlay.quad.slice(..));
                pass.set_vertex_buffer(1, overlay.instances);
                pass.draw(0..QUAD_VERTICES.len() as u32, 0..count);
            }
        }
        let next = 1 - self.current;
        {
            let mut pass = begin(encoder, "chipviz phosphor decay", &self.displays[next]);
            pass.set_pipeline(&pipelines.decay);
            pass.set_bind_group(0, &self.decay_groups[self.current], &[]);
            pass.draw(0..3, 0..1);
        }
        self.current = next;
        if let Some(view) = view {
            self.composite(encoder, pipelines, view);
        }
    }

    /// Writes the scene copy with the current display added onto `view`.
    fn composite(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &Pipelines,
        view: &wgpu::TextureView,
    ) {
        let mut pass = begin(encoder, "chipviz phosphor composite", view);
        pass.set_pipeline(&pipelines.composite);
        pass.set_bind_group(0, &self.composite_groups[self.current], &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
impl Phosphor {
    /// Fills the scene copy and the current display with RGBA8 rows (the
    /// scene's in its target's encoding, the display's linear).
    pub fn fill(&self, queue: &wgpu::Queue, scene: &[u8], display: &[u8]) {
        let (width, height) = self.size;
        for (texture, data) in [
            (&self.textures[0], scene),
            (&self.textures[1 + self.current], display),
        ] {
            queue.write_texture(
                texture.as_image_copy(),
                data,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Records the composite pass alone onto `view`.
    pub fn composite_only(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        view: &wgpu::TextureView,
    ) {
        pipelines.write_params(queue, 0);
        self.composite(encoder, pipelines, view);
    }
}

/// A render pass on `view`, cleared to transparent black (every pass here
/// writes each pixel, or starts the emitted light from nothing).
fn begin<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &str,
    view: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resized_targets_wait_to_settle_before_the_phosphor_is_remade() {
        let mut settle = Settle::default();
        // The first phosphor is made at once.
        assert!(settle.ready(false, (800, 600), 0.0));
        // A drag: a new size every frame, none made meanwhile.
        assert!(!settle.ready(true, (810, 600), 1.0));
        let mut held = 1.0;
        for width in (820..=890).step_by(10) {
            held += 1.0 / 60.0;
            assert!(!settle.ready(false, (width, 600), held), "{width}");
        }
        // Held still: made once `SETTLE_S` has passed at the final size.
        assert!(!settle.ready(false, (890, 600), held + SETTLE_S - 0.01));
        assert!(settle.ready(false, (890, 600), held + SETTLE_S + 0.001));
        assert!(settle.ready(false, (890, 600), held + 1.0));
    }

    #[test]
    fn decay_clock_carries_fractions_of_a_step() {
        for fps in [30.0, 60.0, 144.0, 240.0, 1000.0] {
            let mut clock = DecayClock::default();
            let frames = (fps * 10.0) as usize;
            let total: u32 = (0..=frames)
                .map(|frame| clock.tick(frame as f64 / fps))
                .sum();
            let ideal = DECAY_PER_S * GRID * 10.0;
            assert!(
                (total as f32 - ideal).abs() <= 1.0,
                "{fps} fps: {total} steps in 10 s, ideal {ideal}"
            );
        }
    }

    #[test]
    fn the_display_keeps_its_red_share_at_most_red_share() {
        let mut display = [0.0_f32; 3];
        for step in 0..2000_u32 {
            let seed = step * 3;
            let emitted = if step % 7 < 3 {
                [crate::scenes::hash(seed), 0.0, 0.0]
            } else {
                std::array::from_fn(|channel| crate::scenes::hash(seed + channel as u32))
            };
            let previous = display;
            display = decay_pixel(display, emitted, step % 5);
            let sum: f32 = display.iter().sum();
            assert!(
                sum == 0.0 || display[0] / sum <= RED_SHARE + 1e-6,
                "{display:?}"
            );
            for channel in 0..3 {
                // Falls are at most the decay; everything is on the grid.
                assert!(previous[channel] - display[channel] <= (step % 5) as f32 / GRID + 1e-6);
                let units = display[channel] * GRID;
                assert!((units - units.round()).abs() < 1e-3 && units <= GRID);
            }
        }
    }

    #[test]
    fn the_composite_adds_the_display_and_follows_the_scene_one_to_one() {
        assert_eq!(
            composite_pixel([1.0, 0.0, 0.0], [0.0, 0.9, 0.9]),
            [1.0, 0.9, 0.9]
        );
        assert_eq!(composite_pixel([0.3; 3], [0.9, 0.1, 0.5]), [1.0, 0.4, 0.8]);
        assert_eq!(composite_pixel([0.2, 0.7, 0.1], [0.0; 3]), [0.2, 0.7, 0.1]);
        // C = 1: a scene change of `d` in a channel changes the added light
        // in that channel by at most `d`.
        let added = |scene: [f32; 3], display| {
            let out = composite_pixel(scene, display);
            std::array::from_fn::<f32, 3, _>(|channel| out[channel] - scene[channel])
        };
        for step in 0..20_000_u32 {
            let seed = step * 9;
            let pick = |offset: u32| -> [f32; 3] {
                std::array::from_fn(|channel| crate::scenes::hash(seed + offset + channel as u32))
            };
            let (scene, display, nudge) = (pick(0), pick(3), pick(6));
            let moved: [f32; 3] = std::array::from_fn(|channel| {
                (scene[channel] + 0.05 * (nudge[channel] - 0.5)).clamp(0.0, 1.0)
            });
            let (before, after) = (added(scene, display), added(moved, display));
            for channel in 0..3 {
                let change = (after[channel] - before[channel]).abs();
                assert!(
                    change <= (moved[channel] - scene[channel]).abs() + 1e-6,
                    "{scene:?} -> {moved:?} under {display:?}"
                );
            }
        }
    }
}
