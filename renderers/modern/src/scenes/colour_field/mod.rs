//! Scene 7: the colour field (`shared/specs/colour-field-v0.md`).
//!
//! Two or three large regions of flat, primary-aligned colour for projecting
//! onto performers in the dark. The scene is stateful, so it is not a
//! [`REGISTRY`](super::REGISTRY) entry: `scene == 7` is checked before the
//! registry mapping (see [`super::dispatch`]). [`state`] holds the
//! deterministic step function; this module only turns a [`Trace`] into
//! instances.

mod state;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::time::Duration;

pub use state::{ColourField, Config, Controls, Display, PaletteSource, SCENE, Trace, regions};

use crate::scene::Instance;

pub const NAME: &str = "ColourField";

/// Shown on stderr when `--allow-flash` is given.
pub const FLASH_WARNING: &str = "WARNING: Flash/gate mode produces full-field flashing light at up to 7.5 flashes per second. This exceeds the WCAG 2.3.1 and Harding limits of 3 flashes per second. Saturated red flashes are the highest-risk case. It may trigger seizures in people with photosensitive epilepsy. Enable it only where performers and audience have been warned.";

const A_RED: [u8; 3] = [0xE0, 0x10, 0x20];
const A_YELLOW: [u8; 3] = [0xFF, 0xD0, 0x00];
const A_BLUE: [u8; 3] = [0x10, 0x30, 0xC0];
const WHITE: [u8; 3] = [0xFF; 3];
const BLACK: [u8; 3] = [0x00; 3];
const B_RED: [u8; 3] = [0xFF, 0x00, 0x00];
const B_GREEN: [u8; 3] = [0x00, 0xFF, 0x00];
const B_BLUE: [u8; 3] = [0x00, 0x00, 0xFF];
const B_CYAN: [u8; 3] = [0x00, 0xFF, 0xFF];
const B_MAGENTA: [u8; 3] = [0xFF, 0x00, 0xFF];
const B_YELLOW: [u8; 3] = [0xFF, 0xFF, 0x00];

/// `[family][combo]` colours for 2 and 3 regions.
const COMBOS_2: [[[[u8; 3]; 2]; 4]; 2] = [
    [
        [A_RED, A_BLUE],
        [A_YELLOW, A_BLUE],
        [A_RED, A_YELLOW],
        [A_BLUE, WHITE],
    ],
    [
        [B_RED, B_CYAN],
        [B_GREEN, B_MAGENTA],
        [B_BLUE, B_YELLOW],
        [B_CYAN, B_MAGENTA],
    ],
];
const COMBOS_3: [[[[u8; 3]; 3]; 4]; 2] = [
    [
        [A_RED, A_YELLOW, A_BLUE],
        [A_RED, WHITE, A_BLUE],
        [A_YELLOW, BLACK, A_BLUE],
        [A_RED, BLACK, A_YELLOW],
    ],
    [
        [B_RED, B_GREEN, B_BLUE],
        [B_CYAN, B_MAGENTA, B_YELLOW],
        [B_RED, BLACK, B_CYAN],
        [B_BLUE, BLACK, B_YELLOW],
    ],
];

/// The sRGB colour of each region (unused regions are black): region `i`
/// shows combo entry `(i + rot) mod n`, scaled per channel to
/// `(c * L + 1) / 3`.
pub fn region_colours(trace: &Trace) -> [[u8; 3]; 3] {
    let family = usize::from(trace.family & 1);
    let combo = usize::from(trace.combo & 3);
    let n = usize::from(regions(trace.layout));
    let entry = |index: usize| match n {
        2 => COMBOS_2[family][combo][index % 2],
        _ => COMBOS_3[family][combo][index % 3],
    };
    std::array::from_fn(|region| {
        if region >= n {
            return BLACK;
        }
        let level = u16::from(trace.lvl[region].min(3));
        entry(region + usize::from(trace.rot))
            .map(|channel| ((u16::from(channel) * level + 1) / 3) as u8)
    })
}

/// Region boundaries along an axis of `size` pixels (`g = 1`):
/// `b_i = (i * size) / n`, with `b_n = size`.
pub fn boundaries(size: u32, n: u8) -> Vec<u32> {
    (0..=u64::from(n))
        .map(|index| (index * u64::from(size) / u64::from(n)) as u32)
        .collect()
}

fn srgb_to_linear(channel: u8) -> f32 {
    let value = f32::from(channel) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// One flat `ScreenSpace` quad per region, covering the whole viewport of
/// `width` x `height` pixels with pixel-exact boundaries. Columns run from
/// the left and rows from the bottom.
pub fn build(trace: &Trace, width: u32, height: u32, out: &mut Vec<Instance>) {
    let n = regions(trace.layout);
    let vertical = trace.layout.is_multiple_of(2);
    let size = if vertical { width } else { height };
    let edges = boundaries(size.max(1), n);
    let colours = region_colours(trace);
    for region in 0..usize::from(n) {
        let to_ndc = |pixel: u32| 2.0 * pixel as f32 / size.max(1) as f32 - 1.0;
        let (start, end) = (to_ndc(edges[region]), to_ndc(edges[region + 1]));
        let (center, extent) = ((start + end) / 2.0, end - start);
        let [red, green, blue] = colours[region].map(srgb_to_linear);
        out.push(Instance {
            center: if vertical {
                [center, 0.0, 0.0]
            } else {
                [0.0, center, 0.0]
            },
            size: if vertical {
                [extent, 2.0]
            } else {
                [2.0, extent]
            },
            color: [red, green, blue, 1.0],
            rotation: 0.0,
            scene_mix: 0.0,
            pattern: [0.0; 2],
        });
    }
}

/// Enforces at most 60 colour-field steps in any one-second window; a
/// renderer that falls behind drops steps rather than catching up.
#[derive(Debug, Default)]
pub struct StepClock {
    recent: VecDeque<Duration>,
}

impl StepClock {
    pub const MAX_STEPS_PER_SECOND: usize = 60;

    /// Records and allows a step at `now` (monotonic) if fewer than 60 steps
    /// fall in the preceding second.
    pub fn try_step(&mut self, now: Duration) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|&time| now.saturating_sub(time) >= Duration::from_secs(1))
        {
            self.recent.pop_front();
        }
        if self.recent.len() >= Self::MAX_STEPS_PER_SECOND {
            return false;
        }
        self.recent.push_back(now);
        true
    }
}
