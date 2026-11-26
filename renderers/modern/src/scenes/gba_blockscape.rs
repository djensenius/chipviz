//! Source-derived GBA Blockscape composition, not a selectable scene.
//!
//! The caller supplies the native 16-bit animation counter and already
//! selected demo state. This does not reproduce input acquisition, page flips,
//! VBlank timing, hardware colour, or the native Colour Field scene. The
//! native beat/key overlays can flash; the output is not approved to present.

pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 160;
pub const PIXELS: usize = WIDTH * HEIGHT;
const BLOCK_SIZE: usize = 4;
const BLOCK_COLUMNS: usize = WIDTH / BLOCK_SIZE;
const BLOCK_ROWS: usize = HEIGHT / BLOCK_SIZE;

pub const KEY_A: u16 = 0x0001;
pub const KEY_B: u16 = 0x0002;
pub const KEY_SELECT: u16 = 0x0004;
pub const KEY_START: u16 = 0x0008;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DemoState {
    pub spectrum: [u8; 8],
    pub scene: u8,
    pub palette: u8,
    pub energy: u8,
    pub beat: bool,
    pub note: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub indices: Vec<u8>,
    pub palette: [u16; 256],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposeError {
    InvalidScene(u8),
    InvalidPalette(u8),
}

fn colour_index(r: u32, g: u32, b: u32) -> u8 {
    ((r >> 2) | ((g >> 2) << 3) | ((b >> 3) << 6)) as u8
}

fn make_palette(frame: u16, state: DemoState) -> [u16; 256] {
    let mut palette = [0; 256];
    let warm = (u32::from(state.palette) * 3 + u32::from(frame >> 3)) & 7;
    for (i, colour) in palette.iter_mut().enumerate() {
        let i = i as u32;
        let r = ((i & 7) * 4 + warm) & 31;
        let g = (((i >> 3) & 7) * 4 + u32::from(state.scene) * 3) & 31;
        let b = (((i >> 6) & 3) * 10 + u32::from(state.energy >> 4)) & 31;
        *colour = (r | (g << 5) | (b << 10)) as u16;
    }
    palette[usize::from(colour_index(31, 31, 31))] = 0x7fff;
    palette
}

fn block_colour(bx: usize, by: usize, frame: u16, state: DemoState) -> u8 {
    let band = u32::from(state.spectrum[((bx >> 3) + (by >> 2)) & 7] >> 5);
    let bx = bx as u32;
    let by = by as u32;
    let phase = (u32::from(frame >> 1) + bx * 3 + by * 5 + u32::from(state.palette) * 7) & 31;
    let mut r = (phase + band + (bx >> 1)) & 31;
    let mut g = (by + band * 3 + u32::from(state.scene) * 4) & 31;
    let mut b = ((bx ^ by) + u32::from(state.palette) * 2) & 31;

    match state.scene {
        1 => {
            let cx = bx as i32 - (BLOCK_COLUMNS / 2) as i32;
            let cy = by as i32 - (BLOCK_ROWS / 2) as i32;
            let tunnel = (((cx * cx + cy * cy) >> 1) as u32
                + u32::from(frame >> 2)
                + u32::from(state.beat) * 5) as u8;
            r = (u32::from(tunnel) + u32::from(state.energy >> 3)) & 31;
            g = (u32::from(tunnel >> 1) + u32::from(state.palette) * 3) & 31;
            b = 31 - ((u32::from(tunnel) + band) & 31);
        }
        2 => {
            let stripe =
                (bx + u32::from(frame >> 2) + u32::from(state.spectrum[(by & 7) as usize] >> 4))
                    & 31;
            r = (stripe + u32::from(state.palette) * 2) & 31;
            g = (31 - stripe + band * 2) & 31;
            b = (phase + (stripe >> 1)) & 31;
        }
        3 => {
            let star = (bx * 11 + by * 17 + u32::from(frame) * 3 + u32::from(state.note)) & 63;
            r = if star < 3 { 31 } else { (phase + band) & 15 };
            g = if star < 5 {
                16 + band * 2
            } else {
                (u32::from(state.palette) * 4 + by) & 31
            };
            b = if star < 7 {
                31
            } else {
                (bx + u32::from(state.energy >> 3)) & 31
            };
        }
        _ => {}
    }

    if bx & 7 == 0 || by & 7 == 0 {
        r = 24 + u32::from(state.beat) * 7;
        g = (band * 4 + u32::from(state.palette) * 2) & 31;
        b = phase;
    }
    colour_index(r, g, b)
}

fn fill_rect(indices: &mut [u8], x: usize, y: usize, width: usize, height: usize, colour: u8) {
    if x >= WIDTH || y >= HEIGHT || width == 0 || height == 0 {
        return;
    }
    // Mode 4 writes halfwords, so odd rectangles expand to whole pixel pairs.
    let left = x & !1;
    let right = (x + width).min(WIDTH).div_ceil(2) * 2;
    for row in indices.chunks_exact_mut(WIDTH).skip(y).take(height) {
        row[left..right].fill(colour);
    }
}

fn render_overlay(indices: &mut [u8], frame: u16, state: DemoState, keys: u16) {
    for x in 0..8 {
        let height = 10 + usize::from(state.spectrum[x] >> 2);
        let bar = colour_index(
            31,
            ((x as u32 * 3) + u32::from(state.palette)) & 31,
            31 - x as u32 * 3,
        );
        let tick = colour_index(u32::from(state.energy >> 3), 31, 31);
        fill_rect(indices, 16 + x * 26, 150 - height, 16, height, bar);
        fill_rect(
            indices,
            20 + x * 25,
            32 + ((usize::from(frame) + x * 13) & 63),
            12,
            4,
            tick,
        );
    }
    let white = colour_index(31, 31, 31);
    if state.beat {
        fill_rect(indices, 0, 0, WIDTH, 4, white);
        fill_rect(indices, 0, HEIGHT - 4, WIDTH, 4, white);
    }
    if keys & KEY_START != 0 {
        fill_rect(indices, 0, 78, WIDTH, 4, white);
    }
    if keys & KEY_SELECT != 0 {
        fill_rect(indices, 118, 0, 4, HEIGHT, white);
    }
    if keys & KEY_A != 0 {
        for y in (0..HEIGHT).step_by(12) {
            fill_rect(indices, 0, y, WIDTH, 2, colour_index(31, 31, 0));
        }
    }
    if keys & KEY_B != 0 {
        for x in (0..WIDTH).step_by(12) {
            fill_rect(indices, x, 0, 2, HEIGHT, colour_index(0, 31, 31));
        }
    }
}

pub fn compose(frame: u16, state: DemoState, keys: u16) -> Result<Frame, ComposeError> {
    if state.scene > 3 {
        return Err(ComposeError::InvalidScene(state.scene));
    }
    if state.palette > 7 {
        return Err(ComposeError::InvalidPalette(state.palette));
    }
    let mut indices = vec![0; PIXELS];
    for by in 0..BLOCK_ROWS {
        for bx in 0..BLOCK_COLUMNS {
            let colour = block_colour(bx, by, frame, state);
            for y in by * BLOCK_SIZE..(by + 1) * BLOCK_SIZE {
                let offset = y * WIDTH + bx * BLOCK_SIZE;
                indices[offset..offset + BLOCK_SIZE].fill(colour);
            }
        }
    }
    render_overlay(&mut indices, frame, state, keys);
    Ok(Frame {
        indices,
        palette: make_palette(frame, state),
    })
}
