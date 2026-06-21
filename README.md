# chipviz

Original visualization software for real retro hardware and FPGA consoles.

## Initial targets

| Target | Output | Hardware path | Role |
| --- | --- | --- | --- |
| N64 / Analogue 3D | `.z64` | Flashcart on Analogue 3D or N64 | Flagship big-screen 3D/particle visualizer. |
| GBA / Analogue Pocket | `.gba` | GBA cartridge, flashcart, or Pocket adapter path | Portable 2D sketchpad with palettes, sprites, and affine effects. |
| Commodore 64 / THEC64 | `.prg` / `.d64` | THEC64 USB media or real C64 storage | VIC-II/SID/raster/PETSCII visual personality. |

## Project layout

```text
cores/
  n64/      N64 homebrew target, likely libdragon + tiny3d.
  gba/      GBA homebrew target, likely Butano or Tonc.
  c64/      C64 target, likely cc65 or KickAssembler.
shared/
  specs/    Cross-platform control protocol and scene model.
  tools/    Asset conversion, palettes, MIDI/audio preprocessing.
host/
  bridge/   Desktop bridge for MIDI/audio analysis and hardware transport.
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

See [`shared/specs/control-frame-v0.md`](shared/specs/control-frame-v0.md).

## Plan

chipviz can run from chipsynth data, generic MIDI/clock input, audio-analysis listeners, recorded sessions, or self-running procedural sources. See [`PLAN.md`](PLAN.md).

## First milestone

Build one self-running demo per platform before live input:

1. `chipviz-n64`: rotating geometry, palette cycling, controller-driven scene changes.
2. `chipviz-gba`: sprite/tile/affine visualizer with beat simulation.
3. `chipviz-c64`: raster bars, PETSCII patterning, SID/noise-reactive fake input loop.
