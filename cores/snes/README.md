# chipviz-snes

SNES visualizer target for Analogue Pocket Dock SNES-core workflows first, with
real SNES controller-port hardware later.

## Stack choice

Use **libSFX** for the first production SNES homebrew path. It is C-friendly,
modern enough for repeatable builds, and a better fit for this repo's portable C
scaffold than starting with assembly-only examples. The first hardware milestone
is a `.sfc`/`.smc` ROM once the SNES SDK is installed; until then, the native
simulator builds the same control-frame mapping.

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
a SNES-style model: beat phase drives a Mode-7-style angle, energy drives sprite
count, palette selects CGRAM banks, and flags trigger HDMA/raster-like pulses.
The simulator prints those values until the libSFX renderer replaces the stub.

## Pocket Dock live path

The first live control path is Raspberry Pi USB HID through the Analogue Pocket
Dock. The SNES core treats the Pi as a controller and decodes the same reduced
button-state mapping documented in
[`../../shared/specs/usb-hid-transport-v0.md`](../../shared/specs/usb-hid-transport-v0.md).

## Real SNES follow-up

Real SNES hardware should use a controller-port adapter. Required cable: a SNES
controller extension cable, preferably one per emulated controller port. The
adapter can be Pico/ESP32-based and should emulate the serial controller shift
protocol rather than attempting raw audio or high-bandwidth data.
