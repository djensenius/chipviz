//! Stand-alone, non-selectable NES Diamond Scroll study (issue #176).
//!
//! The indexed picture and accepted-NMI state follow `cores/nes/homebrew/src/main.s`,
//! not a wall clock. Reset clears Frame, loads both vertically adjacent
//! nametables and the palettes, and starts at scroll (0, 0). An accepted NMI
//! increments Frame even outside the demo; a busy NMI does nothing. In demo
//! mode it updates $3f01 at multiples of 32 and writes (0, Frame >> 1) to
//! the scroll registers. Input can select another scene, but does not drive
//! the demo's pattern, palette or scroll.
//!
//! `queue_reentry` represents SceneExit in the *logic* portion of an accepted
//! NMI: it captures that Frame's foreground and queues restoration for the
//! next accepted NMI, without resetting Frame. The previous scene remains
//! visible until then. Debouncing, field/warning rendering, and exact PPU
//! timing are deliberately not modelled.
//!
//! This is an indexed, square-pixel, nearest-neighbour offscreen design, not
//! emulator parity. It omits analogue colour, overscan and display aspect
//! correction. The optional RGB mapping below is an illustrative hand-tuned
//! sRGB approximation, anchored by the repository's 2C02 reference examples
//! in `shared/specs/colour-field-v0.md` ($12 = 0058f8, $19 = 00b800).
//! Other entries are chosen approximations, not measured NES output. Identical
//! encoded bytes are intended for UNORM and sRGB attachments; an eventual
//! sRGB GPU shader must decode them to linear light before attachment writes.
//! Colour and scroll transitions have not been assessed for photosensitivity.

pub const WIDTH: u32 = 256;
pub const HEIGHT: u32 = 240;
const MAX_VIEWPORT: u32 = 8192;

const FOREGROUND: [u8; 8] = [0x11, 0x12, 0x13, 0x14, 0x15, 0x14, 0x13, 0x12];
const PALETTES: [[u8; 4]; 4] = [
    [0x0f, 0x11, 0x21, 0x31],
    [0x0f, 0x05, 0x15, 0x25],
    [0x0f, 0x09, 0x19, 0x29],
    [0x0f, 0x0c, 0x1c, 0x2c],
];
const CHR_LOW: [u8; 8] = [0x00, 0x18, 0x3c, 0x7e, 0xff, 0x7e, 0x3c, 0x18];
const CHR_HIGH: [u8; 8] = [0x00, 0x00, 0x18, 0x3c, 0x7e, 0x3c, 0x18, 0x00];

#[derive(Clone, Debug)]
pub struct DiamondScroll {
    frame: u8,
    foreground: u8,
    scroll_y: u8,
    demo_visible: bool,
    queued_foreground: Option<u8>,
}

impl Default for DiamondScroll {
    fn default() -> Self {
        Self {
            frame: 0,
            foreground: 0x11,
            scroll_y: 0,
            demo_visible: true,
            queued_foreground: None,
        }
    }
}

impl DiamondScroll {
    pub fn frame(&self) -> u8 {
        self.frame
    }

    pub fn scroll_y(&self) -> u8 {
        self.scroll_y
    }

    pub fn foreground(&self) -> u8 {
        self.foreground
    }

    /// Marks the demo no longer visible. Non-demo accepted NMIs still count.
    pub fn leave_demo(&mut self) {
        self.demo_visible = false;
        self.queued_foreground = None;
    }

    /// Queue the SceneExit restoration after this frame's NMI uploads/PPU work.
    /// Call only after leaving the demo and completing the relevant logic step.
    pub fn queue_reentry(&mut self) {
        assert!(!self.demo_visible && self.queued_foreground.is_none());
        self.queued_foreground = Some(FOREGROUND[usize::from(self.frame >> 5)]);
    }

