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
(`CSV0`) and maps it into `control-frame-v0`. It maps either a single packet or
a multi-packet event log, and can emit a deterministic demo log for fixtures:

```sh
python3 host/bridge/chipsynth_stream.py \
  --demo-packet build/chipsynth-viz-stream-v0.bin \
  --output build/chipsynth-derived.cvz

python3 host/bridge/chipsynth_stream.py \
  --stream host/fixtures/chipsynth/groove.csv0 \
  --output build/chipsynth-log.cvz
```

## Golden fixtures and the host encoder

`chipviz_encode.py` is the host-side encoder that turns checked-in fixtures into
deterministic raw `control-frame-v0` streams. The resulting `.cvz` files are
plain sequences of 33-byte frames, so they replay directly through the N64, GBA,
C64, and modern playback paths.

It encodes two fixture kinds:

- `music-source-v0` JSON, a generic musical timeline
  (see [`../../shared/specs/music-source-v0.md`](../../shared/specs/music-source-v0.md)).
- chipsynth event logs, concatenated `CSV0` packets.

```sh
# Encode a generic musical source into a raw frame stream.
python3 host/bridge/chipviz_encode.py \
  --music host/fixtures/musical/scale.musicsource.json \
  --output build/scale.cvz

# Encode a chipsynth event log into a raw frame stream.
python3 host/bridge/chipviz_encode.py \
  --chipsynth-log host/fixtures/chipsynth/groove.csv0 \
  --output build/groove.cvz

# Validate the encoder against the checked-in golden outputs (run by `make check`).
python3 host/bridge/chipviz_encode.py --verify-fixtures
```

Inputs and their expected outputs live under
[`../fixtures/`](../fixtures). Regenerate the golden outputs after an
intentional encoder change with `--write-fixtures`.


## Colour-field flash guard

Scene 7 is the colour-field scene, and palette `p = palette & 7` of 3 or 7
selects its flash/gate mode (see
[`colour-field-v0`](../../shared/specs/colour-field-v0.md)). Every host
scene source (`live_bridge.py`, `chipsynth_stream.py`, `chipviz_bridge.py`,
`chipviz_encode.py`, `n64_joybus.py` and `usb_hid.py`) sends frames through
one shared guard, `chipviz_bridge.sanitise_flash`. Unless the process was
started with `--allow-flash`, a frame whose *decoded scene* is 7 has palette
bit 0 cleared before transmission, so 3 and 7 go out as 2 and 6. The decoded
scene is the one the receiver sees on each transport:

| Transport | Decoded scene |
| --- | --- |
| Full control frame, `.cvz` file, UDP | `scene` byte |
| N64 Joybus | `scene & 0x0F` (23 decodes as 7) |
| USB HID | `scene & 0x07` (15 decodes as 7) |

The guard runs per transport, so one frame sent to a file and to Joybus is
checked separately for each. In `live_bridge.py` MIDI mode this also covers
CC2: selecting palette 3 or 7 while in scene 7 needs `--allow-flash`.

`--allow-flash` disables the guard and prints this warning to stderr:

> WARNING: Flash/gate mode produces full-field flashing light at up to 7.5
> flashes per second. This exceeds the WCAG 2.3.1 and Harding limits of 3
> flashes per second. Saturated red flashes are the highest-risk case. It may
> trigger seizures in people with photosensitive epilepsy. Enable it only where
> performers and audience have been warned.

`live_bridge.py` audio mode cycles scenes 0–6 and never selects scene 7.

### MIDI CC1 scene debounce

`live_bridge.py` MIDI mode maps CC1 to `scene = value // 16`. A control
resting near the 6/7 boundary must not toggle the sent scene across 7, so the
bridge debounces with the target's scene-7 constants, counted in 60 Hz steps:

- A change into or out of scene 7 is sent only after the CC1-derived scene
  has been unchanged for 30 consecutive steps (`P_STABLE`). It takes effect
  at the end of the 30th step and is sent from the next frame. Any change
  restarts the count.
- No scene-7 entry or exit is sent sooner than 20 steps (`CUT_MIN`) after
  the previous one.
- Changes between other scenes are sent at once.

Both minimums must hold. Every output must have transmitted at least 30
frames (20 for `CUT_MIN`), because targets count one step per received frame;
frames dropped by the 60 fps cap below do not count. The wait must also last
at least 500 ms (333 ms for `CUT_MIN`): above 60 Hz the source frame counts
scale with the rate (60 frames at `--rate 120`). At `--rate 90`, entering
scene 7 takes 45 frames (500 ms), and leaving takes 30 transmitted frames
(about 60 frames, roughly 660 ms), because the cap passes only about 45
scene-7 frames per second. Targets apply their own scene-7 debounce as a
backstop.

### Colour-field frame-rate cap

Hosts must not send more than 60 colour-field frames per second (see
"Step rate" in [`colour-field-v0`](../../shared/specs/colour-field-v0.md)).
`live_bridge.py` caps each real-time output (`--udp`, `--n64-serial`)
separately. Only frames whose *decoded scene* on that output is 7 (same table
as the flash guard) are limited; other frames always go out.

- Scene-7 frames follow a 60 Hz slot grid. A frame is sent once it is no more
  than a quarter slot early, which absorbs pacing jitter, and never when 60
  scene-7 frames were already sent in the second before it (half-open
  window: any `[t, t + 1 s)` holds at most 60). Other frames are dropped.
- While the scene is 7, `--rate` 120 (or any multiple of 60) gives 60
  frames/s. Other rates above 60 give between about rate/2 and 60 (45 at 90,
  50 at 100, 55 at 110, about 58 at 144). Rates up to 60 are unchanged.
- Frames are only dropped, never reordered or renumbered, so the frame
  counter stays monotonic and skips values; targets step once per new value.
- Events on dropped frames are carried per target event class, so the
  target sees each class at most once per sent frame, as if it had resampled
  the input itself: `B` (a `beat` or `bar` rising edge), `X` (a `drop`
  rising edge) and `N` (a note onset: a new note at velocity 64 or more on the
  full path; byte 15, the highest velocity, rising across 64 on Joybus).
- A pending class is cleared as soon as a sent frame already shows that class
  against the previous sent frame, whichever event caused it (a new beat
  satisfies a pending bar; any new loud note satisfies a pending note).
  Otherwise one pending event is written in: a flag bit, or a note in a free
  slot or in place of the quietest note below velocity 64 (on Joybus that
  raises byte 15 to the carried velocity).
- A class that the previous sent frame already shows cannot be an edge yet,
  so it is held for at most 2 sends and then dropped. A note that only lacks
  a slot (all 4 hold louder notes) waits up to 30 sends, so it can arrive late
  but never doubles an onset.
- Invalid frames (bad magic, version, note count or checksum, or reserved
  flag bits set) never raise or receive carried events, are never the edge
  reference, and go out as they are.
- `--output` files are not capped: a `.cvz` file has no wall-clock rate, and
  whatever replays it paces and caps it. The other host tools only write
  files or burst procedural frames that never use scene 7.
