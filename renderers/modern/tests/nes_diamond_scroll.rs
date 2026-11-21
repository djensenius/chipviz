// The prototype is deliberately not in the production scene registry.
#[path = "../src/scenes/nes_diamond_scroll.rs"]
mod nes_diamond_scroll;

use nes_diamond_scroll::{DiamondScroll, Viewport, approximate_srgb};

fn advance(scene: &mut DiamondScroll, count: usize) {
    for _ in 0..count {
        scene.nmi(true);
    }
}

#[test]
fn independently_decoded_chr_and_quadrants() {
    let scene = DiamondScroll::default();
    // CHR planes in main.s decode to these palette-entry numbers, MSB first.
    // This literal matrix is intentionally independent of the model's decoder.
    let entries = [
        [0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 1, 3, 3, 1, 0, 0],
        [0, 1, 3, 3, 3, 3, 1, 0],
        [1, 3, 3, 3, 3, 3, 3, 1],
        [0, 1, 3, 3, 3, 3, 1, 0],
        [0, 0, 1, 3, 3, 1, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
    ];
    let palette = [0x0f, 0x11, 0x21, 0x31];
    for (y, row) in entries.iter().enumerate() {
        for (x, &entry) in row.iter().enumerate() {
            assert_eq!(scene.pixel(x as u32, y as u32), Some(palette[entry]));
            // All first 32 CHR tiles are identical.
            assert_eq!(scene.pixel(x as u32 + 8, y as u32), Some(palette[entry]));
        }
    }
    // $e4 packs quadrants 0, 1, 2, 3; entry 1 and 3 probe both colours.
    for (x, y, one, three) in [
        (2, 2, 0x11, 0x31),
        (18, 2, 0x05, 0x25),
        (2, 18, 0x09, 0x29),
        (18, 18, 0x0c, 0x2c),
        (34, 34, 0x11, 0x31),
    ] {
        assert_eq!(scene.pixel(x, y), Some(one));
        assert_eq!(scene.pixel(x + 1, y), Some(three));
    }
    // Samples immediately on either side of 16-pixel attribute boundaries.
    assert_eq!(scene.pixel(15, 4), Some(0x11));
    assert_eq!(scene.pixel(16, 4), Some(0x05));
    assert_eq!(scene.pixel(3, 15), Some(0x11));
    assert_eq!(scene.pixel(3, 16), Some(0x0f));
    assert_eq!(scene.pixel(3, 17), Some(0x09));
    assert_eq!(scene.pixel(256, 0), None);
    assert_eq!(scene.pixel(0, 240), None);
}

#[test]
fn startup_palette_scroll_busy_and_wrap() {
    let mut scene = DiamondScroll::default();
    assert_eq!(
        (scene.frame(), scene.scroll_y(), scene.foreground()),
        (0, 0, 0x11)
    );
    scene.nmi(false);
    assert_eq!(
        (scene.frame(), scene.scroll_y(), scene.foreground()),
        (0, 0, 0x11)
    );
    advance(&mut scene, 1);
    assert_eq!((scene.frame(), scene.scroll_y()), (1, 0));
    advance(&mut scene, 1);
    assert_eq!((scene.frame(), scene.scroll_y()), (2, 1));
    // Vertical scroll crosses from $2000 to the identically populated $2800.
    assert_eq!(scene.pixel(3, 238), Some(0x11));
    assert_eq!(scene.pixel(3, 239), Some(0x0f));
    advance(&mut scene, 29);
    assert_eq!(
        (scene.frame(), scene.scroll_y(), scene.foreground()),
        (31, 15, 0x11)
    );
    for expected in [0x12, 0x13, 0x14, 0x15, 0x14, 0x13, 0x12] {
        scene.nmi(true);
        assert_eq!(scene.foreground(), expected);
        advance(&mut scene, 31);
        assert_eq!(scene.foreground(), expected);
    }
    assert_eq!((scene.frame(), scene.scroll_y()), (255, 127));
    scene.nmi(true);
    assert_eq!(
        (scene.frame(), scene.scroll_y(), scene.foreground()),
        (0, 0, 0x11)
    );
}

#[test]
fn reentry_restores_on_the_next_accepted_nmi_without_reset() {
    let mut scene = DiamondScroll::default();
    advance(&mut scene, 10);
    scene.leave_demo();
    advance(&mut scene, 35);
    assert_eq!(scene.frame(), 45);
    assert_eq!(scene.pixel(3, 1), None);
    scene.queue_reentry();
    scene.nmi(false);
    assert_eq!(scene.frame(), 45);
    assert_eq!(scene.pixel(3, 1), None);
    scene.nmi(true);
    assert_eq!(
        (scene.frame(), scene.scroll_y(), scene.foreground()),
        (46, 23, 0x12)
    );

    scene.leave_demo();
    advance(&mut scene, 241); // 46 + 241 wraps to 31
    scene.queue_reentry(); // queues phase 0 at Frame 31
    scene.nmi(true); // upload first, then Frame 32's phase 1 overwrites it
    assert_eq!(
        (scene.frame(), scene.scroll_y(), scene.foreground()),
        (32, 16, 0x12)
    );
}

#[test]
fn viewport_fit_is_bounded_and_pixel_aligned() {
    for (size, bounds) in [
        ((256, 240), (0, 0, 256, 240)),
        ((1920, 1080), (448, 60, 1024, 960)),
        ((3840, 2160), (768, 0, 2304, 2160)),
        ((1080, 1920), (28, 480, 1024, 960)),
        ((128, 120), (0, 0, 128, 120)),
    ] {
        let view = Viewport::fit(size.0, size.1).unwrap();
        assert_eq!(view.bounds(), bounds);
        assert_eq!(view.source_pixel(bounds.0, bounds.1), Some((0, 0)));
        assert_eq!(view.source_pixel(bounds.0 + bounds.2, bounds.1), None);
        assert_eq!(view.source_pixel(bounds.0, bounds.1 + bounds.3), None);
        if size == (1920, 1080) {
            assert_eq!(view.source_pixel(451, 63), Some((0, 0)));
            assert_eq!(view.source_pixel(452, 64), Some((1, 1)));
        }
    }
    assert!(Viewport::fit(0, 240).is_none());
    assert!(Viewport::fit(8193, 240).is_none());
    assert_eq!(Viewport::fit(1, 1).unwrap().bounds(), (0, 0, 1, 1));
}

#[test]
fn rgb_values_are_explicit_encoded_approximations() {
    assert_eq!(approximate_srgb(0x0f), Some([0, 0, 0]));
    assert_eq!(approximate_srgb(0x12), Some([0, 88, 248]));
    assert_eq!(approximate_srgb(0x19), Some([0, 184, 0]));
    assert_eq!(approximate_srgb(0xff), None);
}
