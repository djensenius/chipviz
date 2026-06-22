#!/usr/bin/env python3
"""Map control-frame-v0 records to reduced USB HID states."""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

from n64_joybus import decode_control_frame
from chipviz_bridge import WIRE_SIZE

PALETTE_KEYS = ("Q", "W", "E", "R", "T", "Y", "U", "I")


@dataclass(frozen=True)
class HidState:
  buttons: int
  left_x: int
  left_y: int
  keys: tuple[str, ...]


def signed_bucket(value: int) -> int:
  return max(-127, min(127, value - 128))


def map_frame(wire: bytes) -> HidState:
  frame = decode_control_frame(wire)
  buttons = 0
  if frame.flags & 0x01:
    buttons |= 1 << 0
  if frame.flags & 0x02:
    buttons |= 1 << 1
  if frame.flags & 0x0C:
    buttons |= 1 << 2
  if frame.flags & 0x10:
    buttons |= 1 << 3
  if frame.energy > 170:
    buttons |= 1 << 4
  if frame.spectrum[7] > 170:
    buttons |= 1 << 5

  keys = []
  keys.append(str((frame.scene & 0x07) + 1))
  keys.append(PALETTE_KEYS[frame.palette & 0x07])
  if frame.flags & 0x01:
    keys.append("Space")
  if frame.flags & 0x02:
    keys.append("Return")
  if frame.flags & 0x0C:
    keys.append("F")
  if frame.flags & 0x10:
    keys.append("M")

  return HidState(
    buttons=buttons,
    left_x=signed_bucket(frame.energy),
    left_y=signed_bucket(frame.beat_phase),
    keys=tuple(keys),
  )


def convert_stream(stream: bytes) -> list[HidState]:
  if len(stream) % WIRE_SIZE != 0:
    raise ValueError("control-frame stream is not aligned to 33-byte records")
  return [
    map_frame(stream[offset:offset + WIRE_SIZE])
    for offset in range(0, len(stream), WIRE_SIZE)
  ]


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--input", type=Path, required=True, help="raw control-frame-v0 stream")
  parser.add_argument("--output", type=Path, help="JSON HID state stream; stdout if omitted")
  return parser


def main_from_args(argv: list[str] | None = None) -> int:
  args = build_parser().parse_args(argv)
  states = [asdict(state) for state in convert_stream(args.input.read_bytes())]
  output = json.dumps(states, indent=2).encode("utf-8") + b"\n"
  if args.output is None or str(args.output) == "-":
    sys.stdout.buffer.write(output)
  else:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(output)
  return 0


def main() -> int:
  return main_from_args()


if __name__ == "__main__":
  raise SystemExit(main())
