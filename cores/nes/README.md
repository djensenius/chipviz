# chipviz-nes

NES visualizer for the 2A03 ChipStation side.

## Homebrew stack

- cc65/ca65 for the first real iNES/NROM artifact.
- PPU nametable, attribute, CHR tile, palette, and sprite/OAM-style effects.

## First demo

- Self-running NROM demo with pulse/triangle/noise-inspired bars and scrolling
  tile fields.
- Controller input selects the colour field (scene 7) and its palette.

## Colour field (scene 7)

`cores/nes/homebrew/src/main.s` implements the target role of
`shared/specs/colour-field-v0.md`: layouts V2, H2, V3 and H3 filling the
screen, held, spectral and hard-cut modes, the change budget, the P_STABLE
palette gate, the scene-7 window and the 30-step scene debounce. Flash/gate
(modes 3 and 7) is opt-in only and exists only in the operator build.

One step runs per NMI. The NMI first uploads the attribute table (64 bytes at
`$23C0`, the only thing a layout change touches) and the 16-byte palette, then
reads the pad and runs the step that prepares the next upload, so a change
never tears. Region *i* uses background palette *i*; every background tile is
solid colour 1, and the left column is not masked.

### NES colour indices

The spec's NES column, used unchanged (`$0D` is never written):

| id | colour  | level 3 | level 2 |
|----|---------|---------|---------|
| R  | red     | `$16`   | `$06`   |
| Y  | yellow  | `$28`   | `$18`   |
| B  | blue    | `$12`   | `$02`   |
| W  | white   | `$30`   | `$10`   |
| K  | black   | `$0F`   | `$0F`   |
| G  | green   | `$19`   | `$09`   |
| C  | cyan    | `$3C`   | `$2C`   |
| M  | magenta | `$14`   | `$04`   |

Levels 1 and 0 use `$0F`. Family A (palettes 0-3) is R/Y/B/W/K and family B
(palettes 4-7) is R/G/B/C/M/Y/K; the combos are the spec's tables. Actual RGB
depends on the emulator or TV palette, so tests check index-to-RGB consistency,
not fixed RGB.

### Input

Controller 1 drives a receiver-side control frame each step:

| button      | meaning |
|-------------|---------|
| Up / Down   | scene ring {0 native demo, 7 colour field} |
| Right / Left| palette ring 0-7 (pressed edges; opposite directions cancel) |
| B / A       | beat / bar flag |
| Select      | operator build: layout pin auto, V2, H2, V3, H3 |
| Start       | operator build: confirm the flashing warning |

Until B or A is first pressed the detector uses the procedural generator (a
port of `chipviz_frame_make_procedural`; its scene and palette are ignored).
After that the pad is a digital-only source: energy and every band are fixed at
`HID_E_LO` (64), like the C64 joystick, because the pad has no analogue input.

- **Data role** (default, `chipviz-nes.nes`): scene changes are debounced
  (30 stable steps) and the palette ring is a relative wire value that goes
  through P_STABLE, so it never arms. Modes 3 and 7 run as 2 and 6.
- **Operator role** (`make INPUT_ROLE=operator`, `chipviz-nes-operator.nes`):
  scene presses apply at once; palette presses are operator pulses. Choosing
  3 or 7 while unarmed shows the photosensitivity warning: Start arms, any
  other press cancels (runs unarmed as mode 2/6), and it times out after 600
  steps. Pulses last 4 steps and start at least 8 steps apart.

Not implemented on the NES: the interface-v0 screens, the HID auto/demo/manual
combos and the `CUT_MIN` floor on scene changes (every scene change is already
debounced, or exempt in the operator role).

### Checks

`tests/test_nes_colour_field.py` drives the Mesen libretro core headlessly
through `scripts/nes_libretro_capture.py`, reads the trace from CPU RAM (via
the ld65 label files) and checks every pixel of each layout against the spec
boundaries and colours. Captures go to `build/nes-colour-field/`. The tests
skip without cc65 or the Mesen core (`mesen_libretro` in `RETROARCH_CORES`, or
the path in `NES_LIBRETRO_CORE`).

```sh
python3 scripts/nes_libretro_capture.py --rom cores/nes/homebrew/chipviz-nes.nes --frames 120 --png out.png
```

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
NES-style nametable scroll, pulse height, triangle step, and noise gate values.
`cores/nes/homebrew` contains the ca65 project for `chipviz-nes.nes`.

## Connection path

Start with baked/procedural playback and controller input. Live telemetry can
come later through a flashcart/dev adapter or reduced controller-port bridge.

## Constraints

Stay inside NROM-style limits first: small CHR, fixed palettes, nametable
animation, and sparse sprites.
