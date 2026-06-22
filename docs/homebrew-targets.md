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

The portable C simulator is only a host-testable scaffold. A platform issue is
not fully done until its homebrew artifact exists and the intended loading path
has been validated.

## Current generated artifacts

`shared/tools/build_homebrew.py` generates the minimal homebrew artifacts that do
not require proprietary boot blobs or unavailable SDKs:

- `chipviz-c64.prg`: a tokenized BASIC/VIC-II color-cycling C64 program intended
  to load directly on C64/C64 Ultimate.
- `chipviz-snes.sfc`: a minimal LoROM-style SNES image that initializes the
  display to a visible backdrop and is intended for emulator, flash cart, or
  Pocket SNES-core validation.

GBA and N64 now have SDK-backed project directories:

- `cores/gba/homebrew`: builds `chipviz-gba.gba` with devkitPro/devkitARM and
  `gbafix`.
- `cores/n64/homebrew`: builds `chipviz-n64.z64` with libdragon via `N64_INST`.

Those SDKs handle the platform-specific boot/header details. The release
packager includes `.gba` and `.z64` when the SDKs are present, and otherwise
ships the C64/SNES/minimal assets plus a status file.
