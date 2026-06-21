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

## Constraints

The C64 target should embrace hard limits: tiny data, fixed palettes, raster tricks, and expressive character graphics.
