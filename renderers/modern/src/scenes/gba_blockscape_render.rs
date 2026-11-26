//! Offscreen-only GBA Blockscape prototype; not a selectable or safe scene.
//!
//! The renderer expands native RGB15 values to illustrative sRGB bytes by
//! bit replication, not calibrated GBA screen colours. It preserves aspect
//! ratio with nearest-neighbour scaling and opaque black letterboxing, using
//! integral GPU viewport bounds even for fractional or tiny target sizes.
//! Native timing, device behaviour, performance, and flash safety remain open.

use super::gba_blockscape::{Frame, HEIGHT, PIXELS, WIDTH};
use bytemuck::{Pod, Zeroable};

pub const MAX_TARGET_DIMENSION: u32 = 8192;
const WIDTH_U32: u32 = WIDTH as u32;
const HEIGHT_U32: u32 = HEIGHT as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
pub(crate) struct Geometry {
    pub origin_twice: [u32; 2],
    pub scale_num: u32,
    pub scale_den: u32,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RenderError {
    UnsupportedFormat(wgpu::TextureFormat),
    DeviceLimit {
        name: &'static str,
        required: u32,
        available: u32,
    },
    BufferSizeLimit {
        required: u64,
        available: u64,
    },
    TargetFormat {
        expected: wgpu::TextureFormat,
        actual: wgpu::TextureFormat,
    },
    TargetSize {
        width: u32,
        height: u32,
        maximum: u32,
    },
    TargetShape,
    TargetUsage,
    FrameLength {
        actual: usize,
    },
    PaletteColour {
        index: usize,
        colour: u16,
    },
}

pub fn rgba_pixels(frame: &Frame) -> Result<Vec<u8>, RenderError> {
    if frame.indices.len() != PIXELS {
        return Err(RenderError::FrameLength {
            actual: frame.indices.len(),
        });
    }
    if let Some((index, &colour)) = frame
        .palette
        .iter()
        .enumerate()
        .find(|(_, colour)| **colour & 0x8000 != 0)
    {
        return Err(RenderError::PaletteColour { index, colour });
    }
    let mut pixels = Vec::with_capacity(PIXELS * 4);
    for &index in &frame.indices {
        let colour = frame.palette[usize::from(index)];
        for shift in [0, 5, 10] {
            let component = ((colour >> shift) & 31) as u8;
            pixels.push((component << 3) | (component >> 2));
        }
        pixels.push(255);
    }
    Ok(pixels)
}

pub(crate) fn geometry(width: u32, height: u32) -> Option<Geometry> {
    if width == 0 || height == 0 || width > MAX_TARGET_DIMENSION || height > MAX_TARGET_DIMENSION {
        return None;
    }
    let whole_scale = (width / WIDTH_U32).min(height / HEIGHT_U32);
    if whole_scale > 0 {
        return Some(Geometry {
            origin_twice: [
                ((width - WIDTH_U32 * whole_scale) / 2) * 2,
                ((height - HEIGHT_U32 * whole_scale) / 2) * 2,
            ],
            scale_num: whole_scale,
            scale_den: 1,
        });
    }
    let (scale_num, scale_den) = if width * HEIGHT_U32 <= height * WIDTH_U32 {
        (width, WIDTH_U32)
    } else {
        (height, HEIGHT_U32)
    };
    Some(Geometry {
        origin_twice: [
            width * scale_den - WIDTH_U32 * scale_num,
            height * scale_den - HEIGHT_U32 * scale_num,
        ],
        scale_num,
        scale_den,
    })
}

pub(crate) fn validate_limits(limits: &wgpu::Limits) -> Result<u32, RenderError> {
    for (name, available, required) in [
        (
            "max_texture_dimension_2d",
            limits.max_texture_dimension_2d,
            WIDTH_U32,
        ),
        (
            "max_texture_array_layers",
            limits.max_texture_array_layers,
            1,
        ),
        ("max_bind_groups", limits.max_bind_groups, 1),
        (
            "max_bindings_per_bind_group",
            limits.max_bindings_per_bind_group,
            2,
        ),
        (
            "max_sampled_textures_per_shader_stage",
            limits.max_sampled_textures_per_shader_stage,
            1,
        ),
        (
            "max_uniform_buffers_per_shader_stage",
            limits.max_uniform_buffers_per_shader_stage,
            1,
        ),
        (
            "max_uniform_buffer_binding_size",
            limits.max_uniform_buffer_binding_size,
            std::mem::size_of::<Geometry>() as u32,
        ),
        ("max_color_attachments", limits.max_color_attachments, 1),
        (
            "max_color_attachment_bytes_per_sample",
            limits.max_color_attachment_bytes_per_sample,
            8,
        ),
    ] {
        if available < required {
            return Err(RenderError::DeviceLimit {
                name,
                required,
                available,
            });
        }
    }
    let required = std::mem::size_of::<Geometry>() as u64;
    if limits.max_buffer_size < required {
        return Err(RenderError::BufferSizeLimit {
            required,
            available: limits.max_buffer_size,
        });
    }
    Ok(limits.max_texture_dimension_2d.min(MAX_TARGET_DIMENSION))
}

/// Fixed resources for synchronous offscreen draws; no window or surface.
pub struct GbaBlockscapeRenderer<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    format: wgpu::TextureFormat,
    max_target_dimension: u32,
    pixels: wgpu::Texture,
    geometry: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl<'a> GbaBlockscapeRenderer<'a> {
    pub fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Result<Self, RenderError> {
        if !matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
        ) {
            return Err(RenderError::UnsupportedFormat(format));
        }
        let max_target_dimension = validate_limits(&device.limits())?;
        let pixels = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("GBA Blockscape prototype pixels"),
            size: wgpu::Extent3d {
                width: WIDTH_U32,
                height: HEIGHT_U32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let geometry = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GBA Blockscape prototype geometry"),
            size: std::mem::size_of::<Geometry>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GBA Blockscape prototype layout"),
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
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<Geometry>() as u64
                        ),
                    },
                    count: None,
                },
            ],
        });
        let view = pixels.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GBA Blockscape prototype bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: geometry.as_entire_binding(),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GBA Blockscape prototype shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gba_blockscape.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("GBA Blockscape prototype pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("GBA Blockscape prototype pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_fullscreen",
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: if format.is_srgb() {
                    "fs_srgb"
                } else {
                    "fs_unorm"
                },
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
            device,
            queue,
            format,
            max_target_dimension,
            pixels,
            geometry,
            bind_group,
            pipeline,
        })
    }

    /// Validate the complete frame and attachment before any upload or clear.
    pub fn render(&mut self, target: &wgpu::Texture, frame: &Frame) -> Result<(), RenderError> {
        if target.format() != self.format {
            return Err(RenderError::TargetFormat {
                expected: self.format,
                actual: target.format(),
            });
        }
        let size = target.size();
        if size.width == 0
            || size.height == 0
            || size.width > self.max_target_dimension
            || size.height > self.max_target_dimension
        {
            return Err(RenderError::TargetSize {
                width: size.width,
                height: size.height,
                maximum: self.max_target_dimension,
            });
        }
        if target.dimension() != wgpu::TextureDimension::D2
            || size.depth_or_array_layers != 1
            || target.mip_level_count() != 1
            || target.sample_count() != 1
        {
            return Err(RenderError::TargetShape);
        }
        if !target
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(RenderError::TargetUsage);
        }
        let geometry = geometry(size.width, size.height).ok_or(RenderError::TargetSize {
            width: size.width,
            height: size.height,
            maximum: self.max_target_dimension,
        })?;
        let pixels = rgba_pixels(frame)?;

        self.queue
            .write_buffer(&self.geometry, 0, bytemuck::bytes_of(&geometry));
        self.queue.write_texture(
            self.pixels.as_image_copy(),
            &pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH_U32 * 4),
                rows_per_image: Some(HEIGHT_U32),
            },
            wgpu::Extent3d {
                width: WIDTH_U32,
                height: HEIGHT_U32,
                depth_or_array_layers: 1,
            },
        );
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("GBA Blockscape prototype encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("GBA Blockscape prototype pass"),
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
        let submission = self.queue.submit(std::iter::once(encoder.finish()));
        self.device
            .poll(wgpu::Maintain::WaitForSubmissionIndex(submission));
        Ok(())
    }
}
