# chipviz-gb

Original Game Boy / DMG visualizer for the Game Boy APU side of ChipStation.

## Homebrew stack

- RGBDS for the first real `.gb` artifact. The repo does not hand-embed the
  Nintendo logo bytes; `rgbfix` writes a valid header during the SDK build.
- DMG tile maps, four-shade palette cycling, scrolling backgrounds, and small
  sprite/channel meters.

## First demo

- Self-running DMG tile-field visualizer with beat-synced scrolling.
- D-pad/buttons can later select scenes and palettes.
- Later: Pocket Dock USB HID or link-cable adapter for reduced live controls.

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
DMG-style scroll/window/sprite-lane parameters for host simulation.
`cores/gb/homebrew` contains the RGBDS project for `chipviz-gb.gb`.

## Connection path

Start with baked/procedural playback and Pocket Dock controller input. A link
cable bridge remains a later richer live-input path.

## Constraints

Use the hardware the way the DMG wants: tiny tile maps, four shades, cheap
scrolling, and sparse sprite/meters rather than framebuffer effects.
