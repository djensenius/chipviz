//! Unregistered, offscreen renderer for the logical PETSCII meter field.
//!
//! The caller explicitly steps the model; this module has no clock, flash mode,
//! scene registration, or presentation path. Its 8x8 marks are the repository's
//! original geometric artwork, not a C64 ROM. The colours below are invented
//! sRGB bytes, not a hardware palette or a pixel-parity claim.
//!
//! Untouched cells use a distinctive diagnostic colour, not a known glyph or
//! background. Unsupported codes, colours outside 0..=15, incorrect cell counts,
//! and inconsistent artwork evidence are errors rather than valid-looking art.
//! The exact 40x25 cell grid is row-major. The only internal GPU image is a
//! 320x200 RGBA8 texture; CPU composition is one 256,000-byte image, plus
//! WGPU's transient upload staging.
//!
//! Targets are single-sample, single-layer, single-mip RGBA8 UNORM or sRGB
//! render attachments, at most 8192 pixels per dimension and within the device
//! limit. The target's own metadata determines its viewport. The source texture
//! holds encoded bytes: the shader passes them through for UNORM and decodes
//! them to linear for an sRGB attachment, so both store the same encoded
//! colours. Integer scaling is preferred; smaller targets use nearest-neighbour
//! fractional scaling, with opaque black letterboxing.
//!
//! Each successful call uploads, submits, and waits for that submission before
//! returning. This bounds in-flight uploads and prevents a later frame from
//! replacing the shared texture before an earlier draw. The device, queue and
//! undestroyed target must belong together. This blocking offscreen prototype
//! says nothing about presented timing, physical Pi behaviour, or safety.

use super::petscii_meter_field::{Cell, HEIGHT, PetsciiMeterField, WIDTH};
use super::petscii_meter_field_artwork::{self, CELL_SIZE, PIXEL_HEIGHT, PIXEL_WIDTH, Pixel};

pub const CELL_COUNT: usize = WIDTH * HEIGHT;
pub const FRAME_BYTES: usize = PIXEL_WIDTH * PIXEL_HEIGHT * 4;
pub const MAX_TARGET_DIMENSION: u32 = 8192;

const TEXTURE_WIDTH: u32 = PIXEL_WIDTH as u32;
const TEXTURE_HEIGHT: u32 = PIXEL_HEIGHT as u32;

/// Invented sRGB-encoded RGBA colours; index zero is a valid foreground.
pub const PALETTE: [[u8; 4]; 16] = [
    [4, 8, 12, 255],
    [232, 196, 104, 255],
    [82, 164, 203, 255],
    [214, 104, 92, 255],
    [150, 202, 131, 255],
    [171, 114, 200, 255],
    [240, 164, 94, 255],
    [118, 207, 190, 255],
    [109, 117, 185, 255],
    [196, 155, 128, 255],
    [225, 135, 180, 255],
    [137, 163, 75, 255],
    [212, 215, 154, 255],
    [90, 126, 150, 255],
    [201, 129, 76, 255],
    [226, 230, 220, 255],
];
pub const BACKGROUND: [u8; 4] = [16, 22, 32, 255];
pub const UNKNOWN_CELL: [u8; 4] = [109, 55, 96, 255];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposeError {
    CellCount {
        expected: usize,
        actual: usize,
    },
    InvalidColour {
        x: usize,
        y: usize,
        colour: u8,
    },
    UnsupportedCode {
        x: usize,
        y: usize,
        code: u8,
    },
    InconsistentArtwork {
        x: usize,
        y: usize,
        pixel_x: usize,
        pixel_y: usize,
    },
}

fn snapshot(field: &PetsciiMeterField) -> [Option<Cell>; CELL_COUNT] {
    std::array::from_fn(|index| field.cell(index % WIDTH, index / WIDTH))
}

/// Compose a model snapshot without advancing it.
pub fn compose(field: &PetsciiMeterField) -> Result<Vec<u8>, ComposeError> {
    compose_cells(&snapshot(field))
}

