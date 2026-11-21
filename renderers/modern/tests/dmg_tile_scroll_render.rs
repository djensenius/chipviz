// Exercise the unregistered CPU preparation without starting a GPU.
#[expect(
    dead_code,
    reason = "this fixture uses only the model API needed by CPU preparation"
)]
#[path = "../src/scenes/dmg_tile_scroll.rs"]
mod dmg_tile_scroll;
#[expect(
    dead_code,
    reason = "this CPU fixture does not construct the GPU renderer"
)]
#[path = "../src/scenes/dmg_tile_scroll_render.rs"]
mod dmg_tile_scroll_render;

use dmg_tile_scroll::{Display, DmgTileScroll};
use dmg_tile_scroll_render::{Viewport, shade_pixels, viewport};

#[test]
fn frame_pixels_use_native_rows_palette_and_scroll() {
    let mut scene = DmgTileScroll::default();
    let initial = shade_pixels(scene.shown());
    assert_eq!(initial.len(), 160 * 144);
    // Native first row: blank tile 0, tile 1's $81,$81 ends,
    // tile 2's $ff,$00 and tile 3's $11,$ee bitplanes.
    assert_eq!(&initial[0..8], &[255; 8]);
    assert_eq!(&initial[8..16], &[0, 255, 255, 255, 255, 255, 255, 0]);
    assert_eq!(&initial[16..24], &[170; 8]);
    assert_eq!(&initial[24..32], &[85, 85, 85, 170, 85, 85, 85, 170]);
    scene.tick();
    scene.tick();
    let scrolled = shade_pixels(scene.shown());
    assert_eq!(&scrolled[0..8], &[255, 255, 255, 0, 0, 255, 255, 255]);
    // $ec maps tile colour 1 to black and leaves colour 2 dark grey.
    let phase = shade_pixels(Display {
        scx: 0,
        scy: 0,
        bgp: 0xec,
    });
    assert_eq!(&phase[16..24], &[0; 8]);
    assert_eq!(&phase[24..28], &[85, 85, 85, 0]);
}

#[test]
fn viewport_fits_native_aspect_and_aligns_integral_pixels() {
    assert_eq!(viewport(0, 144), None);
    assert_eq!(viewport(160, 0), None);
    assert_eq!(
        viewport(3840, 2160),
        Some(Viewport {
            x: 720.0,
            y: 0.0,
            width: 2400.0,
            height: 2160.0,
        })
    );
    assert_eq!(
        viewport(321, 289),
        Some(Viewport {
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 288.0,
        })
    );
    assert_eq!(
        viewport(80, 72),
        Some(Viewport {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 72.0,
        })
    );
    let portrait = viewport(144, 320).unwrap();
    assert_eq!(portrait.x, 0.0);
    assert_eq!(portrait.y, 95.0);
    assert!((portrait.width - 144.0).abs() < 0.001);
    assert!((portrait.height - 129.6).abs() < 0.001);
}
