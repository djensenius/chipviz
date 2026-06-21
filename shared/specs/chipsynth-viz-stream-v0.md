# chipsynth visualization stream v0

This is the consumer-side copy of the ChipStation visualization stream contract
from sibling `../chipsynth/docs/chipstation-viz-stream-v0.md`.

The stream is latest-state input for chipviz. It mirrors MIDI/control data sent
to Daisy and adds Daisy telemetry when available. Audio is intentionally not
part of this stream; chipviz can capture audio separately and merge FFT/beat
analysis with the chipsynth state.

## Packet layout

All multibyte integer fields are little-endian. The packet is 81 bytes.

| Offset | Size | Field | Meaning |
| --- | --- | --- | --- |
| 0 | 4 | `magic` | ASCII `CSV0` (`43 53 56 30`). |
| 4 | 1 | `version` | `0`. |
| 5 | 1 | `flags` | Frame flags. |
| 6 | 2 | `sequence` | Monotonic packet counter, wraps. |
| 8 | 2 | `bpm_x100` | BPM multiplied by 100, `0` if unknown. |
| 10 | 1 | `beat_phase` | `0..255` phase through the current beat. |
| 11 | 2 | `beat_count` | Beat counter, wraps. |
| 13 | 1 | `active_voice_count` | Active voices represented in `voices`, capped to 4. |
| 14 | 1 | `midi_note_count` | Note-like events represented in `voices`, capped to 4. |
| 15 | 1 | `reserved` | Must be `0`. |
| 16 | 16 | `channel_levels` | Per-MIDI-channel level/intensity, `0..255`. |
| 32 | 16 | `channel_chips` | Per-MIDI-channel ChipStation chip IDs. |
| 48 | 32 | `voices` | Four 8-byte voice records. |
| 80 | 1 | `checksum` | XOR of bytes `0..79`. |

## Chip IDs

| ID | Chip |
| --- | --- |
| `0x00` | Test tone |
| `0x01` | SN76489 PSG |
| `0x02` | Game Boy APU |
| `0x03` | YM2612 |
| `0x04` | NES 2A03 |
| `0x05` | SID |

## Voice record

| Record offset | Size | Field | Meaning |
| --- | --- | --- | --- |
| 0 | 1 | `voice` | Synth voice index. |
| 1 | 1 | `channel` | MIDI channel `0..15`. |
| 2 | 1 | `chip` | Chip ID from the table above. |
| 3 | 1 | `note` | MIDI note `0..127`. |
| 4 | 1 | `velocity` | MIDI velocity or visual emphasis, `0..255`. |
| 5 | 1 | `level` | Current voice level, `0..255`. |
| 6 | 1 | `flags` | Voice flags. |
| 7 | 1 | `reserved` | Must be `0`. |

## Flags

Frame flags:

| Bit | Name | chipviz mapping |
| --- | --- | --- |
| 0 | `playing` | Source is active; no direct control-frame flag. |
| 1 | `beat` | `control-frame-v0.flags.bit0`. |
| 2 | `bar` | `control-frame-v0.flags.bit1`. |
| 3 | `fill` | `control-frame-v0.flags.bit3`. |
| 4..7 | reserved | Must be `0`. |

Voice flags:

| Bit | Name | Meaning |
| --- | --- | --- |
| 0 | `active` | Voice is active. |
| 1 | `gate` | Gate is currently high. |
| 2..7 | reserved | Must be `0`. |

## Validation

chipviz must ignore packets with bad magic, unsupported version, nonzero
reserved bytes, invalid channel/chip/note values, unknown flag bits, or checksum
failure.
