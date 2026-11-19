//! Per-pixel phosphor for the mic layers (`scenes::mic_layers`).
//!
//! The layers are drawn into their own target (the emitted light), then a
//! persistent display texture follows it per channel in linear light:
//! `display = max(emitted, previous - amount)`, where the frames' amounts
//! add up to at most [`DECAY_PER_S`] per second. Rises are instant; falls
//! leave an afterglow, like a long-persistence oscilloscope. The display is
//! added onto the scene (saturating), so the layers' contribution to any
//! pixel falls no faster than the display does.
//!
//! The flash guarantee, for any input, frame rate or geometry: a WCAG 2.3.1
//! flash is a pair of opposing changes, each of at least 0.1 relative
//! luminance. Relative luminance is a convex combination of the linear
//! channels, so it falls no faster than the fastest channel. Three flashes
//! in a 1 s window need three falls of 0.1 ending in it, and the last two
//! lie wholly inside it (each starts after the rise that follows the one
//! before): 0.2 of falls, which take over 1.1 s at [`DECAY_PER_S`]. So
//! there are at most 2 flashes in any 1 s at any point. (At 0.25 per second
//! those two falls fit in 0.8 s, and a pixel can reach 3.) Red flashes also
//! need opposing changes, and every channel's fall is limited alike.
//!
//! Nothing here runs while the layers draw nothing and the display is
//! dark, so layers `off` render exactly as before them.

use crate::scene::QUAD_VERTICES;

/// Largest total fall of a display channel per second of overlay time.
pub const DECAY_PER_S: f32 = 0.18;
/// The display holds multiples of `1 / GRID` in `0..=1`, which
/// [`FORMAT`] stores exactly, so its falls are exactly the decay amounts.
/// Keep in sync with `GRID` in phosphor.wgsl.
pub const GRID: f32 = 1024.0;
/// Longest frame interval counted, so a stall decays no faster than this.
pub const MAX_DT_S: f64 = 0.1;
/// Emitted and display textures: linear, blendable, exact on the grid.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Per-frame decay amounts from overlay time: whole multiples of
/// `1 / GRID`, carrying the remainder, so any span of `t` seconds decays by
/// at most `DECAY_PER_S * t + 1 / GRID` and never stalls at high frame rates.
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

    /// The decay for a frame at overlay time `now` (seconds).
    pub fn tick(&mut self, now: f64) -> f32 {
        let dt = self
            .last
            .map_or(0.0, |last| (now - last).clamp(0.0, MAX_DT_S));
        self.last = Some(now);
        let steps = dt as f32 * self.rate * GRID + self.carry;
        let whole = steps.floor();
        self.carry = steps - whole;
        whole / GRID
    }
}

/// One display channel's next value; the CPU model of `fs_decay` in
/// phosphor.wgsl.
#[cfg(test)]
pub fn decay_pixel(previous: f32, emitted: f32, amount: f32) -> f32 {
    let lit = (emitted.clamp(0.0, 1.0) * GRID + 0.5).floor() / GRID;
    lit.max((previous - amount).max(0.0))
}

/// Pipelines and the bind group layout, created once per [`ScenePipeline`]
/// (cheap; the textures are only made when the layers first draw).
///
/// [`ScenePipeline`]: crate::renderer::ScenePipeline
pub struct Pipelines {
    decay: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    amount: wgpu::Buffer,
}

impl Pipelines {
    /// `format` is the scene target's, which the display is added onto.
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
        let fullscreen = |label, entry_point, format, blend| {
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
                        blend,
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
        let decay = fullscreen("chipviz phosphor decay", "fs_decay", FORMAT, None);
        // final = scene + display (saturating); the scene's alpha is kept.
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let composite = fullscreen(
            "chipviz phosphor composite",
            "fs_composite",
            format,
            Some(additive),
        );
        let amount = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chipviz phosphor decay amount"),
            size: std::mem::size_of::<[f32; 4]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            decay,
            composite,
            layout,
            amount,
        }
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

/// The emitted target and ping-ponged display textures for one size.
pub struct Phosphor {
    size: (u32, u32),
    emitted: wgpu::TextureView,
    displays: [wgpu::TextureView; 2],
    /// `bind_groups[i]` reads `emitted` and `displays[i]`.
    bind_groups: [wgpu::BindGroup; 2],
    /// The display holding the latest frame.
    current: usize,
    clock: DecayClock,
    /// Total decay since the layers last drew: from 1.0 the display is black.
    faded: f32,
}

impl Phosphor {
    /// A dark display at `size` (new textures are zeroed).
    pub fn new(device: &wgpu::Device, pipelines: &Pipelines, size: (u32, u32)) -> Self {
        let texture = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0.max(1),
                        height: size.1.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let emitted = texture("chipviz phosphor emitted");
        let displays = [
            texture("chipviz phosphor display 0"),
            texture("chipviz phosphor display 1"),
        ];
        let bind_groups = std::array::from_fn(|index| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("chipviz phosphor bind group"),
                layout: &pipelines.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&emitted),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&displays[index]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: pipelines.amount.as_entire_binding(),
                    },
                ],
            })
        });
        Self {
            size,
            emitted,
            displays,
            bind_groups,
            current: 0,
            clock: DecayClock::default(),
            faded: 1.0,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Every pixel is black, so skipping the passes draws the same.
    pub fn is_dark(&self) -> bool {
        self.faded >= 1.0
    }

    /// Records one frame at overlay time `now`: draws `overlay` as the
    /// emitted light, decays the display towards it, then adds the display
    /// onto `view` when given (a frame that is replayed but not drawn still
    /// advances the display).
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
        self.faded = if count > 0 { 0.0 } else { self.faded + amount };
        queue.write_buffer(
            &pipelines.amount,
            0,
            bytemuck::cast_slice(&[amount, 0.0, 0.0, 0.0]),
        );
        {
            let mut pass = begin(encoder, "chipviz phosphor emit", &self.emitted, true);
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
            let mut pass = begin(
                encoder,
                "chipviz phosphor decay",
                &self.displays[next],
                true,
            );
            pass.set_pipeline(&pipelines.decay);
            pass.set_bind_group(0, &self.bind_groups[self.current], &[]);
            pass.draw(0..3, 0..1);
        }
        self.current = next;
        if let Some(view) = view {
            let mut pass = begin(encoder, "chipviz phosphor composite", view, false);
            pass.set_pipeline(&pipelines.composite);
            pass.set_bind_group(0, &self.bind_groups[self.current], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// A render pass on `view`, cleared to transparent black or loaded.
fn begin<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &str,
    view: &'a wgpu::TextureView,
    clear: bool,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}
