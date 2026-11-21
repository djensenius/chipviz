//! Unregistered offscreen GPU prototype for the indexed Diamond Scroll model.
//!
//! The CPU uploads one bounded 256x240 image using the model's approximate
//! sRGB palette. A full-screen triangle uses integer texture loads (no
//! filtering) and the model's centered square-pixel viewport: integer scaling
//! when possible, nearest-neighbour downsampling otherwise, with black bars.
//! While the model is not showing the demo, the prototype draws black rather
//! than attempting to render the previous scene. This intentionally does not
//! correct NES display aspect or overscan.
//!
//! The source texture is UNORM so its stored bytes are not implicitly
//! decoded. For an sRGB attachment the shader decodes to linear; for a UNORM
//! attachment it outputs encoded values directly. Neither path establishes
//! emulator parity, photosensitivity safety, or physical Pi performance.

use super::nes_diamond_scroll::{DiamondScroll, HEIGHT, Viewport, WIDTH, approximate_srgb};
use wgpu::util::DeviceExt;

pub struct DiamondGpu {
    source: wgpu::Texture,
    target: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl DiamondGpu {
    /// Only RGBA8 UNORM and sRGB targets are supported by this prototype.
    pub fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<Self, &'static str> {
        let viewport = Viewport::fit(width, height).ok_or("invalid viewport size")?;
        if width > device.limits().max_texture_dimension_2d
            || height > device.limits().max_texture_dimension_2d
        {
            return Err("viewport exceeds device texture limits");
        }
        let srgb = match format {
            wgpu::TextureFormat::Rgba8Unorm => false,
            wgpu::TextureFormat::Rgba8UnormSrgb => true,
            _ => return Err("unsupported target format"),
        };
        let texture = |label, width, height, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let source = texture(
            "diamond source",
            WIDTH,
            HEIGHT,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let target = texture(
            "diamond offscreen target",
            width,
            height,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let (x, y, w, h) = viewport.bounds();
        // Two vec4<u32>s match WGSL's 16-byte uniform alignment.
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("diamond viewport"),
            contents: bytemuck::cast_slice(&[x, y, w, h, if srgb { 1 } else { 0 }, 0, 0, 0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("diamond bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("diamond bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &source.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("diamond shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("nes_diamond_scroll_gpu.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("diamond pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("diamond pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
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
        });
        Ok(Self {
            source,
            target,
            bind_group,
            pipeline,
        })
    }

    pub fn target(&self) -> &wgpu::Texture {
        &self.target
    }

    /// Upload and encode one frame. Submit this encoder before calling encode
    /// again on the same renderer: queue writes are not per-encoder snapshots.
    pub fn encode(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &DiamondScroll,
    ) {
        let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let offset = ((y * WIDTH + x) * 4) as usize;
                let rgb = scene
                    .pixel(x, y)
                    .map(|index| approximate_srgb(index).expect("known demo palette index"))
                    .unwrap_or([0, 0, 0]);
                pixels[offset..offset + 3].copy_from_slice(&rgb);
                pixels[offset + 3] = 255;
            }
        }
        queue.write_texture(
            self.source.as_image_copy(),
            &pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("diamond offscreen pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
