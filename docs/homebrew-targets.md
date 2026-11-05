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

- `cores/n64/homebrew`: builds `chipviz-n64.z64` with libdragon via `N64_INST`;
  `just n64-rom-docker` builds a local compiler image from the official prebuilt
  toolchain and caches a pinned SDK under `.cache/libdragon`.
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

## Pinned SDK builds

`mise exec -- just n64-rom-docker` uses Docker's `linux/amd64` platform on both
Linux and Apple Silicon. It builds `chipviz-libdragon-toolchain:local` using
[`scripts/n64-toolchain.Dockerfile`](../scripts/n64-toolchain.Dockerfile): a
digest-pinned Ubuntu base and DragonMinded's prebuilt compiler with a pinned
SHA-256. It does not compile GCC or require the upstream GHCR image.

The default libdragon source revision is
`e356bf3f56f7afbf7e5246329562f145965cfdfc`. The official compiler download comes
from a rolling release: if its bytes change, the Docker build fails the checksum
check rather than accepting a new compiler. This protects the pin, but does not
guarantee that upstream will retain the old download. Update the source/compiler
pins deliberately together, then perform a clean ROM build and emulator check.

Existing overrides remain available: `LIBDRAGON_DOCKER_IMAGE` skips the default
image build, `LIBDRAGON_BRANCH` selects a source branch/tag/SHA, and
`LIBDRAGON_DOCKER_CACHE` selects the cache directory. A custom image must expose
the toolchain at `/n64_toolchain`. Modified tracked SDK sources are rejected.
Each source revision/compiler image ID gets a separate source-only checkout and
installed SDK under `builds/` in the cache. Build outputs are not copied between
them. The N64 ROM records that identity separately, so switching to another
populated SDK cache also cleans the ROM's object files before compiling.

Both automated SNES build paths pin PVSnesLib to
`d0bd46bc70c16a4cb9c8d40a90468e18a6de9b1c`, including its recorded submodules.
For local builds, check out that revision, run
`git submodule update --init --recursive`, and set `PVSNESLIB_HOME` to its path.
Build and install its `compiler` and `tools`, then build `pvsneslib`, before
running `mise exec -- just snes-rom`. This compatible pin retains the current
SNES Makefile; it does not repair the separate rendering/timing defects.

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
