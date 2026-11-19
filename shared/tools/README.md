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

## Capture assertions

`shared/tools/capture_assert.py` inspects PNG captures that another tool has
already produced (for example `scripts/retroarch_smoke.py` or a renderer
harness). It does not launch emulators, renderers or frontends. It uses only
the Python standard library and decodes non-interlaced 8-bit RGB/RGBA PNGs;
alpha is ignored. Palette, grayscale, 16-bit, interlaced, truncated or
CRC-corrupt files are rejected, as are split (non-consecutive) `IDAT` data and
unknown critical chunks. Bytes after `IEND` are ignored.

It always prints measurements as JSON: file SHA-256, dimensions, distinct RGB
colours, Rec.601 luma mean/stddev and lit pixel count/fraction for the
selected region, plus the changed-pixel count/fraction and maximum channel
delta between consecutive images. `steps_comparable` is `false` when image or
region sizes differ (no steps are reported). Measured `lit_fraction` and
`changed_fraction` values in `frames` and `steps` are rounded to six decimals
for display only; assertions compare exact pixel counts, so a single changed
or lit pixel in a 1080p frame still counts. It applies only the assertions you
request. Fraction options accept plain decimal or ratio text (`0.3`, `.3`,
`3/10`, `1/3`) of at most 32 characters, without signs or exponents, and are
parsed exactly, so `--max-lit-fraction 0.3` accepts exactly 3 of 10 lit
pixels. `parameters.change_fraction` in the report is that exact parsed value
as a string, such as `"3/10"`. Trailing bytes after the zlib stream and `PLTE` after `IDAT` are
rejected. Exit status is `0` when every requested assertion passes, `1` when
one fails and `2` for invalid input, usage or out-of-range options.

```sh
# SNES backdrop-only failure: one colour on the full 256x224 frame.
python3 shared/tools/capture_assert.py --expect-size 256x224 --min-colors 2 \
  --meta target=snes --meta rom_sha256=<sha256> --meta core="Snes9x 1.63" \
  --meta frame_label=retroarch-max-frames-1200 snes-f001200.png

# Modern blank output: require some lit pixels in a known scene.
python3 shared/tools/capture_assert.py --expect-size 1920x1080 \
  --min-colors 2 --min-lit-fraction 0.01 m1-proc-030-signalfield.png

# Sequence checks, optionally limited to a region and with a tolerance.
python3 shared/tools/capture_assert.py --sequence changes \
  --region 0,160,256,64 --tolerance 2 f1200.png f1201.png f1216.png
```

| Option | Meaning |
| --- | --- |
| `--region X,Y,W,H` | Measure and compare only this rectangle |
| `--expect-size WxH` | Each image has exactly these dimensions |
| `--min-colors N`, `--max-colors N` | Distinct RGB colour bounds (non-negative, min <= max) |
| `--min-luma-stddev F` | Minimum luma standard deviation (0-255) |
| `--min-lit-fraction F`, `--max-lit-fraction F` | Lit-pixel fraction bounds (0-1, min <= max) |
| `--lit-threshold N` | A pixel is lit when its integer luma exceeds `N` (0-255, default 8) |
| `--sequence identical` | No consecutive step changes |
| `--sequence changes` | At least one consecutive step changes |
| `--sequence every-step-changes` | Every consecutive step changes |
| `--tolerance N` | Per-channel delta still treated as unchanged (default 0) |
| `--change-fraction F` | A step changes only when its changed fraction exceeds `F` (0 <= F < 1, default 0) |
| `--meta KEY=VALUE` | Provenance echoed under `provenance` in the report |

There are no universal thresholds. Sparse scenes, intentionally static
Genesis/SMS seeds, blank glyphs, nondeterministic boot RAM and capture timing
mean every threshold, region and warmup allowance must be chosen for a
specific target and scene. A changing sequence is not proof of correct
output: a backdrop-only SNES capture can change colour while still having one
colour per frame, so combine motion checks with content checks. Equal hashes
or passing assertions do not prove visual fidelity, hardware behaviour or that
the inputs match their recorded identities.

To reproduce an invocation, record these alongside the command (for example
with `--meta`):

- source commit and ROM/binary SHA-256, verified against the file actually
  loaded rather than copied from earlier notes;
- frontend, core and version/hash, core options and renderer profile/size;
- input script or held buttons, including when each input starts and stops;
- frame selection: say whether a number is a frontend label (such as a
  RetroArch `--max-frames` value) or a directly observed core/callback/NMI
  count, and state any target-specific warmup frames excluded;
- the full option list used: region, lit threshold, tolerance, change fraction
  and every bound.
