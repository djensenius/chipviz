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
target role in the `data` input role. PAL (and Drean) and NTSC machines are
supported: at boot the program reads the KERNAL's PAL/NTSC flag (`get_tv()`,
`$02A6`) and puts a raster IRQ in the vertical blank, at line 300 on PAL
(below the visible border) or line 16 on NTSC (above it). The IRQ runs one step
per VBlank poll: it writes the previous step's VIC-II registers (border first),
samples the keyboard, and stages the next step. All changes land in the
vertical blank, so the screen never tears. The on-time check is an inline
8-bit table compare, so the register writes finish 4 raster lines after the
read (measured in VICE). The latest accepted reads are PAL line 3 and NTSC
line 31, so the writes finish by line 7 or 35, before the first visible
border line (PAL 16, NTSC 41). If a step ever overruns a frame,
the late IRQ drops that step entirely (no register writes, input sampling or
tick) rather than change registers mid-screen or catch up; keys still held
are sampled on the next step. The early NTSC VIC-II (6567R56A: 262 lines of 64
cycles, about 60.99 frames a second) would otherwise make 61 steps in some
seconds. Even one drop in 61 is not enough there: 62 frames last only 121
cycles more than a second, so an IRQ accepted 8 or more lines late could
squeeze 61 steps into one. The program counts the raster lines at boot and,
on 262 lines, drops one IRQ's step in every 31 the same way (59.02 steps a
second), which holds for any lateness the IRQ accepts. The 6567R8 (263
lines, 59.83 frames a second), PAL (50.12) and Drean never drop a step, so
there are never more than 60 steps in any closed 1-second window (spec: Step
rate).
The native PETSCII demo keeps drawing in the main loop and pauses while scene 7
is shown. Its frame counter keeps running during scene 7, so after a visit its
animation phase jumps ahead instead of resuming where it stopped.

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
  select palettes 0–7, Space is the beat and Return the bar. Until a scene key
  arrives, the procedural demo scene is used; it never selects scene 7. There
  is no energy key, so energy and bands stay at `HID_E_LO`. Spectral mode
  therefore rests at level 1 (black), and held mode keeps its layout. The KERNAL
  CIA1 timer IRQ is stopped; the raster IRQ is the only interrupt, and it
  returns through `$EA81` without the KERNAL's clock, cursor and key-buffer
  work. The key mapping (`cf_read_keys`) is portable and host-tested.
- **Joysticks alias the keyboard.** The control ports share the CIA1 matrix
  lines, so they read as keys. Port 1 drives the row lines in every column:
  fire reads as Space (a beat), down as Return (a bar) plus `W R Y I`, up as
  `1 3 5 7` and right as `2 4 6 8`. Several keys in one group select nothing,
  so up/right/down never change the scene or palette. Port 2 drives the column
  lines: on its own it reads as nothing, but while keys are held it copies them
  into other columns, which usually makes their group ambiguous (ignored).
