# Homebrew target policy

chipviz targets real homebrew programs for the retro platforms. The end goal is
that you can load these artifacts on Analogue 3D/N64, Analogue Pocket/GBA/SNES
cores, and C64/C64 Ultimate and have them run directly on those platforms.

Analogue 3D, Analogue Pocket Dock, C64 Ultimate, flash carts, and emulators are
loading, display, input, or validation paths. They are not replacements for
native homebrew builds.

| Platform | Homebrew artifact | First toolchain | Runtime input path |
| --- | --- | --- | --- |
| N64 / Analogue 3D | `.z64` | libdragon + OpenGL 1.1 / RSP path | Four controller ports via Pi Pico Joybus. |
| GBA / Analogue Pocket | `.gba` | Butano first unless Tonc proves simpler | Pocket Dock USB HID first; link cable later if needed. |
| C64 / C64 Ultimate | `.prg` / `.d64` | cc65 first; KickAssembler if raster timing requires it | C64 Ultimate USB HID first; user-port/joystick fallback. |
| SNES / Analogue Pocket Dock | `.sfc` / `.smc` | libSFX first | Pocket Dock USB HID first; real SNES controller-port adapter later. |

The portable C simulator is only a host-testable scaffold. A platform
implementation issue is complete once its buildable artifact, deterministic
transport mapping, and host-testable fallback path are checked in. Manual bench
validation on real consoles, flash carts, Analogue devices, and adapters is
tracked separately in issue #31.

## Current generated artifacts

`shared/tools/build_homebrew.py` generates the minimal homebrew artifacts that do
not require external SDKs:

- `chipviz-c64.prg`: a tokenized BASIC/VIC-II and PETSCII color-cycling C64
  visualizer intended to load directly on C64/C64 Ultimate.
- `chipviz-snes.sfc`: a visible LoROM-style SNES image that initializes CGRAM
  and VRAM markers for emulator, flash cart, or Pocket SNES-core validation.

GBA and N64 have SDK-backed project directories:

- `cores/gba/homebrew`: builds `chipviz-gba.gba` with devkitPro/devkitARM and
  `gbafix`.
- `cores/n64/homebrew`: builds `chipviz-n64.z64` with libdragon via `N64_INST`.

Those SDKs handle the platform-specific boot/header details. The release
asset workflow installs/uses those SDKs so published releases include
`chipviz-gba.gba` and `chipviz-n64.z64` in addition to the C64/SNES artifacts.

## Completion boundary

- N64 implementation: libdragon ROM, packed Joybus transport mapping, and Pi
  Pico controller-port bridge are in-repo. Real controller-port timing and
  Analogue 3D/N64 bench validation remain in #31.
- GBA/Pocket implementation: devkitARM ROM and USB HID reduced-control mapping
  are in-repo. Pocket Dock validation remains in #31.
- C64 implementation: C64-loadable `.prg` visualizer and reduced-control
  transport policy are in-repo. C64 Ultimate validation remains in #31.
- SNES implementation: LoROM `.sfc` artifact and Pocket Dock/SNES controller
  mapping policy are in-repo. Pocket/core/flash-cart validation remains in #31.
