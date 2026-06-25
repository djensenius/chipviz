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

## Connection path

Use Raspberry Pi USB HID keyboard/joystick output as the first C64 Ultimate live
transport candidate. The C64-side program consumes reduced scene, palette,
energy, beat, and trigger state for raster/PETSCII effects. If USB HID is not
viable from the exact Ultimate setup, fall back to Ultimate network, user-port
serial, joystick-port adapter, or baked `.prg`/`.d64` playback. See
[`../../shared/specs/usb-hid-transport-v0.md`](../../shared/specs/usb-hid-transport-v0.md).

## Constraints

The C64 target should embrace hard limits: tiny data, fixed palettes, raster tricks, and expressive character graphics.
