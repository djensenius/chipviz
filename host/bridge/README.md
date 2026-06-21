# chipviz host bridge

Desktop-side bridge for turning live or offline music data into chipviz control frames.

## Planned responsibilities

- Read MIDI, clock, or audio-analysis data.
- Quantize it into `control-frame-v0`.
- Send frames to platform-specific transports.
- Record and replay sessions for deterministic demos.

## Candidate transports

| Target | Transport idea |
| --- | --- |
| N64 | ESP32 emulating N64 controllers, SummerCart64 USB I/O, or precomputed ROM data. |
| GBA / Analogue Pocket + Dock | Precomputed ROM data first; Dock controller input and link cable bridge later. |
| Commodore 64 Ultimate | Precomputed tables first; Ethernet/Wi-Fi/network services or user-port serial bridge later. |
