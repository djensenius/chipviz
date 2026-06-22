#!/usr/bin/env python3
"""Minimal Raspberry Pi/development host bridge for frame files and UDP output."""

from __future__ import annotations

import argparse
import socket
import time
from pathlib import Path

from chipviz_bridge import WIRE_SIZE, make_procedural_frame, pack_frame, parse_udp_endpoint
from chipsynth_stream import encode_event_log


def frame_records(stream: bytes) -> list[bytes]:
  if len(stream) % WIRE_SIZE != 0:
    raise ValueError("control-frame stream is not aligned to 33-byte records")
  return [stream[offset:offset + WIRE_SIZE] for offset in range(0, len(stream), WIRE_SIZE)]


def build_frames(args: argparse.Namespace) -> list[bytes]:
  if args.input:
    return frame_records(args.input.read_bytes())
  if args.chipsynth_log:
    return frame_records(encode_event_log(args.chipsynth_log.read_bytes()))
  return [pack_frame(make_procedural_frame(index)) for index in range(args.frames)]


def send_udp(frames: list[bytes], endpoint: tuple[str, int], rate: int) -> None:
  interval = 1.0 / rate
  with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp_socket:
    for frame in frames:
      udp_socket.sendto(frame, endpoint)
      time.sleep(interval)


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--input", type=Path, help="raw control-frame-v0 stream")
  parser.add_argument("--chipsynth-log", type=Path, help="chipsynth CSV0 event log")
  parser.add_argument("--frames", type=int, default=120, help="procedural frames if no input is supplied")
  parser.add_argument("--rate", type=int, default=60, help="output frame rate")
  parser.add_argument("--udp", type=parse_udp_endpoint, help="send frames to HOST:PORT over UDP")
  parser.add_argument("--output", type=Path, help="write normalized control-frame-v0 stream")
  return parser


def main() -> int:
  args = build_parser().parse_args()
  if args.frames < 1:
    raise SystemExit("--frames must be at least 1")
  if args.rate < 1:
    raise SystemExit("--rate must be at least 1")

  frames = build_frames(args)
  if args.output:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(b"".join(frames))
  if args.udp:
    send_udp(frames, args.udp, args.rate)
  if not args.output and not args.udp:
    raise SystemExit("choose --output, --udp, or both")
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
