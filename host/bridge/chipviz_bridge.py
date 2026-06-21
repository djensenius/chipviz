#!/usr/bin/env python3
"""Emit chipviz control-frame-v0 packets for files, stdout, or UDP."""

from __future__ import annotations

import argparse
import socket
import sys
from dataclasses import dataclass
from pathlib import Path


MAGIC = 0xC7
VERSION = 0
SPECTRUM_BANDS = 8
MAX_NOTES = 4
WIRE_SIZE = 33


@dataclass(frozen=True)
class Note:
  note: int
  velocity: int


@dataclass(frozen=True)
class ControlFrame:
  frame: int
  bpm: int
  beat_phase: int
  beat_count: int
  energy: int
  bass: int
  mid: int
  treble: int
  spectrum: tuple[int, ...]
  scene: int
  palette: int
  flags: int
  notes: tuple[Note, ...]


def checksum(payload: bytes | bytearray) -> int:
  value = 0
  for byte in payload:
    value ^= byte
  return value


def clamp_byte(value: int) -> int:
  return max(0, min(255, value))


def make_procedural_frame(frame_index: int) -> ControlFrame:
  phase = (frame_index * 17) & 0xFF
  beat_count = frame_index // 16
  flags = 0x01 if phase < 17 else 0x00
  energy = clamp_byte(80 + phase // 2)
  notes = (Note(48 + (beat_count % 24), energy),) if flags else ()

  return ControlFrame(
    frame=frame_index & 0xFFFF,
    bpm=120,
    beat_phase=phase,
    beat_count=beat_count & 0xFFFF,
    energy=energy,
    bass=255 - phase,
    mid=clamp_byte(64 + ((frame_index * 9) & 0x7F)),
    treble=(frame_index * 23) & 0xFF,
    spectrum=tuple((phase + band * 29) & 0xFF for band in range(SPECTRUM_BANDS)),
    scene=(frame_index // 96) % 3,
    palette=(frame_index // 32) % 8,
    flags=flags,
    notes=notes,
  )


def pack_frame(frame: ControlFrame) -> bytes:
  if len(frame.spectrum) != SPECTRUM_BANDS:
    raise ValueError(f"spectrum must contain {SPECTRUM_BANDS} bands")
  if len(frame.notes) > MAX_NOTES:
    raise ValueError(f"at most {MAX_NOTES} notes fit in one frame")
  if frame.flags & 0xE0:
    raise ValueError("reserved flag bits must be clear for v0")

  wire = bytearray(WIRE_SIZE)
  wire[0] = MAGIC
  wire[1] = VERSION
  wire[2:4] = frame.frame.to_bytes(2, "little")
  wire[4] = frame.bpm
  wire[5] = frame.beat_phase
  wire[6:8] = frame.beat_count.to_bytes(2, "little")
  wire[8] = frame.energy
  wire[9] = frame.bass
  wire[10] = frame.mid
  wire[11] = frame.treble
  wire[12:20] = bytes(frame.spectrum)
  wire[20] = frame.scene
  wire[21] = frame.palette
  wire[22] = frame.flags
  wire[23] = len(frame.notes)

  for index, note in enumerate(frame.notes):
    wire[24 + index * 2] = note.note
    wire[25 + index * 2] = note.velocity

  wire[32] = checksum(wire[:-1])
  return bytes(wire)


def parse_udp_endpoint(endpoint: str) -> tuple[str, int]:
  host, separator, port_text = endpoint.rpartition(":")
  if not separator or not host or not port_text:
    raise argparse.ArgumentTypeError("UDP endpoint must be HOST:PORT")

  try:
    port = int(port_text)
  except ValueError as error:
    raise argparse.ArgumentTypeError("UDP port must be an integer") from error

  if port < 1 or port > 65535:
    raise argparse.ArgumentTypeError("UDP port must be 1..65535")

  return host, port


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--frames", type=int, default=120, help="frames to emit")
  parser.add_argument(
    "--output",
    type=Path,
    help="raw frame file to write; use '-' for stdout",
  )
  parser.add_argument(
    "--udp",
    type=parse_udp_endpoint,
    help="also send each frame to HOST:PORT over UDP",
  )
  return parser


def main() -> int:
  args = build_parser().parse_args()
  if args.frames < 1:
    raise SystemExit("--frames must be at least 1")
  if args.output is None and args.udp is None:
    raise SystemExit("choose --output, --udp, or both")

  packets = [pack_frame(make_procedural_frame(index)) for index in range(args.frames)]

  if args.output is not None:
    if str(args.output) == "-":
      sys.stdout.buffer.write(b"".join(packets))
    else:
      args.output.parent.mkdir(parents=True, exist_ok=True)
      args.output.write_bytes(b"".join(packets))

  if args.udp is not None:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp_socket:
      for packet in packets:
        udp_socket.sendto(packet, args.udp)

  return 0


if __name__ == "__main__":
  raise SystemExit(main())