    /// Advance exactly one accepted NMI, or leave all state untouched if busy.
    pub fn nmi(&mut self, accepted: bool) {
        if !accepted {
            return;
        }
        self.frame = self.frame.wrapping_add(1);
        if let Some(foreground) = self.queued_foreground.take() {
            self.foreground = foreground;
            self.demo_visible = true;
        }
        if self.demo_visible {
            if self.frame & 31 == 0 {
                self.foreground = FOREGROUND[usize::from(self.frame >> 5)];
            }
            self.scroll_y = self.frame >> 1;
        }
    }

    /// NES palette index at a visible native pixel; None outside the demo or
    /// outside 256x240. Both nametables start with the same demo attributes.
    pub fn pixel(&self, x: u32, y: u32) -> Option<u8> {
        if !self.demo_visible || x >= WIDTH || y >= HEIGHT {
            return None;
        }
        let nametable_y = (y + u32::from(self.scroll_y)) % HEIGHT;
        let row = (nametable_y % 8) as usize;
        let bit = 7 - x % 8;
        let value = ((CHR_LOW[row] >> bit) & 1) | (((CHR_HIGH[row] >> bit) & 1) << 1);
        if value == 0 {
            return Some(0x0f);
        }
        // $e4: top-left 0, top-right 1, bottom-left 2, bottom-right 3.
        let palette = ((x / 16) % 2 + 2 * ((nametable_y / 16) % 2)) as usize;
        Some(if palette == 0 && value == 1 {
            self.foreground
        } else {
            PALETTES[palette][usize::from(value)]
        })
    }
}

/// Hand-tuned sRGB bytes for the indices used by the ROM's demo, including
/// currently unused colour-2 entries. No display calibration is implied.
pub fn approximate_srgb(index: u8) -> Option<[u8; 3]> {
    Some(match index {
        0x0f => [0, 0, 0],
        0x11 => [0, 120, 248],
        0x12 => [0, 88, 248],
        0x13 => [104, 68, 252],
        0x14 => [216, 0, 204],
        0x15 => [228, 0, 88],
        0x21 => [60, 188, 252],
        0x31 => [164, 228, 252],
        0x05 => [172, 0, 124],
        0x25 => [248, 88, 152],
        0x09 => [0, 88, 0],
        0x19 => [0, 184, 0],
        0x29 => [88, 216, 84],
        0x0c => [0, 64, 88],
        0x1c => [0, 232, 216],
        0x2c => [56, 248, 248],
        _ => return None,
    })
}

/// Centered square-NES-pixel viewport, integer-scaled where possible. Smaller
/// targets fit the native aspect with nearest-neighbour downsampling.
/// Dimensions are bounded to avoid unbounded offscreen resource requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl Viewport {
    pub fn fit(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 || width > MAX_VIEWPORT || height > MAX_VIEWPORT {
            return None;
        }
        let scale = (width / WIDTH).min(height / HEIGHT);
        let (w, h) = if scale > 0 {
            (WIDTH * scale, HEIGHT * scale)
        } else if u64::from(width) * u64::from(HEIGHT) <= u64::from(height) * u64::from(WIDTH) {
            (width, (width * HEIGHT / WIDTH).max(1))
        } else {
            ((height * WIDTH / HEIGHT).max(1), height)
        };
        Some(Self {
            x: (width - w) / 2,
            y: (height - h) / 2,
            width: w,
            height: h,
        })
    }

    pub fn bounds(&self) -> (u32, u32, u32, u32) {
        (self.x, self.y, self.width, self.height)
    }

    /// Map an output pixel to its source pixel, or None in the black bars.
    pub fn source_pixel(&self, x: u32, y: u32) -> Option<(u32, u32)> {
        if x < self.x || y < self.y || x - self.x >= self.width || y - self.y >= self.height {
            return None;
        }
        Some((
            (x - self.x) * WIDTH / self.width,
            (y - self.y) * HEIGHT / self.height,
        ))
    }
}
