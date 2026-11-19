# chipviz-c64

Commodore 64 homebrew visualizer for Commodore 64 Ultimate and real C64-style workflows.

## Homebrew stack

- cc65 for the first homebrew C/assembly build, or KickAssembler if raster timing becomes the priority.
- VIC-II raster bars, PETSCII patterns, character animation, sprites, and SID-driven timing experiments.

## First demo

- Raster bar and PETSCII scene.
- Keyboard/joystick scene switching.
- Deterministic baked/procedural control-frame-style playback for self-running
  release assets.
- Later: live bridge input through Commodore 64 Ultimate networking features or a user-port serial adapter.

## Current implementation

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
a minimal C64-style visualization model: beat phase selects a raster position,
scene and palette choose color, and energy chooses a PETSCII cell. The native
simulator build prints those values until the cc65 VIC-II renderer replaces the
stub.
The simulator is not the product; the target artifact is a loadable homebrew
`.prg` and/or `.d64`.
`cores/c64/homebrew` contains the cc65 project for `chipviz-c64.prg`. It writes
directly to border/background color, screen RAM, and color RAM for a self-running
VIC-II/PETSCII/channel-meter demo. `just homebrew-artifacts` still produces an
SDK-free tokenized BASIC fallback when cc65 is not installed.

## Colour field (scene 7)

`src/colour_field.c` implements the [colour-field-v0](../../shared/specs/colour-field-v0.md)
target role in the `data` input role. `src/main.c` runs one step per VBlank
poll and writes all VIC-II changes together at raster line 255, so the screen
never tears.

- **Display.** ECM text mode with the blank glyph (screen code 32). Screen code
  bits 6–7 select `$D021`–`$D023` for regions 0–2. One screen is prepared per
  layout at `$3000`, `$3400`, `$3800` and `$3C00` (V2, H2, V3, H3) at boot.
  A layout change only flips `$D018`; no visible screen matrix is rewritten.
  The Makefile fails the build if the program reaches `$3000`.
- **Geometry.** Cells are 8 pixels (`g = 8`). V2 splits at x = 160, V3 at
  x = 104 and 208. H2 splits 96 px from the bottom, H3 at 64 and 128 px from
  the bottom. Region 0 is left or bottom. The border (`$D020`) stays black.
- **Levels.** Levels 3 and 2 show the region's base colour, levels 1 and 0
  show black.
- **Colours** (VIC-II indices, Pepto RGB):

  | Spec colour | C64 index | Name | Pepto RGB |
  | --- | --- | --- | --- |
  | A.R | 10 | light red | `#9A6759` |
  | A.Y | 7 | yellow | `#B8C76F` |
  | A.B | 6 | blue | `#352879` |
  | W | 1 | white | `#FFFFFF` |
  | K | 0 | black | `#000000` |
  | B.R | 2 | red | `#68372B` |
  | B.G | 5 | green | `#588D43` |
  | B.B | 6 | blue | `#352879` |
  | B.C | 3 | cyan | `#70A4B2` |
  | B.M | 4 | purple | `#6F3D86` |
  | B.Y | 7 | yellow | `#B8C76F` |

- **Input.** The program reads the keyboard matrix directly as the
  usb-hid-transport-v0 keyboard: `1`–`8` select scenes 0–7, `Q W E R T Y U I`
  select palettes 0–7, and Space or Return is the beat. Until a scene key
  arrives, the procedural demo scene is used; it never selects scene 7. There
  is no energy key, so energy and bands stay at `HID_E_LO`. Spectral mode
  therefore rests at level 1 (black), and held mode keeps its layout. The KERNAL
  CIA1 timer IRQ is stopped so it cannot cost a VBlank.
- **Flash/gate.** Palettes 3 and 7 (`R`, `I`) arm flash/gate only when an
  absolute scene key `8` and the palette key are held together in the same
  scan for P_STABLE (30) steps. Otherwise they run as hard cuts.
- **Not implemented.** The operator role (`L` pin and the RETURN warning screen)
  and joystick relative palette pulses.
- **Limit.** The native PETSCII demo redraws for about 20 frames per loop, so the
  30-step scene-7 debounce from the native scene takes about 14 seconds. In
  scene 7 every frame is a step.

Tests: `tests/c64_colour_field_tests.c` runs the spec fixtures on the host as
part of `just c-test`. `just c64-colour-field-capture` drives the VICE x64sc
libretro core headlessly. It holds keys, saves PNGs to `build/c64-colour-field/`,
and checks every region's exact colour, the boundary positions and the black
border. It needs a local RetroArch core, so it is not part of `just check`.

## Connection path

Use Raspberry Pi USB HID keyboard/joystick output as the first C64 Ultimate live
transport candidate. The C64-side program consumes reduced scene, palette,
energy, beat, and trigger state for raster/PETSCII effects. If USB HID is not
viable from the exact Ultimate setup, fall back to Ultimate network, user-port
serial, joystick-port adapter, or baked `.prg`/`.d64` playback. See
[`../../shared/specs/usb-hid-transport-v0.md`](../../shared/specs/usb-hid-transport-v0.md).

## Constraints

The C64 target should embrace hard limits: tiny data, fixed palettes, raster tricks, and expressive character graphics.
