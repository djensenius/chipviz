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
| GBA / Analogue Pocket + Dock | Bluetooth controller input through the Dock first; baked ROM data and link cable bridge later. |
| Commodore 64 Ultimate | Network service on the LAN first if practical; user-port serial or precomputed tables as fallback. |

## Current scaffold

`chipviz_bridge.py` emits deterministic procedural `control-frame-v0` packets as
raw 33-byte records. Use file output for baked playback data, or UDP output for
early live transport experiments:

```sh
python3 host/bridge/chipviz_bridge.py --frames 120 --output build/demo.cvz
python3 host/bridge/chipviz_bridge.py --frames 120 --udp 127.0.0.1:6464
```

See [`../../docs/connections.md`](../../docs/connections.md) for target-specific
connection plans, including the ESP32 bridge path.
