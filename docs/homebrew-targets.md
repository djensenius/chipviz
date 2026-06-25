# Homebrew target policy

chipviz targets real homebrew programs for the retro platforms. The end goal is
that you can load these artifacts on Analogue 3D/N64, Analogue Pocket/GBA/SNES,
original Game Boy, Genesis/Mega Drive, NES, Master System cores, and C64/C64
Ultimate and have them run directly on those platforms.

Analogue 3D, Analogue Pocket Dock, C64 Ultimate, flash carts, and emulators are
loading, display, input, or validation paths. They are not replacements for
native homebrew builds.

| Platform | Homebrew artifact | First toolchain | Runtime input path |
| --- | --- | --- | --- |
| N64 / Analogue 3D | `.z64` | current libdragon + RDPQ path | Four controller ports via Pi Pico Joybus. |
| GBA / Analogue Pocket | `.gba` | Butano first unless Tonc proves simpler | Pocket Dock USB HID first; link cable later if needed. |
| C64 / C64 Ultimate | `.prg` / `.d64` | cc65 first; KickAssembler if raster timing requires it | C64 Ultimate USB HID first; user-port/joystick fallback. |
| SNES / Analogue Pocket Dock | `.sfc` / `.smc` | PVSnesLib first | Pocket Dock USB HID first; real SNES controller-port adapter later. |
| Game Boy / Analogue Pocket | `.gb` | RGBDS first | Pocket Dock controller first; link cable bridge later. |
| Genesis / Mega Drive | `.md` | SGDK or 68000/Z80 assembly path | Controller-port/devcart bridge later; baked playback first. |
| NES | `.nes` | cc65/ca65 NROM first | Controller-port/devcart bridge later; baked playback first. |
| Master System / Game Gear PSG | `.sms` | devkitSMS/SDCC or Z80 assembly path | Controller-port/devcart bridge later; baked playback first. |

The portable C simulator is only a host-testable scaffold. A platform
implementation issue is complete once its buildable artifact, deterministic
transport mapping, and host-testable fallback path are checked in. Manual bench
validation on real consoles, flash carts, Analogue devices, and adapters is
tracked separately in issue #31.

## SDK and fallback artifacts

Every target now has a first-class SDK/toolchain path:

- `cores/n64/homebrew`: builds `chipviz-n64.z64` with libdragon via `N64_INST`; `just n64-rom-docker` uses the official libdragon toolchain image and caches a current SDK under `.cache/libdragon`.
- `cores/gba/homebrew`: builds `chipviz-gba.gba` with devkitPro/devkitARM and
  `gbafix`.
- `cores/c64/homebrew`: builds `chipviz-c64.prg` with cc65/cl65.
- `cores/snes/homebrew`: builds `chipviz-snes.sfc` with PVSnesLib via
  `PVSNESLIB_HOME`.
- `cores/gb/homebrew`: builds `chipviz-gb.gb` with RGBDS/rgbfix, which writes the
  required Game Boy header/logo during the build.
- `cores/genesis/homebrew`: currently builds a generated Genesis `.md` VDP/CRAM
  seed until an SGDK or assembly SDK path is selected.
- `cores/nes/homebrew`: builds `chipviz-nes.nes` with cc65/ca65.
- `cores/sms/homebrew`: currently builds a generated Master System `.sms`
  VDP/CRAM seed until a devkitSMS/SDCC or assembly SDK path is selected.

`shared/tools/build_homebrew.py` still generates SDK-free C64/SNES/Genesis/NES/SMS
fallback artifacts for local smoke tests and packaging when full SDKs are not
installed. Those fallbacks are not the completion target for platforms that have
a selected SDK path.

## Completion boundary

- N64 implementation: libdragon ROM, packed Joybus transport mapping, and Pi
  Pico controller-port bridge are in-repo. Real controller-port timing and
  Analogue 3D/N64 bench validation remain in #31.
- GBA/Pocket implementation: devkitARM ROM and USB HID reduced-control mapping
  are in-repo. Pocket Dock validation remains in #31.
- C64 implementation: cc65 `.prg` demo, generated fallback `.prg`, and
  reduced-control transport policy are in-repo. C64 Ultimate validation remains
  in #31.
- SNES implementation: PVSnesLib `.sfc` demo, generated fallback LoROM, and
  Pocket Dock/SNES controller mapping policy are in-repo. Pocket/core/flash-cart
  validation remains in #31.
- Game Boy implementation: RGBDS `.gb` demo and host-testable DMG-style simulator
  are in-repo. Pocket/core/flash-cart validation remains in #31.
- Genesis implementation: YM2612/SN76489 simulator and generated Genesis ROM seed
  are in-repo. Full SGDK/assembly demo and hardware validation remain in #31.
- NES implementation: ca65 `.nes` demo, generated fallback iNES, and host-testable
  2A03-style simulator are in-repo. Hardware validation remains in #31.
- Master System implementation: SN76489 simulator and generated SMS ROM seed are
  in-repo. Full devkitSMS/SDCC demo and hardware validation remain in #31.
