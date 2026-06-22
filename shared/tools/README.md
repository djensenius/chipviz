# chipviz shared tools

Utilities for converting host-side ideas into platform-specific data.

Planned tools:

- palette generation and quantization
- scene-data packing
- `control-frame-v0` encoders
- MIDI/audio-analysis preprocessing
- platform asset exporters

## Current bridge helper

`host/bridge/chipviz_bridge.py` emits raw fixed-size `control-frame-v0` packets
from a deterministic procedural source. It can write playback files now and can
also send frames over UDP for early transport experiments:

```sh
python3 host/bridge/chipviz_bridge.py --frames 120 --output build/demo.cvz
python3 host/bridge/chipviz_bridge.py --frames 120 --udp 127.0.0.1:6464
```

## Golden fixtures and encoder

`host/bridge/chipviz_encode.py` converts checked-in fixtures into deterministic
raw `control-frame-v0` streams for repeatable demos. It encodes generic
`music-source-v0` JSON timelines and chipsynth event logs, and validates the
golden outputs under `host/fixtures/`:

```sh
python3 host/bridge/chipviz_encode.py \
  --music host/fixtures/musical/scale.musicsource.json --output build/scale.cvz
python3 host/bridge/chipviz_encode.py --verify-fixtures
```

`make check` runs `--verify-fixtures` so the encoder stays byte-for-byte stable.
