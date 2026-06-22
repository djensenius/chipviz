# chipviz golden fixtures

Deterministic sample inputs and their expected `control-frame-v0` outputs. The
host encoder (`host/bridge/chipviz_encode.py`) re-encodes each input and
`make check` compares it byte-for-byte against the golden `.cvz` output, so the
encoder path stays repeatable for tests, emulator runs, and hardware debugging.

Each `.cvz` file is a raw stream of 33-byte `control-frame-v0` records and can be
replayed directly by the N64, GBA, C64, and modern renderers.

## Layout

| Path | Kind | Meaning |
| --- | --- | --- |
| `musical/*.musicsource.json` | input | Generic `music-source-v0` timelines. |
| `musical/*.cvz` | golden | Expected frame stream for the matching source. |
| `chipsynth/*.csv0` | input | chipsynth event logs (concatenated `CSV0` packets). |
| `chipsynth/*.cvz` | golden | Expected frame stream for the matching event log. |

## Regenerating

Golden outputs are derived; do not hand-edit them. After an intentional encoder
change, regenerate and review the diff:

```sh
python3 host/bridge/chipviz_encode.py --write-fixtures
python3 host/bridge/chipviz_encode.py --verify-fixtures
```

The chipsynth event-log input is itself deterministic and can be regenerated
with:

```sh
python3 host/bridge/chipsynth_stream.py \
  --demo-log host/fixtures/chipsynth/groove.csv0 --demo-log-frames 96
```

See [`../../shared/specs/music-source-v0.md`](../../shared/specs/music-source-v0.md)
for the music source format and mapping, and
[`../../shared/specs/chipsynth-source-v0.md`](../../shared/specs/chipsynth-source-v0.md)
for the chipsynth mapping.