/// Compose exactly 40x25 row-major cells into 320x200 encoded RGBA bytes.
pub fn compose_cells(cells: &[Option<Cell>]) -> Result<Vec<u8>, ComposeError> {
    compose_with(cells, petscii_meter_field_artwork::cell_pixel)
}

// The injectable artwork seam also lets tests prove inconsistent evidence is
// rejected; production always uses the repository-authored cell_pixel.
pub(crate) fn compose_with(
    cells: &[Option<Cell>],
    artwork: impl Fn(Option<Cell>, usize, usize) -> Option<Pixel>,
) -> Result<Vec<u8>, ComposeError> {
    if cells.len() != CELL_COUNT {
        return Err(ComposeError::CellCount {
            expected: CELL_COUNT,
            actual: cells.len(),
        });
    }
    for (index, cell) in cells.iter().enumerate() {
        if let Some(cell) = cell {
            let (x, y) = (index % WIDTH, index / WIDTH);
            if cell.colour > 15 {
                return Err(ComposeError::InvalidColour {
                    x,
                    y,
                    colour: cell.colour,
                });
            }
            if !matches!(cell.code, 32 | 81..=88 | 96..=111) {
                return Err(ComposeError::UnsupportedCode {
                    x,
                    y,
                    code: cell.code,
                });
            }
        }
    }

    let mut image = vec![0; FRAME_BYTES];
    for (index, &cell) in cells.iter().enumerate() {
        let (x, y) = (index % WIDTH, index / WIDTH);
        for pixel_y in 0..CELL_SIZE {
            for pixel_x in 0..CELL_SIZE {
                let evidence = artwork(cell, pixel_x, pixel_y);
                let error = ComposeError::InconsistentArtwork {
                    x,
                    y,
                    pixel_x,
                    pixel_y,
                };
                if evidence != petscii_meter_field_artwork::cell_pixel(cell, pixel_x, pixel_y) {
                    return Err(error);
                }
                let colour = match (cell, evidence) {
                    (None, Some(Pixel::UnknownCell)) => Some(UNKNOWN_CELL),
                    (Some(cell), Some(Pixel::Background)) if cell.code != 102 => Some(BACKGROUND),
                    (Some(cell), Some(Pixel::Foreground(colour)))
                        if cell.code != 32 && colour == cell.colour =>
                    {
                        PALETTE.get(usize::from(colour)).copied()
                    }
                    _ => None,
                }
                .ok_or(error)?;
                let offset =
                    ((y * CELL_SIZE + pixel_y) * PIXEL_WIDTH + x * CELL_SIZE + pixel_x) * 4;
                image[offset..offset + 4].copy_from_slice(&colour);
            }
        }
    }
    Ok(image)
}

