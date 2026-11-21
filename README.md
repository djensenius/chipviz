# chipviz

Original homebrew visualization software for real retro hardware and FPGA consoles.

## Initial targets

| Target | Output | Hardware path | Role |
| --- | --- | --- | --- |
| N64 / Analogue 3D | homebrew `.z64` | Flashcart on Analogue 3D or N64 | Flagship big-screen 3D/particle visualizer. |
| GBA / Analogue Pocket | homebrew `.gba` | GBA cartridge, flashcart, or Pocket adapter path | Portable 2D sketchpad with palettes, sprites, and affine effects. |
| Commodore 64 / THEC64 | homebrew `.prg` / `.d64` | THEC64 USB media or real C64 storage | VIC-II/SID/raster/PETSCII visual personality. |
| SNES / Analogue Pocket Dock | homebrew `.sfc` / `.smc` | Pocket Dock SNES core first, real SNES later | Tile/sprite/Mode-7-style visualizer. |
| Game Boy / Analogue Pocket | homebrew `.gb` | Pocket GB core, flashcart, or real DMG/GBC | Four-shade tile/sprite visualizer for Game Boy APU telemetry. |
| Sega Genesis / Mega Drive | homebrew `.md` | Genesis Plus GX, flashcart, or real Genesis | YM2612 FM voice planes with PSG accents. |
| NES | homebrew `.nes` | NES emulator/core, flashcart, or real NES | 2A03 pulse/triangle/noise tile visualizer. |
| Sega Master System / Game Gear | homebrew `.sms` | SMS emulator/core, flashcart, or real SMS/GG | SN76489 PSG tile/channel visualizer. |
| Modern 4K | native app | Raspberry Pi 5 HDMI or Apple Silicon Mac | Dense 3D data-field visualizer rendered with Rust + wgpu. |

## Project layout

