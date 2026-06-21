# chipviz host bridge

Bridge software for turning live or offline music data into chipviz control frames.
The live sender/audio-processing device should be an ESP32 or Raspberry Pi, with
a desktop computer used as the development host.

## Planned responsibilities

- Read MIDI, clock, or audio-analysis data.
- Connect to the optional chipsynth visualization stream.
- Quantize it into `control-frame-v0`.
- Send frames to platform-specific transports.
- Record and replay sessions for deterministic demos.

## Sender hardware

| Device | Best role |
| --- | --- |
| ESP32 | Low-latency transport, USB serial input, Wi-Fi UDP, Bluetooth HID, N64 controller-port GPIO, simple MIDI/control reduction. |
| Raspberry Pi | Audio analysis, multiple USB MIDI/audio devices, filesystem-backed recordings, chipsynth stream client, USB HID output, LAN output, and Pi-to-Pico N64 bridge. |
| Desktop computer | Development, debugging, baked frame generation, and parity with the Pi bridge before deployment. |

## Candidate transports

| Target | Transport idea |
| --- | --- |
| N64 | Raspberry Pi to Pi Pico over USB serial; Pico emulates four N64 controllers with PIO Joybus. |
| GBA / Analogue Pocket + Dock | Raspberry Pi as USB HID controller through the Dock first; baked ROM data and link cable bridge later. |
| Commodore 64 Ultimate | Raspberry Pi as USB HID keyboard/joystick first; network, user-port, or precomputed tables as fallback. |

## chipsynth stream

chipsynth should publish a continuous visualization stream that chipviz connects
to as a client. The stream mirrors the MIDI/control data sent to Daisy and adds
Daisy telemetry when available, including active voices, channel/chip assignment,
note, velocity, level, parameter changes, transport/BPM, and performance
gestures.

Audio routing stays independent. The bridge can combine chipsynth telemetry with
local audio analysis from the Pi, desktop, or another source.

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

`chipsynth_stream.py` parses the sibling ChipStation visualization stream
(`CSV0`) and maps it into `control-frame-v0`:

```sh
python3 host/bridge/chipsynth_stream.py \
  --demo-packet build/chipsynth-viz-stream-v0.bin \
  --output build/chipsynth-derived.cvz
```
