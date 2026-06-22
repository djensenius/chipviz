# chipviz-c64

Commodore 64 visualizer core for Commodore 64 Ultimate and real C64-style workflows.

## Planned stack

- cc65 for portable C/assembly builds, or KickAssembler if raster timing becomes the priority.
- VIC-II raster bars, PETSCII patterns, character animation, sprites, and SID-driven timing experiments.

## First demo

- Raster bar and PETSCII scene.
- Keyboard/joystick scene switching.
- Later: generated playback tables from the shared control protocol.
- Later: live bridge input through Commodore 64 Ultimate networking features or a user-port serial adapter.

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
a minimal C64-style visualization model: beat phase selects a raster position,
scene and palette choose color, and energy chooses a PETSCII cell. The native
simulator build prints those values until the cc65 VIC-II renderer replaces the
stub.

## Connection path

Use Raspberry Pi USB HID keyboard/joystick output as the first C64 Ultimate live
transport candidate. The C64-side program consumes reduced scene, palette,
energy, beat, and trigger state for raster/PETSCII effects. If USB HID is not
viable from the exact Ultimate setup, fall back to Ultimate network, user-port
serial, joystick-port adapter, or baked `.prg`/`.d64` playback. See
[`../../shared/specs/usb-hid-transport-v0.md`](../../shared/specs/usb-hid-transport-v0.md).

## Constraints

The C64 target should embrace hard limits: tiny data, fixed palettes, raster tricks, and expressive character graphics.
