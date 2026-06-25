# chipviz-snes

SNES homebrew visualizer target for Analogue Pocket Dock SNES-core workflows first, with
real SNES controller-port hardware later.

## Stack choice

Use **PVSnesLib** for the first production SNES homebrew path. It is a known
C-based SNES homebrew SDK and a better completion target than a hand-generated
LoROM placeholder. The first hardware milestone is a `.sfc`/`.smc` ROM built
from the PVSnesLib project.

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
a SNES-style model: beat phase drives a Mode-7-style angle, energy drives sprite
count, palette selects CGRAM banks, and flags trigger HDMA/raster-like pulses.
The simulator prints those values for host tests.
The simulator is not the product; the target artifact is a bootable homebrew
`.sfc`/`.smc`.
`cores/snes/homebrew` contains the PVSnesLib project for `chipviz-snes.sfc`. It
currently runs a self-contained CGRAM/channel-meter color demo, and the
generated LoROM from `just homebrew-artifacts` remains only an SDK-free fallback
for local smoke tests.

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
