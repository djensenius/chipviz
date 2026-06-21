# chipviz control frame v0

This is the first shared live-control shape for chipviz. It is intentionally tiny so it can map onto very limited transports, including controller ports, serial links, USB debug channels, MIDI-derived streams, and precomputed playback files.

## Goals

- Fit the same musical intent across N64, GBA, and C64.
- Keep the core frame small enough for low-bandwidth links.
- Let every platform interpret the frame natively instead of forcing identical visuals.

## Frame fields

| Field | Type | Meaning |
| --- | --- | --- |
| `magic` | `u8` | Constant `0xC7`, used for sync. |
| `version` | `u8` | Protocol version, currently `0`. |
| `frame` | `u16` | Monotonic frame counter, wraps. |
| `bpm` | `u8` | BPM rounded to nearest integer, `0` if unknown. |
| `beat_phase` | `u8` | `0..255` phase within the current beat. |
| `beat_count` | `u16` | Beat counter, wraps. |
| `energy` | `u8` | Overall intensity, `0..255`. |
| `bass` | `u8` | Low-frequency intensity, `0..255`. |
| `mid` | `u8` | Mid-frequency intensity, `0..255`. |
| `treble` | `u8` | High-frequency intensity, `0..255`. |
| `spectrum` | `u8[8]` | Eight compact frequency bands. |
| `scene` | `u8` | Scene index selected by host or controller. |
| `palette` | `u8` | Palette index selected by host or controller. |
| `flags` | `u8` | Bitfield for trigger-like state. |
| `note_count` | `u8` | Number of note events encoded in this frame. |
| `notes` | `note[4]` | Up to four compact note events. |
| `checksum` | `u8` | XOR of all prior bytes in the frame. |

## Note event

| Field | Type | Meaning |
| --- | --- | --- |
| `note` | `u8` | MIDI note number, `0..127`; values above `127` are reserved. |
| `velocity` | `u8` | Note velocity or visual emphasis, `0..255`. |

## Flags

| Bit | Name | Meaning |
| --- | --- | --- |
| `0` | `beat` | Set on beat onset. |
| `1` | `bar` | Set on bar onset. |
| `2` | `drop` | Set on major transition/drop. |
| `3` | `fill` | Set during fills or busy sections. |
| `4` | `manual` | Set when controller/manual input is overriding host data. |
| `5..7` | reserved | Must be `0` for v0. |

## Transport mappings

### N64 controller-port transport

If using emulated N64 controllers as the input bus, treat each controller poll as a small packet. Four controller ports at 60 Hz can carry enough data for tempo, energy, scene, palette, and several spectrum bands per rendered frame. The exact bit packing should live beside the N64 core once the hardware transport is chosen.

### File playback

For offline demos, frames can be stored as raw fixed-size records or converted into platform-specific arrays. This keeps early demos deterministic before live host bridging exists.

