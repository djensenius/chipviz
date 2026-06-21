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

Use the Commodore 64 Ultimate network path as the first live transport candidate:
the host bridge or ESP32 sends the latest validated `control-frame-v0` over the
LAN, and the C64-side program consumes a reduced frame for raster/PETSCII
effects. If the Ultimate network service is not viable from a running C64
program, fall back to user-port serial or baked `.prg`/`.d64` playback. See
[`../../docs/connections.md`](../../docs/connections.md#commodore-64-ultimate-network-bridge).

## Constraints

The C64 target should embrace hard limits: tiny data, fixed palettes, raster tricks, and expressive character graphics.
