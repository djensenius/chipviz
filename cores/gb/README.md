# chipviz-gb

Original Game Boy / DMG visualizer for the Game Boy APU side of ChipStation.

## Homebrew stack

- RGBDS for the first real `.gb` artifact. The repo does not hand-embed the
  Nintendo logo bytes; `rgbfix` writes a valid header during the SDK build.
- DMG tile maps, four-shade palette cycling, scrolling backgrounds, and small
  sprite/channel meters.

## First demo

- Self-running DMG tile-field visualizer with beat-synced scrolling.
- The pad selects the colour field (scene 7) and its palette.
- Later: Pocket Dock USB HID or link-cable adapter for reduced live controls.

## Colour field (scene 7)

`cores/gb/homebrew/colour_field.asm` implements the target role of
`shared/specs/colour-field-v0.md` with the DMG gating variant: layouts V2,
H2, V3 and H3 filling the screen, held, spectral and hard-cut modes, the
change budget, the P_STABLE palette gate, the scene-7 window and the 30-step
scene debounce. Flash/gate (modes 3 and 7) is opt-in only and reachable only
in the operator build.

One step runs per frame, after VBlank. VRAM is written once at power-on and
never again: one BG map at `$9C00` holds tile `16 + 4 * row class + column
class`, and tiles 16-31 are solid. In `$8000` tile mode (LCDC bit 4 set) a
tile's colour is its column class; in `$8800` mode the same tile numbers
read `$9100-$91FF`, whose colour is the row class. A layout and its shades
are therefore just `LCDC`, `BGP` and scroll, written together at the next
VBlank, so a change never tears.

| class | columns (x) | rows (y, from the top) |
|-------|-------------|------------------------|
| 0     | 0-47        | 0-47                   |
| 1     | 48-79       | 48-71                  |
| 2     | 80-103      | 72-95                  |
| 3     | 104-159     | 96-143                 |

This gives the spec boundaries for 160 x 144 with `g = 8`: V2 at x = 80, V3 at
48 and 104, H2 at 72 px from the bottom, H3 at 48 and 96 from the bottom
(region 0 is the bottom row).

Step *t* renders after the mode rule and before step 7's requests, so a
request (or a scene exit) decided at the end of *t* shows from *t + 1*.
`wTrace` (`$C001`) holds the rendered trace (mode, family, layout, combo, rot,
levels), and `$C009` the copy taken with the registers at VBlank.

### DMG shades

The spec's DMG table, used unchanged. Shade 0 is the lightest and 3 is black;
both families (palettes 0-3 and 4-7) use the same shades. Region *i* shows
entry `(i + rot) mod n`.

| combo | 2 regions | 3 regions |
|-------|-----------|-----------|
| 0     | 0, 3      | 0, 3, 1   |
| 1     | 1, 3      | 3, 0, 2   |
| 2     | 0, 2      | 1, 3, 0   |
| 3     | 3, 1      | 0, 2, 3   |

- Held and hard cuts (level 3): the combo shades.
- Spectral: shade `3 - L` per region (level 3 white, level 0 black).
- Flash/gate: lit shows the combo shades, a dim rest shows every region at
  shade 2, a dark rest (or silence) every region at shade 3.

`BGP` maps each tile colour (class) to its region's shade. SameBoy's DMG
greyscale shows shades 0-3 as `#FFFFFF`, `#AAAAAA`, `#555555` and `#000000`.

### Input

| button       | meaning |
|--------------|---------|
| Up / Down    | scene ring {0 native demo, 7 colour field} |
| Right / Left | palette ring 0-7 (pressed edges; opposite directions cancel) |
| B / A        | beat / bar flag |
| Select       | operator build: layout pin auto, V2, H2, V3, H3 |
| Start        | operator build: confirm the flashing warning |

Until B or A is first pressed the detector uses the procedural generator (a
port of `chipviz_frame_make_procedural`; its scene and palette are ignored).
After that the pad is a digital-only USB HID receiver: energy and every band
are `HID_E_LO` (64) and the spectrum is 0, because the DMG pad has no shoulder
or analogue input.

- **Data role** (default, `chipviz-gb.gb`): scene changes are debounced (30
  stable steps), and the palette ring is a relative wire value that goes
  through P_STABLE, so it never arms (F24). Modes 3 and 7 run as 2 and 6.
- **Operator role** (`make INPUT_ROLE=operator`, `chipviz-gb-operator.gb`):
  scene presses apply at once; palette presses are operator pulses. Choosing 3
  or 7 while unarmed shows the photosensitivity warning: Start arms, any other
  press cancels (runs unarmed as mode 2/6), and it times out after 600 steps.
  Pulses last 4 steps and start at least 8 steps apart.

`just gb-rom` builds both roles. `make FIXTURE=1` builds
`chipviz-gb-fixture.gb` for the emulator tests only: each step takes one
full-path control frame (and scene, palette, local `p`, pin and operator
pulses) from a WRAM mailbox at `$C020`.

### Emulator evidence

`tests/test_gb_colour_field.py` runs the ROMs headlessly in the SameBoy
libretro core (`scripts/gb_libretro_capture.py`, DMG model, greyscale palette,
no border) and skips when RGBDS or the core is missing:

- Fixtures F0-F14, F16-F22 and F24 on the fixture build, checked against
  the spec's Expected column (F19's shoulder energy is fed as values; F24 uses the pad).
  F23 is host-only. F15 and the resampled half of F18 need a 120 Hz source,
  which this target does not have.
- Captures of the data and operator ROMs: every pixel of each frame must be
  its region's expected shade at the spec boundaries. PNGs go to
  `build/gb-colour-field/`.

This is emulator evidence only; it has not been checked on hardware. CI
installs no SameBoy libretro core (and the Python and check jobs have no
RGBDS), so these tests skip there; run them locally, where `just check`
includes them.

The spec's `CUT_MIN` floor after a scene-7 entry or exit is kept by a
since-last-scene-change counter on the debounced path. It cannot bind today:
the DMG has no wire or link-cable scene source, data-role changes already need
30 stable steps, and operator scene presses are exempt. A future debounced
source on the operator build must use the same path.

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
DMG-style scroll/window/sprite-lane parameters for host simulation.
`cores/gb/homebrew` contains the RGBDS project for `chipviz-gb.gb`.

## Connection path

Start with baked/procedural playback and Pocket Dock controller input. A link
cable bridge remains a later richer live-input path.

## Constraints

Use the hardware the way the DMG wants: tiny tile maps, four shades, cheap
scrolling, and sparse sprite/meters rather than framebuffer effects.
