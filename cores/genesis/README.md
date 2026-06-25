# chipviz-genesis

Sega Genesis / Mega Drive visualizer for the YM2612 + SN76489 ChipStation side.

## Homebrew stack

- First SDK target: SGDK or a small 68000/Z80 assembly project once a toolchain
  is selected.
- Current checked-in fallback is a generated, headered Genesis ROM seed so
  packaging and emulator smoke paths have a deterministic artifact.

## First demo

- FM-channel meter planes inspired by the six YM2612 voices.
- PSG flash accents for the Genesis SN76489 side.
- VDP plane/CRAM palette motion rather than expensive framebuffer work.

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
FM voice masks, VDP plane scroll, and PSG flash values for host simulation.
`cores/genesis/homebrew` generates `chipviz-genesis.md` through the shared
SDK-free homebrew artifact builder until a Genesis SDK is chosen.

## Connection path

Start with baked/procedural playback and reduced controller input. A later
hardware bridge can map ChipStation voice telemetry onto Genesis controller-port
or serial/devcart input.

## Constraints

Lean into VDP planes, palettes, sprites, and the distinct YM2612/SN76489 voice
layout. Avoid assuming a linear framebuffer.
