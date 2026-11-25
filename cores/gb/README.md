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
After that the pad input uses a digital-only mapping: energy and every band
are `HID_E_LO` (64) and the spectrum is 0, because the DMG pad has no shoulder
or analogue input. The pad is a local controller, not a USB HID receiver. The
spec reserves the operator ROM for a human at the controls and forbids feeding
it from USB HID, so route any HID input to the data build (`chipviz-gb.gb`)
only.

- **Data role** (default, `chipviz-gb.gb`): scene changes are debounced (30
  stable steps), and the palette ring is a relative wire value that goes
  through P_STABLE, so it never arms (F24). Modes 3 and 7 run as 2 and 6.
- **Operator role** (`make INPUT_ROLE=operator`, `chipviz-gb-operator.gb`):
  scene presses apply at once; palette presses are operator pulses. Choosing 3
  or 7 while unarmed shows the photosensitivity warning: Start arms (only a
  fresh press made after the warning was drawn), any other press cancels (runs
  unarmed as mode 2/6), and it times out after 600 steps. A palette or Select
  press on the scene-entry step applies to the entry (an unarmed 3 or 7 opens
  the warning); palette presses in a native scene only set the entry palette.
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

### Isolated Lines scene

`homebrew/lines.asm` is not included in the ROM or selected by the existing
controls. Its mirrored plane uses receding grid bars; its hallway uses nested
frames. Energy changes vertical scroll speed, while the grayscale palette
stays fixed. It does not implement the modern renderer's continuous fold,
spectral lines or beat-count sections. The pad has no spectrum, and there is
no GB beat-count or BPM input. A caller requests a look; changes within one
visit are at least 240 steps apart, including the first after load. These
changes are still abrupt map switches, not an animated fold. A gradual
transition needs a safe VRAM ownership and VBlank update design. No control
mapping or automatic selection is defined here.

`LinesLoad` requires the LCD off and copies 32 2bpp tiles (512 bytes) to
`$8000` and two 1024-byte maps to `$9800` and `$9C00`. The assembled scene
uses 4 bytes of state and 2560 bytes of asset ROM; the current RGBDS object
has 164 bytes of code before linking. `LinesStep` takes the requested look in
A and energy in B and prepares the existing four display shadows; it makes
no VRAM writes. The existing VBlank path would write those four registers.

This is **not** an integration protocol: loading destroys native tiles and
both native and colour-field maps, including the operator warning. The
future adapter must safely load and restore all overwritten VRAM, preserve
the scene-7 debounce, warning and data/operator palette gates, and bound
transitions across visits (reloading resets the look guard). It must not
repurpose controls before #140. Disabling the LCD or switching scenes may
itself cause an unmeasured full-field change. Runtime cycle/VBlank budgets,
actual ROM placement and RAM coexistence also require verification.

`tests/test_gb_lines.py` compares assembled tile and map bytes against a
pixel oracle and checks all three source configurations. The test-only
`tests/fixtures/gb_lines_execution.asm` links the actual scene into a separate
mailbox fixture; it is not included in production. Its execution tests check
the look hold, speed boundaries, scroll, display shadows and reload. The
fixture deliberately corrupts the Lines VRAM before reloading with the LCD
off; it does not implement safe restoration of the other scenes. A test-only
present command writes the prepared BGP, scroll and LCDC shadows immediately
after waiting for VBlank, without writing VRAM. A prepared pixel test waits
two frames after each present, then compares plane and hallway frames against
a host decoder of the fixture's copied tile/map bytes and actual shadows.
The decoder assumes unsigned BG tiles, no window or sprites, and SameBoy's
fixed DMG greyscale shades (255, 170, 85, 0). Fixture build and host-oracle
checks run without an emulator in the RGBDS-equipped Game Boy ROM CI job;
the Python job skips RGBDS-dependent tests, and CI skips SameBoy execution.
When a SameBoy libretro core is available and the emulator slot is free, run:

```sh
CHIPVIZ_GB_LINES_EXECUTION=1 python3 -m unittest discover -s tests -p test_gb_lines_execution.py -v
```

The separate roundtrip fixture interposes on the real routines for both roles,
restores native VRAM and prepared registers, and compares native pixels before
and after a Lines visit. Its still-frame comparisons freeze all steps,
including scene-7 clocks and budgets; they do not compare active colour-field
restoration pixels. The signed-tile fixture stages repeated diagnostic stripes
with the LCD off, not Lines artwork, and checks native restoration, VRAM
ownership and field/warning priority. Its inspection also uses LCD-off
staging and its still comparisons freeze steps and budgets.

These fixtures and the separate present command and frame delay are test
assumptions, not evidence of a production handoff, live VBlank staging or
timing, visual safety, or physical hardware behavior. Production dispatch,
VRAM ownership/restoration and physical timing remain unverified. Integration,
flash measurement and hardware testing remain necessary; this is not a
physical-hardware safety certification.

## Connection path

Start with baked/procedural playback and Pocket Dock controller input. A link
cable bridge remains a later richer live-input path.

## Constraints

Use the hardware the way the DMG wants: tiny tile maps, four shades, cheap
scrolling, and sparse sprite/meters rather than framebuffer effects.
