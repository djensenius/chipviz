//! Prototype-only model of the autonomous native DMG tile field.
//!
//! This is not registered as a selectable scene. It models native steps and
//! their next-VBlank display registers, not scene switching or boot hardware
//! state. It makes no profile, hardware, or photosensitivity validation claim.

/// The native background is a 32 by 32 tile map, wrapping at 256 pixels.
pub const MAP_PIXELS: u16 = 256;
/// The native LCD dimensions, in pixels.
pub const SCREEN_WIDTH: u16 = 160;
pub const SCREEN_HEIGHT: u16 = 144;

// Two bytes per row: low bitplane, then high bitplane. Bit 7 is leftmost.
// From cores/gb/homebrew/main.asm, Tiles.
const TILES: [[u8; 16]; 4] = [
    [
        0x00, 0x00, 0x18, 0x18, 0x3c, 0x3c, 0x7e, 0x7e, 0xff, 0xff, 0x7e, 0x7e, 0x3c, 0x3c, 0x18,
        0x18,
    ],
    [
        0x81, 0x81, 0x42, 0x42, 0x24, 0x24, 0x18, 0x18, 0x18, 0x18, 0x24, 0x24, 0x42, 0x42, 0x81,
        0x81,
    ],
    [
        0xff, 0x00, 0x81, 0x7e, 0xbd, 0x42, 0xa5, 0x5a, 0xa5, 0x5a, 0xbd, 0x42, 0x81, 0x7e, 0xff,
        0x00,
    ],
    [
        0x11, 0xee, 0x22, 0xdd, 0x44, 0xbb, 0x88, 0x77, 0x11, 0xee, 0x22, 0xdd, 0x44, 0xbb, 0x88,
        0x77,
    ],
];

/// Tile ID at a background-map pixel coordinate, including map wrap.
pub fn tile_at(x: u8, _y: u8) -> u8 {
    // LoadMap writes 0,1,2,3 repeatedly across all 1024 entries.
    (x / 8) & 3
}

/// Native tile colour number (0..=3) at a background-map pixel coordinate.
pub fn colour_at(x: u8, y: u8) -> u8 {
    let tile = &TILES[tile_at(x, y) as usize];
    let row = usize::from(y % 8) * 2;
    let bit = 7 - (x % 8);
    ((tile[row] >> bit) & 1) | (((tile[row + 1] >> bit) & 1) << 1)
}

/// Display registers prepared together by a native step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    pub scx: u8,
    pub scy: u8,
    pub bgp: u8,
}

impl Display {
    /// Shade index for a screen pixel: 0 is lightest, 3 is darkest.
    ///
    /// Coordinates wrap like the 8-bit background scroll registers; callers
    /// may restrict them to the native LCD dimensions when sampling the LCD.
    pub fn shade_at(self, x: u8, y: u8) -> u8 {
        let colour = colour_at(x.wrapping_add(self.scx), y.wrapping_add(self.scy));
        (self.bgp >> (colour * 2)) & 3
    }
}

/// Native-only counter and VBlank register pipeline.
///
/// Starts at the first ApplyDisplay, with the cleared shadow scroll and
/// initial BGP. Each tick applies the previous shadow registers, then runs
/// NativeStep. The counter advances only when this native step is run.
#[derive(Clone, Debug)]
pub struct DmgTileScroll {
    counter: u8,
    shown: Display,
    prepared: Display,
}

impl Default for DmgTileScroll {
    fn default() -> Self {
        let initial = Display {
            scx: 0,
            scy: 0,
            bgp: 0xe4,
        };
        Self {
            counter: 0,
            shown: initial,
            prepared: initial,
        }
    }
}

impl DmgTileScroll {
    pub fn counter(&self) -> u8 {
        self.counter
    }

    pub fn shown(&self) -> Display {
        self.shown
    }

    pub fn prepared(&self) -> Display {
        self.prepared
    }

    /// Apply the previous output at VBlank, then prepare the next native step.
    pub fn tick(&mut self) {
        self.shown = self.prepared;
        self.counter = self.counter.wrapping_add(1);
        let scx = self.counter.rotate_right(1);
        self.prepared = Display {
            scx,
            scy: self.counter,
            bgp: (scx & 0x18) | 0xe4,
        };
    }
}
