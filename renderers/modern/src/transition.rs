//! Linear-light per-pixel output slew used only around Starfield transitions.

use bytemuck::{Pod, Zeroable};

const OUTPUT_SLEW_PER_SECOND: f32 = 0.15;
const MAX_SLEW_STEP_SECONDS: f64 = 0.1;
const INITIAL_SLEW_DELTA: f32 = 0.05;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Parameters {
    values: [f32; 4],
}

struct Targets {
    size: (u32, u32),
    _scene_texture: wgpu::Texture,
    scene_view: Option<wgpu::TextureView>,
    _history_textures: [wgpu::Texture; 2],
    history_views: [wgpu::TextureView; 2],
    bind_groups: [wgpu::BindGroup; 2],
}

/// Keeps the last emitted full-frame image and bounds its linear-light change
/// while Starfield is displayed or for a short settling interval after it.
pub struct TransitionOutput {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    parameters: wgpu::Buffer,
    single_pipeline: wgpu::RenderPipeline,
    dual_pipeline: wgpu::RenderPipeline,
    targets: Option<Targets>,
    history_index: usize,
    history_valid: bool,
    last_elapsed: Option<f64>,
    settling_after_starfield: bool,
    settled_slew_credit: f32,
}

impl TransitionOutput {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chipviz Starfield transition shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("transition.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chipviz Starfield transition bind group layout"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
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
        let parameters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chipviz Starfield transition parameters"),
            size: std::mem::size_of::<Parameters>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("chipviz Starfield transition pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let make_pipeline = |label, entry_point, targets| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point,
                    targets,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            })
        };
        let color_target = || {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let single_targets = [color_target()];
        let dual_targets = [color_target(), color_target()];
        let single_pipeline = make_pipeline(
            "chipviz Starfield transition history pipeline",
            "fs_single",
            &single_targets,
        );
        let dual_pipeline = make_pipeline(
            "chipviz Starfield transition output pipeline",
            "fs_dual",
            &dual_targets,
        );
        Self {
            format,
            layout,
            parameters,
            single_pipeline,
            dual_pipeline,
            targets: None,
            history_index: 0,
            history_valid: false,
            last_elapsed: None,
            settling_after_starfield: false,
            settled_slew_credit: 0.0,
        }
    }

    pub fn ensure_targets(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self
            .targets
            .as_ref()
            .is_some_and(|targets| targets.size == size)
        {
            return;
        }
        let texture = |label, extra_usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | extra_usage,
                view_formats: &[],
            })
        };
        let scene_texture = texture(
            "chipviz Starfield transition scene",
            wgpu::TextureUsages::empty(),
        );
        let scene_view = scene_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let history_textures = [
            texture(
                "chipviz Starfield transition history A",
                wgpu::TextureUsages::empty(),
            ),
            texture(
                "chipviz Starfield transition history B",
                wgpu::TextureUsages::empty(),
            ),
        ];
        let history_views = std::array::from_fn(|index| {
            history_textures[index].create_view(&wgpu::TextureViewDescriptor::default())
        });
        let bind_groups = std::array::from_fn(|index| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("chipviz Starfield transition source bind group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&scene_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&history_views[index]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.parameters.as_entire_binding(),
                    },
                ],
            })
        });
        self.targets = Some(Targets {
            size,
            _scene_texture: scene_texture,
            scene_view: Some(scene_view),
            _history_textures: history_textures,
            history_views,
            bind_groups,
        });
        self.history_index = 0;
        self.history_valid = false;
        self.last_elapsed = None;
        self.settling_after_starfield = false;
        self.settled_slew_credit = 0.0;
    }

    pub fn take_scene_view(&mut self) -> wgpu::TextureView {
        self.targets
            .as_mut()
            .expect("transition targets are initialized")
            .scene_view
            .take()
            .expect("scene target is not already borrowed")
    }

    pub fn restore_scene_view(&mut self, view: wgpu::TextureView) {
        self.targets
            .as_mut()
            .expect("transition targets are initialized")
            .scene_view = Some(view);
    }

    pub fn encode_surface(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        elapsed: f64,
        starfield_involved: bool,
    ) {
        self.encode_final(encoder, queue, elapsed, starfield_involved, Some(view));
    }

    pub fn encode_history(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        elapsed: f64,
        starfield_involved: bool,
    ) {
        self.encode_final(encoder, queue, elapsed, starfield_involved, None);
    }

    fn encode_final(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        elapsed: f64,
        starfield_involved: bool,
        output: Option<&wgpu::TextureView>,
    ) {
        let targets = self
            .targets
            .as_ref()
            .expect("transition targets are initialized");
        let initialize_starfield_history = starfield_involved && !self.history_valid;
        if initialize_starfield_history {
            let attachments = [Some(color_attachment(
                &targets.history_views[self.history_index],
            ))];
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chipviz initialize Starfield transition history"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.history_valid = true;
        }
        let next_index = 1 - self.history_index;
        let now = if elapsed.is_finite() {
            elapsed
        } else {
            self.last_elapsed.unwrap_or(0.0)
        };
        let dt = self
            .last_elapsed
            .map_or(0.0, |last| (now - last).clamp(0.0, MAX_SLEW_STEP_SECONDS));
        if starfield_involved {
            self.settling_after_starfield = true;
            self.settled_slew_credit = 0.0;
        }
        let limiting = starfield_involved || self.settling_after_starfield;
        let limit_delta = if limiting {
            if initialize_starfield_history {
                INITIAL_SLEW_DELTA
            } else {
                OUTPUT_SLEW_PER_SECOND * dt as f32
            }
        } else {
            1.0
        };
        if !starfield_involved && self.settling_after_starfield {
            self.settled_slew_credit += limit_delta;
            if self.settled_slew_credit >= 1.0 {
                self.settling_after_starfield = false;
            }
        }
        let params = Parameters {
            values: [
                limit_delta,
                if self.history_valid { 1.0 } else { 0.0 },
                if self.format.is_srgb() { 1.0 } else { 0.0 },
                0.0,
            ],
        };
        queue.write_buffer(&self.parameters, 0, bytemuck::bytes_of(&params));

        let history_view = &targets.history_views[next_index];
        if let Some(output) = output {
            let attachments = [
                Some(color_attachment(output)),
                Some(color_attachment(history_view)),
            ];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chipviz Starfield transition output"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.dual_pipeline);
            pass.set_bind_group(0, &targets.bind_groups[self.history_index], &[]);
            pass.draw(0..3, 0..1);
        } else {
            let attachments = [Some(color_attachment(history_view))];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chipviz Starfield transition history"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.single_pipeline);
            pass.set_bind_group(0, &targets.bind_groups[self.history_index], &[]);
            pass.draw(0..3, 0..1);
        }

        self.history_index = next_index;
        self.history_valid = true;
        self.last_elapsed = Some(self.last_elapsed.map_or(now, |last| last.max(now)));
    }

    pub fn reset(&mut self) {
        self.history_valid = false;
        self.last_elapsed = None;
        self.settling_after_starfield = false;
        self.settled_slew_credit = 0.0;
        self.history_index = 0;
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn color_attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            store: wgpu::StoreOp::Store,
        },
    }
}
