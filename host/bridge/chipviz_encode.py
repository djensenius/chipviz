#!/usr/bin/env python3
"""Encode golden fixtures into deterministic control-frame-v0 streams.

Two fixture kinds are supported:

* music-source-v0 JSON, a generic musical timeline (see
  ``shared/specs/music-source-v0.md``).
* chipsynth event logs, a stream of concatenated chipsynth-viz-stream-v0
  packets, mapped through ``chipsynth_stream``.

The same encoder validates the checked-in golden outputs so ``make check`` can
guarantee the host encoder stays byte-for-byte deterministic.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from chipviz_bridge import (
  MAX_NOTES,
  SPECTRUM_BANDS,
  ControlFrame,
  Note,
  clamp_byte,
  pack_frame,
)
from chipsynth_stream import encode_event_log


SUPPORTED_VERSION = 0
FLAG_BEAT = 1 << 0
FLAG_BAR = 1 << 1

FIXTURES_ROOT = Path(__file__).resolve().parents[2] / "host" / "fixtures"
FIXTURES = (
  ("music", "musical/scale.musicsource.json", "musical/scale.cvz"),
  ("music", "musical/groove.musicsource.json", "musical/groove.cvz"),
  ("chipsynth", "chipsynth/groove.csv0", "chipsynth/groove.cvz"),
)


def _require_int(source: dict, key: str, default: int | None = None) -> int:
  if key not in source:
    if default is not None:
      return default
    raise ValueError(f"music source missing required field: {key}")
  value = source[key]
  if isinstance(value, bool) or not isinstance(value, int):
    raise ValueError(f"music source field {key} must be an integer")
  return value


def _average(values: list[int]) -> int:
  if not values:
    return 0
  return clamp_byte(sum(values) // len(values))


def encode_music_source(source: dict) -> bytes:
  """Encode a music-source-v0 document into a raw control-frame-v0 stream."""
  version = _require_int(source, "version")
  if version != SUPPORTED_VERSION:
    raise ValueError(f"unsupported music source version: {version}")

  bpm = _require_int(source, "bpm")
  if not 1 <= bpm <= 255:
    raise ValueError("music source bpm must be 1..255")

  frame_rate = _require_int(source, "frame_rate")
  if frame_rate < 1:
    raise ValueError("music source frame_rate must be >= 1")

  frame_count = _require_int(source, "frame_count")
  if not 1 <= frame_count <= 65536:
    raise ValueError("music source frame_count must be 1..65536")

  scene = _require_int(source, "scene", 0)
  palette = _require_int(source, "palette", 0)
  if not 0 <= scene <= 255 or not 0 <= palette <= 255:
    raise ValueError("music source scene/palette must be 0..255")

  notes = _parse_notes(source.get("notes", []), frame_count)

  divisor = frame_rate * 60
  packets = []
  previous_beat = -1
  for index in range(frame_count):
    position = (index * bpm * 256) // divisor
    beat_index = position // 256
    beat_phase = position % 256
    flags = 0
    if beat_index != previous_beat:
      flags |= FLAG_BEAT
      if beat_index % 4 == 0:
        flags |= FLAG_BAR
    previous_beat = beat_index

    active = [note for note in notes if note.start <= index < note.start + note.duration]
    onsets = sorted(
      (note for note in active if note.start == index),
      key=lambda note: (note.note, note.velocity),
    )
    if len(onsets) > MAX_NOTES:
      raise ValueError(f"frame {index} has more than {MAX_NOTES} note onsets")

    spectrum = [0] * SPECTRUM_BANDS
    for note in active:
      band = min(SPECTRUM_BANDS - 1, note.note // 16)
      spectrum[band] = max(spectrum[band], note.velocity)

    energy = max((note.velocity for note in active), default=0)

    frame = ControlFrame(
      frame=index & 0xFFFF,
      bpm=bpm,
      beat_phase=beat_phase,
      beat_count=beat_index & 0xFFFF,
      energy=clamp_byte(energy),
      bass=_average(spectrum[0:3]),
      mid=_average(spectrum[3:6]),
      treble=_average(spectrum[6:8]),
      spectrum=tuple(spectrum),
      scene=scene,
      palette=palette,
      flags=flags,
      notes=tuple(Note(note.note, note.velocity) for note in onsets),
    )
    packets.append(pack_frame(frame))

  return b"".join(packets)


class _SourceNote:
  __slots__ = ("start", "duration", "note", "velocity")

  def __init__(self, start: int, duration: int, note: int, velocity: int) -> None:
    self.start = start
    self.duration = duration
    self.note = note
    self.velocity = velocity


def _parse_notes(raw_notes: object, frame_count: int) -> list[_SourceNote]:
  if not isinstance(raw_notes, list):
    raise ValueError("music source notes must be a list")
  notes = []
  for entry in raw_notes:
    if not isinstance(entry, dict):
      raise ValueError("each music source note must be an object")
    start = _require_int(entry, "start")
    duration = _require_int(entry, "duration")
    note = _require_int(entry, "note")
    velocity = _require_int(entry, "velocity")
    if not 0 <= start < frame_count:
      raise ValueError("note start must be within 0..frame_count-1")
    if duration < 1:
      raise ValueError("note duration must be >= 1")
    if not 0 <= note <= 127:
      raise ValueError("note value must be 0..127")
    if not 0 <= velocity <= 255:
      raise ValueError("note velocity must be 0..255")
    notes.append(_SourceNote(start, duration, note, velocity))
  return notes


def encode_path(kind: str, path: Path) -> bytes:
  if kind == "music":
    return encode_music_source(json.loads(path.read_text()))
  if kind == "chipsynth":
    return encode_event_log(path.read_bytes())
  raise ValueError(f"unknown fixture kind: {kind}")


def verify_fixtures(root: Path = FIXTURES_ROOT) -> int:
  failures = 0
  for kind, input_name, output_name in FIXTURES:
    input_path = root / input_name
    output_path = root / output_name
    encoded = encode_path(kind, input_path)
    expected = output_path.read_bytes()
    if encoded == expected:
      print(f"ok   {output_name} ({len(encoded) // 33} frames)")
    else:
      failures += 1
      print(f"FAIL {output_name}: {len(encoded)} bytes encoded vs {len(expected)} golden")
  return failures


def write_fixtures(root: Path = FIXTURES_ROOT) -> None:
  for kind, input_name, output_name in FIXTURES:
    output_path = root / output_name
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_bytes(encode_path(kind, root / input_name))
    print(f"wrote {output_name}")


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--music", type=Path, help="music-source-v0 JSON to encode")
  parser.add_argument("--chipsynth-log", type=Path, help="chipsynth event log to encode")
  parser.add_argument("--output", type=Path, help="raw control-frame-v0 stream to write")
  parser.add_argument(
    "--verify-fixtures",
    action="store_true",
    help="re-encode checked-in fixtures and compare against golden outputs",
  )
  parser.add_argument(
    "--write-fixtures",
    action="store_true",
    help="(re)generate the golden outputs from the checked-in inputs",
  )
  return parser


def main() -> int:
  args = build_parser().parse_args()

  if args.write_fixtures:
    write_fixtures()
    return 0

  if args.verify_fixtures:
    failures = verify_fixtures()
    if failures:
      print(f"{failures} fixture(s) out of date", file=sys.stderr)
      return 1
    print("chipviz fixtures match golden outputs")
    return 0

  if args.music is not None:
    stream = encode_path("music", args.music)
  elif args.chipsynth_log is not None:
    stream = encode_path("chipsynth", args.chipsynth_log)
  else:
    raise SystemExit("choose --music, --chipsynth-log, --verify-fixtures, or --write-fixtures")

  if args.output is None or str(args.output) == "-":
    sys.stdout.buffer.write(stream)
  else:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(stream)
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
