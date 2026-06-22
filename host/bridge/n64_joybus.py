#!/usr/bin/env python3
"""Pack control-frame-v0 streams into N64 Joybus transport frames."""

from __future__ import annotations

import argparse
import sys
from dataclasses import dataclass
from pathlib import Path

from chipviz_bridge import MAGIC, VERSION, WIRE_SIZE, checksum


JOYBUS_SIZE = 16


@dataclass(frozen=True)
class DecodedFrame:
  frame: int
  beat_phase: int
  energy: int
  spectrum: tuple[int, ...]
  scene: int
  palette: int
  flags: int
  note_velocities: tuple[int, ...]


def decode_control_frame(wire: bytes) -> DecodedFrame:
  if len(wire) != WIRE_SIZE:
    raise ValueError(f"control frame must be {WIRE_SIZE} bytes")
  if wire[0] != MAGIC or wire[1] != VERSION:
    raise ValueError("bad control frame header")
  if checksum(wire[:-1]) != wire[-1]:
    raise ValueError("bad control frame checksum")
  note_count = wire[23]
  if note_count > 4:
    raise ValueError("bad control frame note count")
  velocities = tuple(wire[25 + index * 2] for index in range(note_count))
  return DecodedFrame(
    frame=int.from_bytes(wire[2:4], "little"),
    beat_phase=wire[5],
    energy=wire[8],
    spectrum=tuple(wire[12:20]),
    scene=wire[20],
    palette=wire[21],
    flags=wire[22],
    note_velocities=velocities,
  )


def pack_joybus_frame(frame: DecodedFrame) -> bytes:
  midi_intensity = max(frame.note_velocities, default=0)
  packet = bytearray(JOYBUS_SIZE)
  packet[0] = ((frame.flags & 0x01) << 7) | (1 if frame.spectrum[0] > 127 else 0) | (2 if frame.spectrum[1] > 127 else 0)
  packet[2] = frame.spectrum[0]
  packet[3] = frame.spectrum[1]
  packet[4] = (1 if frame.spectrum[2] > 127 else 0) | (2 if frame.spectrum[3] > 127 else 0)
  packet[6] = frame.spectrum[2]
  packet[7] = frame.spectrum[3]
  packet[8] = (1 if frame.spectrum[4] > 127 else 0) | (2 if frame.spectrum[5] > 127 else 0)
  packet[10] = frame.spectrum[4]
  packet[11] = frame.spectrum[5]
  packet[12] = (frame.flags & 0x0F) | ((frame.scene & 0x03) << 4) | ((frame.palette & 0x03) << 6)
  packet[13] = ((frame.scene & 0x0C) << 2) | (frame.palette & 0x0F)
  packet[14] = frame.energy
  packet[15] = midi_intensity
  return bytes(packet)


def convert_stream(stream: bytes) -> bytes:
  if len(stream) % WIRE_SIZE != 0:
    raise ValueError("control-frame stream is not aligned to 33-byte records")
  return b"".join(
    pack_joybus_frame(decode_control_frame(stream[offset:offset + WIRE_SIZE]))
    for offset in range(0, len(stream), WIRE_SIZE)
  )


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--input", type=Path, required=True, help="raw control-frame-v0 stream")
  parser.add_argument("--output", type=Path, help="16-byte Joybus packet stream; stdout if omitted")
  return parser


def main_from_args(argv: list[str] | None = None) -> int:
  args = build_parser().parse_args(argv)
  output = convert_stream(args.input.read_bytes())
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
