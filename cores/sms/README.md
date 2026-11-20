# chipviz-sms

Sega Master System / SN76489 PSG visualizer for the ChipStation SN76489 core.

## Homebrew stack

- First SDK target: devkitSMS, SDCC, or a small Z80 assembly project once a
  toolchain is selected.
- Current checked-in fallback is a generated, headered `.sms` ROM seed so
  packaging and emulator smoke paths have a deterministic artifact.

## First demo

- PSG tone/noise channel meters.
- TMS9918/SMS VDP tile and CRAM palette cycling.
- Controller input can later select scene/palette/intensity.

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
PSG tone/noise and tile-phase values for host simulation.
`cores/sms/homebrew` generates `chipviz-sms.sms` through the shared SDK-free
homebrew artifact builder until an SMS SDK is chosen.

## Host-only field assets

`homebrew/field_assets.py` generates deterministic candidate field patterns,
maps and lookup tables into a supplied directory. These files are not used by
the static seed ROM. Python host-model tests are discovered by
`just python-test`; `just sms-procedural-test` generates the assets in a
temporary directory and compares all 65,536 procedural frames against the
shared C control-frame implementation. It also runs as part of `just c-test`.
These checks do not execute SMS code or establish visualization or hardware
coverage.

## Connection path

Start with baked/procedural playback and controller input. A later hardware
bridge can map ChipStation PSG telemetry to a tiny controller-port protocol.

## Constraints

Use tile maps, simple CRAM changes, and channel meters. Avoid framebuffer-style
assumptions.