```text
cores/
  n64/      N64 homebrew target, likely libdragon + tiny3d.
  gba/      GBA homebrew target, likely Butano or Tonc.
  c64/      C64 target, likely cc65 or KickAssembler.
  snes/     SNES target, PVSnesLib for the production SDK path.
  gb/       Original Game Boy target, RGBDS for the production SDK path.
  genesis/  Sega Genesis/Mega Drive target for YM2612 + SN76489 visuals.
  nes/      NES target, ca65/cc65 for the first NROM SDK path.
  sms/      Sega Master System target for SN76489 visuals.
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
See [`shared/specs/colour-field-v0.md`](shared/specs/colour-field-v0.md) for the
projection colour-field scene (scene 7) shared by the modern and console
renderers.
See [`docs/connections.md`](docs/connections.md) for the planned hardware,
ESP32 bridge, and Raspberry Pi sender/audio-analysis paths.
See [`docs/homebrew-targets.md`](docs/homebrew-targets.md) for the target policy:
N64, GBA, C64, SNES, Game Boy, Genesis, NES, and Master System are tracked as
homebrew builds.

## First milestone

Build one self-running demo per platform before live input:

1. `chipviz-n64`: 3D planes, camera motion, palette cycling, particles, and controller-driven scene changes.
2. `chipviz-gba`: sprite/tile/affine visualizer with beat simulation.
3. `chipviz-c64`: raster bars, PETSCII patterning, SID/noise-reactive fake input loop.
4. `chipviz-snes`: tile/sprite/Mode-7-style visualizer with Pocket Dock control input.
5. `chipviz-gb`: DMG four-shade scroll/tile visualizer for Game Boy APU telemetry.
6. `chipviz-genesis`: VDP plane and palette visualizer for YM2612 plus SN76489.
7. `chipviz-nes`: NROM tile/palette visualizer for 2A03 telemetry.
8. `chipviz-sms`: SMS VDP channel-meter visualizer for SN76489 telemetry.
9. `chipviz-pi5` / `chipviz-m1`: 4K data fields, layered planes, and camera motion using the same scene controls at different quality budgets; the modern renderer also offers catalog-driven Starfield scene 6.

## Releases

Merges to `main` run release-please. Published releases build individual assets:
modern renderer binaries, `.cvz` demos, N64 Joybus and USB HID mapped streams,
generated C arrays, and current homebrew artifacts. The release asset workflow
builds `chipviz-gba.gba` with devkitPro, `chipviz-n64.z64` with libdragon,
`chipviz-c64.prg` with cc65, `chipviz-gb.gb` with RGBDS, and
`chipviz-nes.nes` with ca65. Generated C64/Genesis/NES/SMS assets remain
SDK-free fallbacks for local packaging when those toolchains are not installed.

The `chipviz-v0.1.4` modern renderer assets are x86-64 Linux binaries, as are
the current release workflow's outputs. The `chipviz-pi5` and `chipviz-m1`
names select rendering profiles; those files are not native ARM64 Raspberry Pi
or macOS executables. To run on those targets, build from source as described
in the [modern renderer guide](renderers/modern/README.md). Native release work
is tracked in [#215](https://github.com/djensenius/chipviz/issues/215) and
[#216](https://github.com/djensenius/chipviz/issues/216).

All SNES ROMs, including the generated diagnostic seed and SDK build, are
temporarily omitted from future individual release assets and combined zips;
past releases are unchanged.
The SDK build links an allocator whose redistribution permission is unresolved;
this is a precaution, not a legal determination. Restoring releases awaits a
clean-source or permission resolution and symbol-scan safeguard in
[#109](https://github.com/djensenius/chipviz/issues/109). Local builds and
emulator use remain available; the generated seed is not a visualizer.
The release guard allows only known asset names; adding a new release asset
requires updating `scripts/check-release-assets.py` after policy review.

## Current scaffold

The first checked-in code path is intentionally portable C and Python so it can
validate without platform SDKs installed:

```sh
mise run check
just lint
just test
just simulate
just modern-m1 -- --udp 127.0.0.1:6464
just modern-m1 -- --scene 6 --star-catalog /path/to/stars.csv --location 45.5,-73.25
python3 host/bridge/chipviz_bridge.py --frames 120 --output build/demo.cvz
python3 host/bridge/chipviz_encode.py --verify-fixtures
```

The modern renderer's scene 6 requires a user-supplied HYG-compatible CSV via
`--star-catalog PATH` (`--catalog` is an alias) and
observer coordinates (`--location LAT,LON`, decimal degrees, longitude east).
An optional `--time UNIX_SECONDS` selects a UTC observation time; otherwise it
uses current UTC. No star catalog is bundled while catalog licensing research
is unresolved. See [`renderers/modern/README.md`](renderers/modern/README.md)
for the CSV schema and the projection's accuracy limits.

Each platform entrypoint consumes the shared `control-frame-v0` shape through a
small connection abstraction. Until a live receiver is supplied, the connection
generates deterministic procedural frames so every target has an immediate
visualization loop.

The procedural formula is an implementation convention, not a v0 wire
requirement. For indices 0..65535, C, Python, and the modern renderer use
the beat count modulo 24 for the note on an onset. The modern renderer formerly
truncated the beat to eight bits first: its fallback notes intentionally change
starting at frame 4096 (48 becomes 64), which can change visuals and generated
captures; frames before 4096 are unchanged.

The generators retain their existing wrap behavior. The C connection restarts
its 16-bit procedural index after 65535, so its next note is 48 with beat 0
and scene 0. Rust and Python continue their wider index: at 65536 the wire
frame is 0 but the beat is 4096, note 64, and scene 1. Both wrap the wire beat
at 65536 beats; Rust calculates notes from that wrapped beat, while Python uses
the full beat before wrapping (at frame 1048576, notes 48 and 64 respectively).
No parity beyond C's 16-bit input domain is promised. Scene, palette, flags,
transport, and flash behavior are unchanged.

## License

Copyright David Jensenius. chipviz is dual licensed:

- **Noncommercial use is free** under the
  [PolyForm Noncommercial License 1.0.0](LICENSE): personal use without any
  anticipated commercial application (including hobby projects and research
  for the benefit of public knowledge), and use by charitable organizations,
  educational institutions, public research organizations, government
  institutions, and the other noncommercial organizations the licence lists.
  The licence text is authoritative.
- **Commercial use needs a separate licence** from David Jensenius. See
  [COMMERCIAL.md](COMMERCIAL.md) for what counts and how to ask.

Contributions are accepted under the contributor licence agreement in
[CONTRIBUTING.md](CONTRIBUTING.md).

Third-party components keep their own licences. Toolchains and SDKs fetched at
build time, such as libdragon, devkitPro, cc65, PVSnesLib, RGBDS, and the
Raspberry Pi pico-sdk, are not redistributed in this repository. Rust crates
are fetched by Cargo under their own licences. The modern Starfield scene
accepts a user-supplied catalog; HYG/catalog redistribution licensing research
is unresolved, so no star data is bundled.

Notices for third-party code shipped in release binaries are in
[THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES): [rust.md](THIRD_PARTY_NOTICES/rust.md)
for the Rust crates in the renderer binaries (regenerate with
`just third-party-notices`) and [roms.md](THIRD_PARTY_NOTICES/roms.md) for SDK
runtime code linked into the ROMs. Releases ship them as
`THIRD_PARTY_NOTICES-*` assets.
