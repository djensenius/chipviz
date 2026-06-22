# chipviz music source v0

`music-source-v0` is a small, human-readable description of a deterministic
musical performance. It is the generic source fixture format used to bake
repeatable demos before live hardware or live audio is connected. The host
encoder converts a music source into a raw `control-frame-v0` stream that the
N64, GBA, C64, and modern renderers can all play back.

A music source is intentionally higher level than `control-frame-v0`: it
describes notes on a timeline plus a fixed tempo, and the encoder derives the
transport, energy, spectrum, and note-window fields. The mapping is fully
deterministic so the same source always encodes to the same bytes.

## Document shape

A music source is a JSON object:

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | integer | Must be `0`. |
| `name` | string | Human-readable label (ignored by the encoder). |
| `bpm` | integer | Tempo `1..255`. |
| `frame_rate` | integer | Render rate in Hz, e.g. `60`. |
| `frame_count` | integer | Number of frames to encode, `1..65536`. |
| `scene` | integer | Scene index `0..255`, default `0`. |
| `palette` | integer | Palette index `0..255`, default `0`. |
| `notes` | array | Note events on the frame timeline. |

Each entry in `notes`:

| Field | Type | Meaning |
| --- | --- | --- |
| `start` | integer | Frame index the note begins on, `0..frame_count-1`. |
| `duration` | integer | Number of frames the note is held, `>= 1`. |
| `note` | integer | MIDI note number `0..127`. |
| `velocity` | integer | Velocity / visual emphasis `0..255`. |

## Deterministic mapping

For each output frame index `i` in `0..frame_count` (all divisions below are
integer/floor division, matching the encoder's `//` operator):

### Transport

- `position = floor(i * bpm * 256 / (frame_rate * 60))` (integer beat phase in
  256ths of a beat).
- `beat_count = (position // 256) & 0xFFFF`.
- `beat_phase = position % 256`.
- `flags.beat` (bit 0) is set when frame `i` starts a new beat, i.e. when
  `position // 256` is greater than the previous frame's value. Frame `0` is a
  beat onset.
- `flags.bar` (bit 1) is set on a beat onset when that beat is a multiple of
  four.

### Notes, energy, and spectrum

- A note is **active** on frame `i` when `start <= i < start + duration`.
- A note is an **onset** on frame `i` when `start == i`.
- `notes[]` holds up to four onset notes, ordered by ascending MIDI note then
  ascending velocity, as `(note, velocity)` pairs.
- Each active note maps to a spectrum band: `band = min(7, note // 16)`.
- `spectrum[band]` is the maximum velocity of active notes in that band, else
  `0`.
- `bass = average(spectrum[0:3])`, `mid = average(spectrum[3:6])`,
  `treble = average(spectrum[6:8])`.
- `energy` is the maximum velocity of all active notes, else `0`.

### Static fields

- `scene` and `palette` come from the document-level fields.
- All reserved `control-frame-v0` flag bits stay clear.

## Validation

The encoder rejects sources with an unsupported version, out-of-range tempo,
non-positive `frame_count`, more than four onset notes on a single frame, or
note fields outside their documented ranges. Inputs and their expected raw
frame outputs are checked in under `host/fixtures/`, and `make check`
re-encodes each input and compares it byte-for-byte against the golden output.
