# chipsynth source mapping v0

This source adapter consumes `chipsynth-viz-stream-v0` and maps it into
`control-frame-v0`.

## Field mapping

| control-frame field | chipsynth source |
| --- | --- |
| `frame` | Stream `sequence`, or local reducer frame counter for merged sources. |
| `bpm` | `round(bpm_x100 / 100)`, clamped to `0..255`. |
| `beat_phase` | Stream `beat_phase`. |
| `beat_count` | Stream `beat_count`. |
| `energy` | Maximum channel/voice level, or audio adapter value if audio is present. |
| `bass` | Average channel levels `0..3`, unless audio adapter owns bass. |
| `mid` | Average channel levels `4..11`, unless audio adapter owns mid. |
| `treble` | Average channel levels `12..15`, unless audio adapter owns treble. |
| `spectrum[8]` | Pairwise averages of 16 channel levels. |
| `scene` | Dominant active voice chip ID, or manual/MIDI scene override. |
| `palette` | Dominant active voice channel, or manual/MIDI palette override. |
| `flags.beat` | Stream `beat` flag. |
| `flags.bar` | Stream `bar` flag. |
| `flags.fill` | Stream `fill` flag. |
| `notes` | Up to four active/gated voice records as `(note, velocity)`. |

## Merge behavior

The chipsynth adapter is musical/control telemetry. It should not carry audio.
When an audio adapter is active, audio-derived FFT/beat fields can override
energy and spectrum fields while chipsynth continues to contribute notes,
channel/chip identity, scene/palette hints, transport, and performance gestures.

## Native rendering hints

- N64: map voices to 3D emitters and chip/channel-colored planes.
- Pocket/GBA/SNES: map channel activity to tile/sprite groups.
- C64: map channel levels and chip IDs to raster/PETSCII regions.
- Modern 4K: map voices and channels to dense 3D data fields and spectrum
  volumes.
