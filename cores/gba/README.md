# chipviz-gba

GBA visualizer core for Analogue Pocket and GBA-compatible hardware.

## Planned stack

- Butano for a batteries-included C++ GBA path, or Tonc for lower-level C examples.
- Tile, sprite, palette, affine, and bitmap-mode effects.

## First demo

- Self-running palette and affine-effect visualizer.
- D-pad and buttons select scene/palette/intensity.
- Dock controller input can become a low-bandwidth live control surface.
- Later: playback files generated from the shared control protocol.
- Later: an ESP32-to-GBA link cable bridge can provide richer live frame input.

## Constraints

GBA is ideal for fast 2D motion and palette work. Keep scenes stylized and data-light so they can be shared with Pocket and cartridge/flashcart workflows.
