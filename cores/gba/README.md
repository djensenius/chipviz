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

The homebrew scene ring is 0, 1, 2, 3, 7: D-pad up/down selects the next or
previous scene and left/right selects the palette. Scene 7 is the colour field.
The default data role debounces scene 7 and palette changes; palettes 3 and 7
run as hard cuts, never flash. South and East produce beat/bar edges and the
left shoulder selects the high energy bucket. The digital-only path has no
separate bands, notes, drop or silence input.

The default host HID encoder emits absolute scene and palette as keyboard keys,
which this ROM does not read. Its opt-in `--gba-gamepad` conversion emits
relative D-pad JSON states for this scene ring, subject to the known-ring and
release-preamble requirements in the
[HID transport spec](../../shared/specs/usb-hid-transport-v0.md#gba-gamepad-only-json-conversion).
It is not a USB sender, and actual Dock/hardware operation is not yet verified.
No keyboard or richer control-frame input is implemented by this ROM.

Native overlays use the A/B/Select/Start state sampled at the beginning of each
page; short presses between snapshots can be missed. Scene and palette D-pad
edges are polled separately. While a field exit debounces, the candidate
native page is prepared in the hidden buffer using a page-start input and
palette snapshot. A changed candidate discards that preparation.

## Constraints

GBA is ideal for fast 2D motion and palette work. Keep scenes stylized and data-light so they can be shared with Pocket and cartridge/flashcart workflows.
