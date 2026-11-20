// Test the unregistered prototype without adding it to the application.
#[path = "../src/scenes/dmg_tile_scroll.rs"]
mod dmg_tile_scroll;

use dmg_tile_scroll::{Display, DmgTileScroll, colour_at, tile_at};

#[test]
fn original_tile_rows_decode_both_bitplanes_and_leftmost_bit() {
    // These are decoded by hand from the native Tiles rows, not generated
    // from the Rust tile table. Coordinates select each tile in map row 0.
    let rows = [
        (0, 3, [0, 3, 3, 3, 3, 3, 3, 0]),  // $7e,$7e
        (8, 0, [3, 0, 0, 0, 0, 0, 0, 3]),  // $81,$81
        (16, 1, [1, 2, 2, 2, 2, 2, 2, 1]), // $81,$7e
        (24, 0, [2, 2, 2, 1, 2, 2, 2, 1]), // $11,$ee
        (24, 3, [1, 2, 2, 2, 1, 2, 2, 2]), // $88,$77
    ];
    for (x, y, expected) in rows {
        for (offset, colour) in expected.into_iter().enumerate() {
            assert_eq!(colour_at(x + offset as u8, y), colour, "x={x} y={y}");
        }
    }
}

#[test]
fn map_repeats_across_and_down_through_the_last_row() {
    // LoadMap's 0,1,2,3 sequence fills 32 entries per row.
    for y in [0, 7, 8, 127, 248, 255] {
        for (x, id) in [
            (0, 0),
            (7, 0),
            (8, 1),
            (24, 3),
            (31, 3),
            (32, 0),
            (224, 0),
            (255, 3),
        ] {
            assert_eq!(tile_at(x, y), id, "x={x} y={y}");
        }
    }
    assert_eq!(colour_at(0, 1), colour_at(32, 1));
    assert_eq!(colour_at(24, 0), colour_at(24, 248));
}

#[test]
fn vblank_pipeline_and_both_scroll_axes_follow_native_steps() {
    let mut scene = DmgTileScroll::default();
    let initial = Display {
        scx: 0,
        scy: 0,
        bgp: 0xe4,
    };
    assert_eq!(scene.shown(), initial);
    scene.tick();
    assert_eq!(scene.shown(), initial);
    // RRCA rotates the low bit into bit 7, not a logical right shift.
    assert_eq!(
        scene.prepared(),
        Display {
            scx: 128,
            scy: 1,
            bgp: 0xe4
        }
    );
    scene.tick();
    assert_eq!(scene.shown().shade_at(3, 0), 3); // map (131,1)
    assert_eq!(scene.shown().shade_at(0, 0), 0);
    assert_eq!(
        scene.prepared(),
        Display {
            scx: 1,
            scy: 2,
            bgp: 0xe4
        }
    );
    scene.tick();
    assert_eq!(scene.shown().shade_at(1, 0), 3); // map (2,2)
    assert_eq!(scene.shown().shade_at(0, 0), 0); // map (1,2)
    assert_eq!(scene.shown().shade_at(255, 255), 0); // map (0,1), wrapped
}

#[test]
fn palette_boundaries_and_counter_wrap_match_native_registers() {
    let mut scene = DmgTileScroll::default();
    // NativeStep: increment 8-bit wFrame, RRCA, AND $18, OR $e4.
    let examples = [
        (15, 0x87, 0xe4, [0, 1, 2, 3]),
        (16, 0x08, 0xec, [0, 3, 2, 3]),
        (31, 0x8f, 0xec, [0, 3, 2, 3]),
        (32, 0x10, 0xf4, [0, 1, 3, 3]),
        (47, 0x97, 0xf4, [0, 1, 3, 3]),
        (48, 0x18, 0xfc, [0, 3, 3, 3]),
        (63, 0x9f, 0xfc, [0, 3, 3, 3]),
        (64, 0x20, 0xe4, [0, 1, 2, 3]),
        (255, 0xff, 0xfc, [0, 3, 3, 3]),
        (256, 0x00, 0xe4, [0, 1, 2, 3]),
    ];
    let mut steps = 0;
    for (target, scx, bgp, shades) in examples {
        while steps < target {
            scene.tick();
            steps += 1;
        }
        assert_eq!(scene.counter(), target as u8);
        assert_eq!(
            scene.prepared(),
            Display {
                scx,
                scy: target as u8,
                bgp
            }
        );
        // Map coordinates (0,0), (16,0), (17,1), (8,0) contain
        // colour numbers 0, 1, 2, 3 respectively.
        let palette = Display {
            scx: 0,
            scy: 0,
            bgp,
        };
        for ((x, y), shade) in [(0, 0), (16, 0), (17, 1), (8, 0)].into_iter().zip(shades) {
            assert_eq!(palette.shade_at(x, y), shade);
        }
    }
    // The wrapped output is still pending for one VBlank.
    assert_eq!(scene.shown().scx, 255);
    assert_eq!(scene.shown().scy, 255);
    assert_eq!(scene.shown().bgp, 0xfc);
    scene.tick();
    assert_eq!(scene.shown().scx, 0);
    assert_eq!(scene.shown().scy, 0);
    assert_eq!(scene.shown().bgp, 0xe4);
}
