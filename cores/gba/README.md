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

The current host HID encoder emits absolute scene and palette as keyboard
keys; it does **not** synthesize the gamepad D-pad pulses. Consequently it
does not currently stream scene or palette selections to this GBA receiver.
No keyboard or richer control-frame input is implemented by this ROM.

For native overlays, sampled A/B/Select/Start states, including releases, are
presented in order rather than combined across polls. The 16-transition queue
drops newer transitions if input outruns presentation; after it drains, the
latest held state is used. Scene and palette D-pad edges remain separate.
On field exit, the last field page stays visible while the next native page is
prepared in chunks and pad input continues to be polled.

## Constraints

GBA is ideal for fast 2D motion and palette work. Keep scenes stylized and data-light so they can be shared with Pocket and cartridge/flashcart workflows.
