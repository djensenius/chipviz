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

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
a minimal GBA-style visualization model: beat phase scrolls tiles, mid energy
sets sprite count, and total energy drives affine scale. The native simulator
build prints those values until the Butano or Tonc renderer replaces the stub.

## Connection path

Use the Analogue Pocket Dock's Bluetooth controller support as the first live
control path. A paired controller, or an ESP32 presenting as a Bluetooth HID
gamepad, maps `control-frame-v0` state down to scene, palette, intensity, and
trigger buttons. See
[`../../docs/connections.md`](../../docs/connections.md#analogue-pocket-dock--gba-bluetooth-controller-bridge).

## Constraints

GBA is ideal for fast 2D motion and palette work. Keep scenes stylized and data-light so they can be shared with Pocket and cartridge/flashcart workflows.
