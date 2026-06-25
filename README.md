# chipviz

Original homebrew visualization software for real retro hardware and FPGA consoles.

## Initial targets

| Target | Output | Hardware path | Role |
| --- | --- | --- | --- |
| N64 / Analogue 3D | homebrew `.z64` | Flashcart on Analogue 3D or N64 | Flagship big-screen 3D/particle visualizer. |
| GBA / Analogue Pocket | homebrew `.gba` | GBA cartridge, flashcart, or Pocket adapter path | Portable 2D sketchpad with palettes, sprites, and affine effects. |
| Commodore 64 / THEC64 | homebrew `.prg` / `.d64` | THEC64 USB media or real C64 storage | VIC-II/SID/raster/PETSCII visual personality. |
| SNES / Analogue Pocket Dock | homebrew `.sfc` / `.smc` | Pocket Dock SNES core first, real SNES later | Tile/sprite/Mode-7-style visualizer. |
| Modern 4K | native app | Raspberry Pi 5 HDMI or Apple Silicon Mac | Dense 3D data-field visualizer rendered with Rust + wgpu. |

## Project layout

```text
cores/
  n64/      N64 homebrew target, likely libdragon + tiny3d.
  gba/      GBA homebrew target, likely Butano or Tonc.
  c64/      C64 target, likely cc65 or KickAssembler.
  snes/     SNES target, PVSnesLib for the production SDK path.
shared/
  include/  Tiny C interface for control frames and platform connection polling.
  specs/    Cross-platform control protocol and scene model.
  src/      Portable frame packing, validation, and procedural fallback source.
  tools/    Asset conversion, palettes, MIDI/audio preprocessing.
host/
  bridge/   Desktop bridge for frame generation, playback, and hardware transport.
  fixtures/ Golden musical/chipsynth sample inputs and expected frame outputs.
renderers/
  modern/   Rust + wgpu renderers for Raspberry Pi 5 and Apple Silicon.
assets/
  palettes/ Shared palette experiments.
  scenes/   Shared scene sketches and data-driven visual definitions.
```

## Shared control model

The first common interface is a compact frame that every platform can interpret in its own way:

- transport: tempo, beat phase, beat counter
- energy: overall level plus bass/mid/treble
- spectrum: eight compact bands
- scene: scene index, palette index, mode flags
- notes: small event window for MIDI note-style triggers

See [`shared/specs/control-frame-v0.md`](shared/specs/control-frame-v0.md),
[`shared/specs/source-adapter-v0.md`](shared/specs/source-adapter-v0.md),
[`shared/specs/music-source-v0.md`](shared/specs/music-source-v0.md), and
[`shared/specs/chipsynth-source-v0.md`](shared/specs/chipsynth-source-v0.md).
See [`shared/specs/interface-v0.md`](shared/specs/interface-v0.md) for the
cross-target title/scene-select/auto/demo interface model.
See [`docs/connections.md`](docs/connections.md) for the planned hardware,
ESP32 bridge, and Raspberry Pi sender/audio-analysis paths.
See [`docs/homebrew-targets.md`](docs/homebrew-targets.md) for the target policy:
N64, GBA, C64, and SNES are real homebrew builds.

## First milestone

Build one self-running demo per platform before live input:

1. `chipviz-n64`: 3D planes, camera motion, palette cycling, particles, and controller-driven scene changes.
2. `chipviz-gba`: sprite/tile/affine visualizer with beat simulation.
3. `chipviz-c64`: raster bars, PETSCII patterning, SID/noise-reactive fake input loop.
4. `chipviz-snes`: tile/sprite/Mode-7-style visualizer with Pocket Dock control input.
5. `chipviz-pi5` / `chipviz-m1`: 4K data fields, layered planes, and camera motion using the same scene controls at different quality budgets.

## Releases

Merges to `main` run release-please. Published releases build individual assets:
modern renderer binaries, `.cvz` demos, N64 Joybus and USB HID mapped streams,
generated C arrays, and current homebrew artifacts. The release asset workflow
builds `chipviz-gba.gba` with devkitPro, `chipviz-n64.z64` with libdragon,
`chipviz-c64.prg` with cc65, and `chipviz-snes.sfc` with PVSnesLib. Generated
C64/SNES assets remain SDK-free fallbacks for local packaging when those
toolchains are not installed.

## Current scaffold

The first checked-in code path is intentionally portable C and Python so it can
validate without platform SDKs installed:

```sh
mise run check
just lint
just test
just simulate
just modern-m1 -- --udp 127.0.0.1:6464
python3 host/bridge/chipviz_bridge.py --frames 120 --output build/demo.cvz
python3 host/bridge/chipviz_encode.py --verify-fixtures
```

Each platform entrypoint consumes the shared `control-frame-v0` shape through a
small connection abstraction. Until a live receiver is supplied, the connection
generates deterministic procedural frames so every target has an immediate
visualization loop.
