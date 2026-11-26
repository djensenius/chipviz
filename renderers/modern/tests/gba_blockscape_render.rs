//! Portable validation of the unregistered offscreen renderer's input contract.

#[expect(dead_code, reason = "only the model frame is used by this test")]
#[path = "../src/scenes/gba_blockscape.rs"]
mod gba_blockscape;
#[expect(dead_code, reason = "GPU construction is exercised by the opt-in test")]
#[path = "../src/scenes/gba_blockscape_render.rs"]
mod gba_blockscape_render;

use gba_blockscape::{Frame, PIXELS};
use gba_blockscape_render::{
    MAX_TARGET_DIMENSION, RenderError, geometry, rgba_pixels, validate_limits,
};

fn sample(width: u32, height: u32, x: u32, y: u32) -> Option<(u32, u32)> {
    let geometry = geometry(width, height)?;
    let center = [
        (x * 2 + 1) * geometry.scale_den,
        (y * 2 + 1) * geometry.scale_den,
    ];
    let offset = [
        center[0].checked_sub(geometry.origin_twice[0])?,
        center[1].checked_sub(geometry.origin_twice[1])?,
    ];
    let coordinate = [
        offset[0] / (2 * geometry.scale_num),
        offset[1] / (2 * geometry.scale_num),
    ];
    (coordinate[0] < 240 && coordinate[1] < 160).then_some((coordinate[0], coordinate[1]))
}

#[test]
fn rgb15_expansion_and_invalid_frame_rejection() {
    let mut frame = Frame {
        indices: vec![0; PIXELS],
        palette: [0; 256],
    };
    frame.indices[..5].copy_from_slice(&[0, 1, 2, 3, 4]);
    frame.palette[0] = 31;
    frame.palette[1] = 31 << 5;
    frame.palette[2] = 31 << 10;
    frame.palette[3] = 16 | (3 << 5) | (5 << 10);
    frame.palette[4] = 0x7fff;
    let pixels = rgba_pixels(&frame).unwrap();
    assert_eq!(
        &pixels[..20],
        &[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 132, 24, 41, 255, 255, 255, 255, 255
        ]
    );
    frame.palette[200] = 0x8000;
    assert_eq!(
        rgba_pixels(&frame),
        Err(RenderError::PaletteColour {
            index: 200,
            colour: 0x8000,
        })
    );
    frame.indices.pop();
    assert_eq!(
        rgba_pixels(&frame),
        Err(RenderError::FrameLength { actual: PIXELS - 1 })
    );
}

#[test]
fn aspect_preserving_sampling_handles_tiny_and_letterboxed_targets() {
    assert_eq!(sample(1, 1, 0, 0), Some((120, 80)));
    assert_eq!(sample(239, 1, 119, 0), Some((120, 80)));
    assert_eq!(sample(239, 1, 118, 0), None);
    assert_eq!(sample(200, 100, 24, 0), None);
    assert_eq!(sample(200, 100, 25, 0), Some((0, 0)));
    assert_eq!(sample(200, 100, 174, 99), Some((239, 159)));
    assert_eq!(sample(200, 100, 175, 99), None);
    assert_eq!(sample(241, 160, 239, 159), Some((239, 159)));
    assert_eq!(sample(241, 160, 240, 159), None);
    assert_eq!(geometry(0, 1), None);
    assert_eq!(geometry(1, 0), None);
    assert_eq!(geometry(MAX_TARGET_DIMENSION + 1, 1), None);
}

#[test]
fn insufficient_device_limits_are_rejected_before_resource_creation() {
    let mut limits = wgpu::Limits::default();
    assert!(validate_limits(&limits).is_ok());
    limits.max_texture_dimension_2d = 239;
    assert!(matches!(
        validate_limits(&limits),
        Err(RenderError::DeviceLimit {
            name: "max_texture_dimension_2d",
            ..
        })
    ));
    limits.max_texture_dimension_2d = 240;
    limits.max_uniform_buffer_binding_size = 15;
    assert!(matches!(
        validate_limits(&limits),
        Err(RenderError::DeviceLimit {
            name: "max_uniform_buffer_binding_size",
            ..
        })
    ));
    limits.max_uniform_buffer_binding_size = 16;
    limits.max_buffer_size = 15;
    assert!(matches!(
        validate_limits(&limits),
        Err(RenderError::BufferSizeLimit { .. })
    ));
}
