# chipviz-nes

NES visualizer for the 2A03 ChipStation side.

## Homebrew stack

- cc65/ca65 for the first real iNES/NROM artifact.
- PPU nametable, attribute, CHR tile, palette, and sprite/OAM-style effects.

## First demo

- Self-running NROM demo with pulse/triangle/noise-inspired bars and scrolling
  tile fields.
- Controller input can later select scenes and palettes.

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
