use std::sync::Arc;

use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;

use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scene::{Camera, Instance, QUAD_VERTICES, Vertex, build_instances, scene_uniforms};
use crate::scenes;

/// Offscreen capture format; sRGB like the window's preferred surface format.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Pipeline, buffers and camera shared by the window and offscreen targets.
pub struct ScenePipeline {
    profile: RenderProfile,
    camera: Camera,
    device: wgpu::Device,
    queue: wgpu::Queue,
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
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
        }
    }
}

impl std::error::Error for RendererError {}

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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("chipviz pipeline layout"),
            bind_group_layouts: &[&uniform_bind_group_layout],
            push_constant_ranges: &[],
        });
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("chipviz render pipeline"),
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
        });
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chipviz quad vertices"),
            contents: bytemuck::cast_slice(&QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instance_capacity = profile.instance_count.max(1);
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chipviz instances"),
            size: (instance_capacity * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            profile,
            camera: Camera::default(),
            device,
            queue,
            render_pipeline,
            vertex_buffer,
            instance_buffer,
            instance_capacity,
            uniform_buffer,
            uniform_bind_group,
        }
    }

    /// Restarts camera smoothing, as at renderer start-up.
    pub fn reset_camera(&mut self) {
        self.camera = Camera::default();
    }

    /// Steps camera smoothing for a frame that is not drawn.
    pub fn advance_camera(&mut self, frame: ControlFrame, elapsed: f32, aspect: f32) {
        self.camera
            .build_uniforms(&self.profile, frame, elapsed, aspect);
    }

    /// Uploads this frame's instances and uniforms, then records its render pass.
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        frame: ControlFrame,
        elapsed: f32,
        aspect: f32,
    ) {
        let instances = build_instances(&self.profile, frame, elapsed);
        let instance_count = instances.len().min(self.instance_capacity);
        self.queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&instances[..instance_count]),
        );
        let uniforms = scene_uniforms(
            scenes::select(frame.scene).camera,
            &mut self.camera,
            &self.profile,
            frame,
            elapsed,
            aspect,
        );
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

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

    pub fn resize_to_current_window(&mut self) {
        let size = self.window.inner_size();
        self.resize(size.width, size.height);
    }

    pub fn render(&mut self, frame: ControlFrame, elapsed: f32) -> Result<(), RendererError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let mut encoder = self.pipeline.command_encoder();
        self.pipeline
            .encode(&mut encoder, &view, frame, elapsed, aspect);
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
        let (width, height) = (width.max(1), height.max(1));
        let (_, device, queue) = request_device(&gpu_instance(), &profile, None).await?;
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
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chipviz offscreen readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let aspect = width as f32 / height as f32;
        let pipeline = ScenePipeline::new(device, queue, OFFSCREEN_FORMAT, profile, aspect);

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

    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }

    pub fn pipeline_mut(&mut self) -> &mut ScenePipeline {
        &mut self.pipeline
    }

    /// Draws one frame and returns tightly packed RGB8 rows, top row first.
    pub fn render(&mut self, frame: ControlFrame, elapsed: f32) -> Result<Vec<u8>, RendererError> {
        let aspect = self.aspect();
        let mut encoder = self.pipeline.command_encoder();
        self.pipeline
            .encode(&mut encoder, &self.view, frame, elapsed, aspect);
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
    size.width.max(1) as f32 / size.height.max(1) as f32
}