- **Flash/gate is off by default.** Shipped builds must not flash until the
  effect has had independent photosensitivity measurement (PEAT or Harding,
  [#96](https://github.com/djensenius/chipviz/issues/96)). The default ROM
  (`just c64-rom`, the release artifact) is built without `CF_UNMEASURED_FLASH`:
  `cf_tick` drops every arming input, so palettes 3 and 7 (`R`, `I`) always run
  as unarmed 2 and 6 (hard cuts), even with `8` held, and the flash latch is
  never set. For measurement only, `make -C cores/c64/homebrew
  UNMEASURED_FLASH=1` builds `chipviz-c64-unmeasured-flash.prg` (objects and
  map in `build/unmeasured-flash/`) next to the untouched release
  `chipviz-c64.prg`; load that file by name in VICE or RetroArch, and never ship
  it. In it, 3 and 7 arm flash/gate only when an absolute scene key `8` and the
  palette key are held together in the same scan for P_STABLE (30) steps.
  Each palette value becomes a request once: a value already consumed (for
  example `R` held alone for 30 steps) stays consumed when the incoming scene
  briefly leaves 7. The palette is read on every step, so a different value,
  even while the scene is briefly not 7, or a new visit to scene 7 starts a
  new run, which needs 30 stable steps in scene 7 again.
- **Not implemented.** The operator role (`L` pin and the RETURN warning screen)
  and joystick relative palette pulses.
- **Timing.** Every frame is a step, in or out of scene 7 (on the 6567R56A,
  30 frames in every 31). Holding `8` shows
  the field from the 31st sampled step (the debounce decides at the end of the
  30th). Measured in VICE (PAL and NTSC alike): 31–32 frames from the key press until the step that
  shows scene 7, and 33–34 frames until it is on screen (registers land at the
  next VBlank). Exit with `1` takes 31–33 frames, and 32–34 until the last
  native frame is back on screen. The per-frame step costs the native demo
  some time: it draws one loop in about 26 frames instead of about 22.

Tests: `tests/c64_colour_field_tests.c` runs the spec fixtures on the host as
part of `just c-test`, built twice: by default every flash fixture checks that
3 and 7 run exactly as 2 and 6 and never latch, and with `-DCF_UNMEASURED_FLASH`
the spec's flash results are checked. `just c64-colour-field-capture` drives the VICE x64sc
libretro core headlessly as a PAL, an NTSC (6567R8) and an early NTSC
(6567R56A, `C64 OLD NTSC`) C64. It holds keys,
saves PNGs to `build/c64-colour-field/` (NTSC files are `c64-ntsc-*` and
`c64-ntsc-old-*`), and
checks every region's exact colour, the boundary positions and the black
border. It also measures the entry and exit timing from the core's RAM, and
checks that `R` held without `8`, and `8`+`R` in the default ROM, stay in
steady hard cuts with the flash latch clear, that 610 frames of scene 7 make
610 steps (590 or 591 on the 6567R56A), and that the native
demo animates. It needs a local RetroArch core, so it is not part of
`just check`.

## SpectrumVolume prototype (not selectable)

`homebrew/src/spectrum_volume.c` is an isolated scene core, not linked into
the program or legacy scene dispatch. It adapts the modern scene's repeated
eight-band volume as three staggered slices of reverse-space blocks in the
uppercase/graphics ROM. Each band has a height of 0–5 cells, rising or
falling by at most one cell per step; low energy reduces the height. The
eight fixed VIC-II inks exclude black and white. Palette changes advance one
band per step; at most 32 screen cells and 52 colour cells are written per
step. This bounds individual updates, but is not a photosensitivity
measurement or a guarantee about transitions.

The caller must reset the state and supply an exclusively owned, blank 40x25
screen on a black background, with the uppercase/graphics ROM selected; it
must arrange safe entry and restoration itself. The core writes no VIC
registers, does not clear the screen, and must run outside the raster IRQ.
The caller must avoid the reserved ColourField screens at `$3000`–`$3FFF`.
Eventual integration must preserve the existing IRQ timing and
keyboard/joystick roles, and account for the current HID keyboard path's zero
spectrum (which produces silence, not fabricated audio). Selection awaits the
extended catalog contract in #140 and #152. Production cc65 linking/memory
headroom, PAL/NTSC timing, native/ColourField transitions, visual captures and
independent flash-safety measurement remain unverified.

The isolated host test can be run without building the C64 program:

```sh
mkdir -p build/tests
cc -std=c99 -Wall -Wextra -Werror -pedantic -Icores/c64/homebrew/src tests/c64_spectrum_volume_tests.c cores/c64/homebrew/src/spectrum_volume.c -o build/tests/c64_spectrum_volume_tests
build/tests/c64_spectrum_volume_tests
```

`tests/c64_spectrum_volume_fixture.c` is a separate test-only PRG, linked
against the isolated scene. It uses guarded ordinary RAM buffers, not VIC
screen or colour RAM. Its 112-record bounded trace covers each band rising
and falling, band and energy thresholds, palette changes, silence and reset.
Each record contains state, a 16-bit sum and an order-sensitive CRC-16 per
complete buffer, and a guard result. These are not exact comparisons:
collisions remain possible. `scripts/c64_spectrum_volume_fixture.py` compares
them against an independently rendered synthetic oracle through the existing
VICE libretro RAM seam. Versioned completion and progress markers reject the
old trace format. The host-only oracle tests run with Python unittest
discovery. Build the fixture and, when an emulator slot is available, run it
separately:

```sh
mkdir -p build/tests/c64-spectrum-fixture
cl65 -t c64 -Oirs --static-locals -Icores/c64/homebrew/src -c -o build/tests/c64-spectrum-fixture/fixture.o tests/c64_spectrum_volume_fixture.c
cl65 -t c64 -Oirs --static-locals -c -o build/tests/c64-spectrum-fixture/spectrum_volume.o cores/c64/homebrew/src/spectrum_volume.c
cl65 -t c64 -m build/tests/c64-spectrum-fixture/fixture.map -Ln build/tests/c64-spectrum-fixture/fixture.lbl -o build/tests/c64-spectrum-fixture/fixture.prg build/tests/c64-spectrum-fixture/fixture.o build/tests/c64-spectrum-fixture/spectrum_volume.o
python3 scripts/c64_spectrum_volume_fixture.py --model pal
python3 scripts/c64_spectrum_volume_fixture.py --model ntsc
python3 scripts/c64_spectrum_volume_fixture.py --model ntsc-old
```

The script needs a local VICE x64sc libretro core (override with `--core`).
Always rebuild matching PRG and labels together. On prototype head `9727671`,
the rebuilt fixture matched all 112 records after 5304 PAL frames, 6156 NTSC
frames and 6284 early NTSC frames using the earlier checksums, not the current
CRC fixture. A prior NTSC run did not finish within the old 6000-frame
timeout. PAL's 5304 frames represent about 104.3 million
nominal VIC cycles, versus 102.6 million in 6000 NTSC frames (100.6 million
on early NTSC). Boot, IRQs and VIC bus contention vary, so this supports an
insufficient nominal cycle budget as a diagnosis, but does not prove the
timeout's cause. The current CRC fixture has not been run on the target, and
its cost and completion time are unverified. The default is 9000 frames,
still bounded and overridable with `--frames` (up to 12000); it may require
recalibration. On timeout the script reports recent
completed-record counts and checks the completed trace prefix, distinguishing
some trace errors from lack of completion; a matching prefix does not prove
the target cannot hang later. These prototype results are not exact-head
verification. The fixture does not exercise the production PRG, actual
screen/colour RAM, raster IRQ, hardware timing, scene transitions, visible
output or physical flash safety.

## Connection path

Use Raspberry Pi USB HID keyboard/joystick output as the first C64 Ultimate live
transport candidate. The C64-side program consumes reduced scene, palette,
energy, beat, and trigger state for raster/PETSCII effects. If USB HID is not
viable from the exact Ultimate setup, fall back to Ultimate network, user-port
serial, joystick-port adapter, or baked `.prg`/`.d64` playback. See
[`../../shared/specs/usb-hid-transport-v0.md`](../../shared/specs/usb-hid-transport-v0.md).

## Constraints

The C64 target should embrace hard limits: tiny data, fixed palettes, raster tricks, and expressive character graphics.