/// A centered, aspect-preserving viewport in target pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Reject zero or over-limit dimensions; prefer whole-pixel scaling when possible.
pub fn viewport(width: u32, height: u32) -> Option<Viewport> {
    if width == 0 || height == 0 || width > MAX_TARGET_DIMENSION || height > MAX_TARGET_DIMENSION {
        return None;
    }
    let scale = (width / TEXTURE_WIDTH).min(height / TEXTURE_HEIGHT);
    if scale > 0 {
        let scene_width = TEXTURE_WIDTH * scale;
        let scene_height = TEXTURE_HEIGHT * scale;
        return Some(Viewport {
            x: ((width - scene_width) / 2) as f32,
            y: ((height - scene_height) / 2) as f32,
            width: scene_width as f32,
            height: scene_height as f32,
        });
    }

    // Keep centering exact until the final viewport conversion. Rounding a
    // fractional scale first can move a mathematically integral edge by one
    // target pixel (for example 192x124 at a 3/5 scale).
    if width * TEXTURE_HEIGHT <= height * TEXTURE_WIDTH {
        let scene_height = width * TEXTURE_HEIGHT;
        Some(Viewport {
            x: 0.0,
            y: ((height * TEXTURE_WIDTH - scene_height) / (2 * TEXTURE_WIDTH)) as f32,
            width: width as f32,
            height: scene_height as f32 / TEXTURE_WIDTH as f32,
        })
    } else {
        let scene_width = height * TEXTURE_WIDTH;
        Some(Viewport {
            x: ((width * TEXTURE_HEIGHT - scene_width) / (2 * TEXTURE_HEIGHT)) as f32,
            y: 0.0,
            width: scene_width as f32 / TEXTURE_HEIGHT as f32,
            height: height as f32,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenderError {
    UnsupportedFormat(wgpu::TextureFormat),
    DeviceLimit {
        name: &'static str,
        required: u32,
        available: u32,
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
    TargetShape {
        dimension: wgpu::TextureDimension,
        layers: u32,
        mip_levels: u32,
        samples: u32,
    },
    TargetUsage,
    Composition(ComposeError),
}

// These are the only device limits used by the one-texture, one-target
// pipeline. WGPU counts an RGBA8 colour attachment as eight bytes per sample.
pub(crate) fn validate_limits(limits: &wgpu::Limits) -> Result<u32, RenderError> {
    for (name, available, required) in [
        (
            "max_texture_dimension_2d",
            limits.max_texture_dimension_2d,
            TEXTURE_WIDTH.max(TEXTURE_HEIGHT),
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
            1,
        ),
        (
            "max_sampled_textures_per_shader_stage",
            limits.max_sampled_textures_per_shader_stage,
            1,
        ),
        (
            "max_inter_stage_shader_components",
            limits.max_inter_stage_shader_components,
            2,
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
    Ok(limits.max_texture_dimension_2d.min(MAX_TARGET_DIMENSION))
}

/// A fixed-texture pipeline that submits and completes one offscreen draw per call.
pub struct PetsciiMeterFieldRenderer<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    format: wgpu::TextureFormat,
    max_target_dimension: u32,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl<'a> PetsciiMeterFieldRenderer<'a> {
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
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PETSCII meter prototype pixels"),
            size: wgpu::Extent3d {
                width: TEXTURE_WIDTH,
                height: TEXTURE_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PETSCII meter prototype layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PETSCII meter prototype bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PETSCII meter prototype shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("petscii_meter_field.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("PETSCII meter prototype pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("PETSCII meter prototype pipeline"),
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
            texture,
            bind_group,
            pipeline,
        })
    }

    /// Render the supplied model without stepping it.
    pub fn render(
        &mut self,
        target: &wgpu::Texture,
        field: &PetsciiMeterField,
    ) -> Result<(), RenderError> {
        self.render_cells(target, &snapshot(field))
    }

    /// Render validated row-major evidence. Errors occur before upload or clear.
    pub fn render_cells(
        &mut self,
        target: &wgpu::Texture,
        cells: &[Option<Cell>],
    ) -> Result<(), RenderError> {
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
            return Err(RenderError::TargetShape {
                dimension: target.dimension(),
                layers: size.depth_or_array_layers,
                mip_levels: target.mip_level_count(),
                samples: target.sample_count(),
            });
        }
        if !target
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(RenderError::TargetUsage);
        }
        let viewport = viewport(size.width, size.height).ok_or(RenderError::TargetSize {
            width: size.width,
            height: size.height,
            maximum: self.max_target_dimension,
        })?;
        let pixels = compose_cells(cells).map_err(RenderError::Composition)?;

        self.queue.write_texture(
            self.texture.as_image_copy(),
            &pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(TEXTURE_WIDTH * 4),
                rows_per_image: Some(TEXTURE_HEIGHT),
            },
            wgpu::Extent3d {
                width: TEXTURE_WIDTH,
                height: TEXTURE_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("PETSCII meter prototype encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("PETSCII meter prototype pass"),
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
            pass.set_viewport(
                viewport.x,
                viewport.y,
                viewport.width,
                viewport.height,
                0.0,
                1.0,
            );
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
