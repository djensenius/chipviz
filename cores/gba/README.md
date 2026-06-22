# chipviz-gba

GBA homebrew visualizer for Analogue Pocket and GBA-compatible hardware.

## Homebrew stack

- Butano for the first batteries-included C++ GBA homebrew path, or Tonc for lower-level C examples.
- Tile, sprite, palette, affine, and bitmap-mode effects.

## First demo

- Self-running palette and affine-effect visualizer.
- D-pad and buttons select scene/palette/intensity.
- Dock controller input can become a low-bandwidth live control surface.
- Later: playback files generated from the shared control protocol.
- Later: an ESP32-to-GBA link cable bridge can provide richer live frame input.

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
a minimal GBA-style visualization model: beat phase scrolls tiles, mid energy
sets sprite count, and total energy drives affine scale. The native simulator
build prints those values until the Butano or Tonc renderer replaces the stub.
The simulator is not the product; the target artifact is a bootable homebrew
`.gba`.
`cores/gba/homebrew` contains the devkitARM project for `chipviz-gba.gba`.

## Connection path

Use Raspberry Pi USB HID through the Analogue Pocket Dock as the first live
control path. The Pi maps `control-frame-v0` state down to scene, palette,
intensity, and trigger buttons. See
[`../../shared/specs/usb-hid-transport-v0.md`](../../shared/specs/usb-hid-transport-v0.md).

## Constraints

GBA is ideal for fast 2D motion and palette work. Keep scenes stylized and data-light so they can be shared with Pocket and cartridge/flashcart workflows.
