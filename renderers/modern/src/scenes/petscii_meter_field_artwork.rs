//! Unregistered CPU-only artwork for the logical meter field.
//!
//! These original geometric marks are an artistic approximation, not C64
//! hardware pixel parity or a character ROM. Indices are synthetic; no
//! measured colours, display output, or safety conclusions are implied.

use super::petscii_meter_field::{Cell, HEIGHT, PetsciiMeterField, WIDTH};

pub const CELL_SIZE: usize = 8;
pub const PIXEL_WIDTH: usize = WIDTH * CELL_SIZE;
pub const PIXEL_HEIGHT: usize = HEIGHT * CELL_SIZE;

/// A known background is distinct from an untouched cell or an unknown code.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Pixel {
    UnknownCell,
    UnknownCode(u8),
    Background,
    Foreground(u8),
}

/// Compose one logical cell pixel. Coordinates outside the cell return `None`.
/// Foreground indices are the low four bits, including zero.
pub fn cell_pixel(cell: Option<Cell>, x: usize, y: usize) -> Option<Pixel> {
    if x >= CELL_SIZE || y >= CELL_SIZE {
        return None;
    }
    let Some(cell) = cell else {
        return Some(Pixel::UnknownCell);
    };
    let filled = match cell.code {
        32 => false,
        // An eight-direction ray emerging from a small central square.
        81..=88 => {
            let core = (3..=4).contains(&x) && (3..=4).contains(&y);
            let ray = match cell.code - 81 {
                0 => x == 3 && y <= 3,
                1 => x == y && x <= 3,
                2 => y == 3 && x <= 3,
                3 => x + y == 7 && x <= 3,
                4 => x == 4 && y >= 4,
                5 => x == y && x >= 4,
                6 => y == 4 && x >= 4,
                7 => x + y == 7 && x >= 4,
                _ => unreachable!(),
            };
            core || ray
        }
        // A solid meter is deliberately recognizable among the field marks.
        102 => true,
        96..=111 => {
            let variant = usize::from(cell.code - 96);
            let shift = (variant % 4) * 2;
            match variant / 4 {
                0 => (x + y + shift) % 8 < 2,
                1 => (x + 7 - y + shift) % 8 < 2,
                2 => (x + shift) % 8 < 2,
                3 => (y + shift) % 8 < 2,
                _ => unreachable!(),
            }
        }
        code => return Some(Pixel::UnknownCode(code)),
    };
    Some(if filled {
        Pixel::Foreground(cell.colour & 15)
    } else {
        Pixel::Background
    })
}

/// Compose a pixel from the current logical model, without rendering a display.
/// Out-of-bounds coordinates return `None`; untouched cells remain unknown.
pub fn pixel(field: &PetsciiMeterField, x: usize, y: usize) -> Option<Pixel> {
    if x >= PIXEL_WIDTH || y >= PIXEL_HEIGHT {
        return None;
    }
    cell_pixel(
        field.cell(x / CELL_SIZE, y / CELL_SIZE),
        x % CELL_SIZE,
        y % CELL_SIZE,
    )
}
