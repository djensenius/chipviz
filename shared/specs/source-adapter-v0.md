# chipviz source adapter v0

Source adapters convert music/control inputs into `control-frame-v0`. Platform
renderers must only consume the resulting control frame; they should not contain
source-specific logic.

## Adapter classes

| Adapter | Role |
| --- | --- |
| Procedural | Deterministic fallback for self-running demos and smoke tests. |
| File playback | Replays raw `control-frame-v0` records for baked demos. |
| MIDI listener | Maps notes, CC, clock, transport, and performance gestures. |
| Audio listener | Computes FFT, beat detection, and intensity from routed audio. |
| chipsynth stream | Consumes ChipStation visualization stream packets from sibling `../chipsynth`. |

## Timing model

- The reducer emits `control-frame-v0` at the target frame rate, usually 60 Hz.
- Live adapters are latest-state inputs. If several packets arrive between
  render frames, keep the newest valid state.
- Recorded adapters must be deterministic so generated fixtures can be compared
  byte-for-byte.

## Merging model

Multiple adapters can contribute to one output frame:

1. Transport/clock data sets `bpm`, `beat_phase`, `beat_count`, and beat/bar
   flags when available.
2. Audio analysis owns `energy`, `bass`, `mid`, `treble`, and `spectrum` when
   available.
3. chipsynth telemetry can fill energy/spectrum gaps from channel and voice
   levels, and always contributes note/chip/channel state.
4. Manual or MIDI control data can set `scene`, `palette`, and mode flags.
5. Missing fields use explicit neutral defaults, never stale unvalidated data.

## Validation

Adapters must clamp numeric values to their destination field ranges, reject
malformed input packets, and preserve deterministic replay for fixtures.
Reserved flag bits in `control-frame-v0` must remain clear.
