# chipviz host bridge

Bridge software for turning live or offline music data into chipviz control frames.
The live sender/audio-processing device should be an ESP32 or Raspberry Pi, with
a desktop computer used as the development host.

## Planned responsibilities

- Read MIDI, clock, or audio-analysis data.
- Quantize it into `control-frame-v0`.
- Send frames to platform-specific transports.
- Record and replay sessions for deterministic demos.

## Sender hardware

| Device | Best role |
| --- | --- |
| ESP32 | Low-latency transport, USB serial input, Wi-Fi UDP, Bluetooth HID, N64 controller-port GPIO, simple MIDI/control reduction. |
| Raspberry Pi | Audio analysis, multiple USB MIDI/audio devices, filesystem-backed recordings, richer chipsynth integration, LAN output to C64 Ultimate or ESP32. |
| Desktop computer | Development, debugging, baked frame generation, and parity with the Pi bridge before deployment. |

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
connection plans, including ESP32 and Raspberry Pi sender paths.
