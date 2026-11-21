#!/usr/bin/env python3
"""Produce an offline scene-catalog-v1 recording from music-source-v0 JSON."""

from __future__ import annotations

import argparse
import json
import os
import stat
import struct
import sys
import tempfile
import zlib
from pathlib import Path

import chipviz_encode
import scene_catalog_v1


MAX_SOURCE_BYTES = 1_048_576
MAX_FRAME_NOTE_WORK = 1_000_000
MAX_FRAMES = 65_536
FRAME_SIZE = 33


def _selection(family: int, variant: int) -> None:
  if (isinstance(family, bool) or not isinstance(family, int)
      or isinstance(variant, bool) or not isinstance(variant, int)
      or not 0 <= family <= 5 or variant != 0):
    raise ValueError("supported catalog selection is family 0..5, variant 0")


def _reject_constant(value: str) -> None:
  raise ValueError(f"invalid JSON constant: {value}")


def encode_catalog(source_bytes: bytes, family: int, variant: int = 0) -> bytes:
  """Encode and independently validate a bounded source, returning complete bytes."""
  _selection(family, variant)
  if len(source_bytes) > MAX_SOURCE_BYTES:
    raise ValueError(f"source exceeds {MAX_SOURCE_BYTES} bytes")
  try:
    source = json.loads(source_bytes.decode("utf-8"), parse_constant=_reject_constant)
  except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
    raise ValueError(f"invalid JSON source: {error}") from error
  if not isinstance(source, dict):
    raise ValueError("music source must be a JSON object")
  rate = source.get("frame_rate")
  if isinstance(rate, bool) or not isinstance(rate, int) or rate != 60:
    raise ValueError("music source frame_rate must be exactly 60")
  count = source.get("frame_count")
  if isinstance(count, bool) or not isinstance(count, int) or not 1 <= count <= MAX_FRAMES:
    raise ValueError("music source frame_count must be 1..65536")
  notes = source.get("notes", [])
  if not isinstance(notes, list):
    raise ValueError("music source notes must be a list")
  if count * len(notes) > MAX_FRAME_NOTE_WORK:
    raise ValueError(f"frame_count * note_count exceeds {MAX_FRAME_NOTE_WORK}")

  frames = chipviz_encode.encode_music_source(source)
  header = b"CVXE\x01\x01\x00\x00" + struct.pack("<Q", count)
  output = bytearray(header + struct.pack("<I", zlib.crc32(header)))
  for index in range(count):
    frame = bytearray(frames[index * FRAME_SIZE:(index + 1) * FRAME_SIZE])
    frame[32] ^= frame[20] ^ family
    frame[20] = family
    record = (b"CVX1\x01\x01" + bytes((variant, 0))
              + struct.pack("<QQ", index, index) + frame + bytes(84))
    output.extend(record + struct.pack("<I", zlib.crc32(record)))

  # The validator accepts only regular descriptors and validates the whole file.
  with tempfile.TemporaryFile(mode="w+b") as candidate:
    candidate.write(output)
    candidate.flush()
    with scene_catalog_v1.open_catalog(candidate.fileno()):
      pass
  return bytes(output)


def write_catalog(source_path: Path, output_path: Path, family: int, variant: int = 0) -> None:
  """Read a regular source and exclusively create an output after validation."""
  _selection(family, variant)
  fd = os.open(source_path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOCTTY)
  try:
    if not stat.S_ISREG(os.fstat(fd).st_mode):
      raise ValueError("source must be a regular file")
    with os.fdopen(fd, "rb") as source:
      fd = -1
      data = source.read(MAX_SOURCE_BYTES + 1)
  finally:
    if fd != -1:
      os.close(fd)
  output = encode_catalog(data, family, variant)
  fd = os.open(output_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
  try:
    with os.fdopen(fd, "wb") as destination:
      fd = -1
      destination.write(output)
  finally:
    if fd != -1:
      os.close(fd)


def main() -> int:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--music", required=True, type=Path, help="music-source-v0 JSON")
  parser.add_argument("--family", required=True, type=int, help="catalog family 0..5")
  parser.add_argument("--variant", type=int, default=0, help="catalog variant (only 0)")
  parser.add_argument("--output", required=True, type=Path, help="new .cvxe file")
  args = parser.parse_args()
  try:
    write_catalog(args.music, args.output, args.family, args.variant)
  except (ValueError, OSError) as error:
    print(f"catalog music source: {error}", file=sys.stderr)
    return 1
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
