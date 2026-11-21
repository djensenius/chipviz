//! Unregistered rendering prototype for the native DMG tile model.
//!
//! This owns a fixed-size shade texture and a separate fullscreen pipeline;
//! it is not wired into the window, capture, scene selection, or safety gates.
//! The caller advances the autonomous model explicitly and supplies its shown
//! display. The grayscale palette is illustrative; offscreen readback is not
//! emulator parity or timed presented output. Profile performance, physical
//! hardware, and photosensitivity are untested.

use super::dmg_tile_scroll::{Display, SCREEN_HEIGHT, SCREEN_WIDTH};

const WIDTH: u32 = SCREEN_WIDTH as u32;
const HEIGHT: u32 = SCREEN_HEIGHT as u32;
const PIXELS: usize = WIDTH as usize * HEIGHT as usize;

/// Illustrative neutral DMG greys, as sRGB-encoded bytes.
pub fn shade_pixels(display: Display) -> [u8; PIXELS] {
    let mut pixels = [0; PIXELS];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            pixels[(y * WIDTH + x) as usize] = 255 - 85 * display.shade_at(x as u8, y as u8);
        }
    }
    pixels
}

/// A centered, aspect-preserving viewport, in target pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Prefer whole-pixel scaling when the target can fit the native LCD.
/// Smaller targets use fractional scaling; zero-sized targets cannot be drawn.
pub fn viewport(width: u32, height: u32) -> Option<Viewport> {
    if width == 0 || height == 0 {
        return None;
    }
    let scale = (width as f32 / WIDTH as f32).min(height as f32 / HEIGHT as f32);
    let scale = if scale >= 1.0 { scale.floor() } else { scale };
    // Clamp rounding at fractional scales to the actual attachment bounds.
    let scene_width = (WIDTH as f32 * scale).min(width as f32);
    let scene_height = (HEIGHT as f32 * scale).min(height as f32);
    Some(Viewport {
        x: ((width as f32 - scene_width) / 2.0).floor(),
        y: ((height as f32 - scene_height) / 2.0).floor(),
        width: scene_width,
        height: scene_height,
    })
}

/// Fixed-resource WGPU renderer for one DMG display.
///
/// The target format must match the view passed to `encode`. Submit each
/// encoded frame before encoding another with this renderer: queue uploads
/// share the same texture. This is not a selectable application scene.
pub struct DmgTileRenderer {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl DmgTileRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("DMG tile prototype shades"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("DMG tile prototype layout"),
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
            label: Some("DMG tile prototype bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("DMG tile prototype shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("dmg_tile_scroll.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("DMG tile prototype pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("DMG tile prototype pipeline"),
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
        Self {
            texture,
            bind_group,
            pipeline,
        }
    }

    /// Upload the model's shown display and draw it with black letterboxing.
    /// `size` must be the target view's actual attachment dimensions.
    /// Returns false without encoding if either target dimension is zero.
    pub fn encode(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        display: Display,
    ) -> bool {
        let Some(viewport) = viewport(size.0, size.1) else {
            return false;
        };
        let pixels = shade_pixels(display);
        queue.write_texture(
            self.texture.as_image_copy(),
            &pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH),
                rows_per_image: Some(HEIGHT),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("DMG tile prototype pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        true
    }
}
