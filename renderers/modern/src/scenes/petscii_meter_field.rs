//! Unregistered logical model of the native C64 PETSCII meter field.
//!
//! Only the seeded grid and subsequent row and meter writes are known here:
//! title, cleared/startup cells and other untouched cells are left unknown.
//! Each step applies rows before meters; skipped steps only advance the frame.
//! IRQ timing, ColourField handoff, keyboard input and display hardware are not
//! modeled. Screen codes are not glyphs; no character ROM or measured colours
//! are supplied, and this makes no pixel-accuracy claim. The native source is
//! `cores/c64/homebrew/src/main.c`; changes there require a separate re-audit.

pub const WIDTH: usize = 40;
pub const HEIGHT: usize = 25;

/// A logical screen code and the effective low four bits of colour RAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub code: u8,
    pub colour: u8,
}

/// Explicit synthetic inputs to the native field update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inputs {
    pub spectrum: [u8; 8],
    pub scene: u8,
    pub palette: u8,
    pub beat: bool,
    pub note: u8,
}

impl Inputs {
    /// Native demo inputs for a given 16-bit frame, independent of hardware.
    pub fn demo(frame: u16) -> Self {
        let frame = u32::from(frame);
        let scene = ((frame / 96) & 3) as u8;
        let mut spectrum = [0; 8];
        for (band, value) in spectrum.iter_mut().enumerate() {
            let wave = (frame * (band as u32 + 3) + band as u32 * 29) & 127;
            *value = (48 + wave.min(127 - wave) + u32::from(scene) * 16) as u8;
        }
        Self {
            spectrum,
            scene,
            palette: ((frame / 32) & 7) as u8,
            beat: frame & 31 < 4,
            note: (48 + ((frame / 16) & 23)) as u8,
        }
    }
}

/// Seeded cells and the frame used by the next step.
#[derive(Clone, Debug)]
pub struct PetsciiMeterField {
    cells: [[Option<Cell>; WIDTH]; HEIGHT],
    frame: u16,
}

impl Default for PetsciiMeterField {
    fn default() -> Self {
        let mut field = Self {
            cells: [[None; WIDTH]; HEIGHT],
            frame: 0,
        };
        for y in 0..18 {
            for x in 0..WIDTH {
                field.cells[y + 4][x] = Some(Cell {
                    code: 96 + ((x + y) & 15) as u8,
                    colour: (1 + ((x / 5 + y) & 15)) as u8 & 15,
                });
            }
        }
        field
    }
}

impl PetsciiMeterField {
    pub fn frame(&self) -> u16 {
        self.frame
    }

    /// Unknown or out-of-bounds cells return `None`.
    pub fn cell(&self, x: usize, y: usize) -> Option<Cell> {
        self.cells
            .get(y)
            .and_then(|row| row.get(x))
            .copied()
            .flatten()
    }

    /// Apply this frame's row pass, then overlay all meters, and advance.
    pub fn step(&mut self, input: &Inputs) {
        let frame = usize::from(self.frame);
        for y in 0..18 {
            if y & 3 != frame & 3 {
                continue;
            }
            for x in 0..WIDTH {
                let band = usize::from(input.spectrum[x / 5]);
                let mut code = 96 + ((x + y + frame / 4 + usize::from(input.scene) * 3) & 15);
                if (x * 3 + y * 5 + frame + band) & 31 < if input.beat { 8 } else { 4 } {
                    code = 81 + usize::from(input.note & 7);
                }
                self.cells[y + 4][x] = Some(Cell {
                    code: code as u8,
                    colour: ((1 + ((band / 16 + usize::from(input.palette) + y) & 15)) & 15) as u8,
                });
            }
        }
        for band in 0..8 {
            let height = 1 + (input.spectrum[band] >> 5);
            for y in 0..5 {
                let cell = Some(Cell {
                    code: if y < height { 102 } else { 32 },
                    colour: ((1 + ((band + usize::from(input.palette)) & 15)) & 15) as u8,
                });
                let x = 2 + band * 4;
                self.cells[23 - usize::from(y)][x] = cell;
                self.cells[23 - usize::from(y)][x + 1] = cell;
            }
        }
        self.skip_frame();
    }

    /// Advance without native writes, as when another field is shown.
    /// The timing and handoff that decide when to skip are outside this model.
    pub fn skip_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }
}
