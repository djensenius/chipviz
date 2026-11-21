#!/usr/bin/env python3
"""Compare a test-only C64 SpectrumVolume RAM trace with a synthetic oracle.

Requires a separately built fixture PRG, its VICE labels and the local VICE
libretro core. It does not test the production PRG, video, timing or safety.
"""
from __future__ import annotations

import argparse
import binascii
import os
import pathlib
import tempfile

from c64_colour_field_capture import CORE_EXT, CORE_OPTIONS, DEFAULT_CORES, MODELS, Emulator

ROOT = pathlib.Path(__file__).resolve().parents[1]
BUILD = ROOT / "build/tests/c64-spectrum-fixture"
STEPS = 112
RECORD_SIZE = 26
INKS = (6, 4, 5, 9, 2, 11, 8, 12)
SLICES = ((8, 2, 1), (15, 1, 1), (22, 0, 2))
THRESHOLDS = (0, 31, 32, 63, 64, 159, 160, 255)
DONE = b"SV02"
PROGRESS = b"SVP2"
DEFAULT_FRAMES = 16000
MAX_FRAMES = 16000
SAMPLE_FRAMES = 300


def input_at(step: int) -> tuple[bool, int, tuple[int, ...], int, int]:
  """Reset, initial palette, spectrum, energy and requested palette."""
  if not 0 <= step < STEPS:
    raise ValueError("step outside fixture")
  if step < 80:
    bands = [0] * 8
    if step % 10 < 5:
      bands[step // 10] = 255
    return step % 10 == 0, 0, tuple(bands), 255, 0
  if step < 90:
    return step == 80, 0, THRESHOLDS, 64 if step < 85 else 63, 0
  if step < 111:
    return False, 0, (255 if step < 106 else 0,) * 8, 255, 3
  return True, 255, (0,) * 8, 255, 255


def checksums(cells: bytes | bytearray) -> bytes:
  total = sum(cells) & 0xFFFF
  crc = binascii.crc_hqx(cells, 0xFFFF)
  return total.to_bytes(2, "little") + crc.to_bytes(2, "little")


def expected_trace() -> bytes:
  """Render from independent height/palette state, retaining inactive inks."""
  heights = [0] * 8
  palettes = [0] * 8
  cursor = 0
  screen = bytearray([32] * 1000)
  colour = bytearray(1000)
  trace = bytearray()
  for step in range(STEPS):
    reset, initial, bands, energy, palette = input_at(step)
    if reset:
      heights = [0] * 8
      palettes = [initial % 8] * 8
      cursor = 0
      screen = bytearray([32] * 1000)
      colour = bytearray(1000)
    palettes[cursor] = palette % 8
    for band, amplitude in enumerate(bands):
      target = min(5, (amplitude // 32) // (2 if energy < 64 else 1))
      heights[band] += (target > heights[band]) - (target < heights[band])
    cursor = (cursor + 1) % 8
    screen[:] = bytes([32]) * 1000
    for band, height in enumerate(heights):
      for depth, (base, shift, width) in enumerate(SLICES):
        ink = INKS[(palettes[band] + band + depth) % 8]
        for row in range(height):
          start = (base - row) * 40 + 4 + band * 4 + shift
          for x in range(width):
            screen[start + x] = 160
            colour[start + x] = ink
    trace.extend(heights)
    trace.extend(palettes)
    trace.append(cursor)
    trace.extend(checksums(screen))
    trace.extend(checksums(colour))
    trace.append(1)  # all four guards intact
  return bytes(trace)


def validate_trace(actual: bytes) -> None:
  expected = expected_trace()
  if len(actual) != len(expected):
    raise ValueError(f"trace length {len(actual)}, expected {len(expected)}")
  for offset, (got, want) in enumerate(zip(actual, expected)):
    if got != want:
      raise ValueError(f"trace step {offset // RECORD_SIZE}, byte {offset % RECORD_SIZE}: {got} != {want}")


def run_fixture(emu, trace: int, done: int, progress: int, frames: int) -> int:
  """Run for at most frames; timeout is always a failure, even with progress."""
  samples = []

  def sample(frame: int) -> None:
    if bytes(emu.peek(progress + i) for i in range(len(PROGRESS))) == PROGRESS:
      count = emu.peek(progress + len(PROGRESS))
      if count > STEPS:
        raise ValueError(f"invalid completed-record count {count}")
      samples.append((frame, count))
      del samples[:-2]

  for frame in range(1, frames + 1):
    emu.run(1)
    if bytes(emu.peek(done + i) for i in range(len(DONE))) == DONE:
      actual = bytes(emu.peek(trace + i) for i in range(STEPS * RECORD_SIZE))
      validate_trace(actual)
      return frame
    if frame % SAMPLE_FRAMES == 0:
      sample(frame)
  if frames % SAMPLE_FRAMES:
    sample(frames)
  if samples:
    count = samples[-1][1]
    actual = bytes(emu.peek(trace + i) for i in range(count * RECORD_SIZE))
    expected = expected_trace()[:count * RECORD_SIZE]
    if actual != expected:
      raise ValueError(f"completed trace prefix differs at timeout ({count} records)")
  raise RuntimeError(f"fixture did not finish in {frames} frames; progress samples (frame, records): {samples}")


def label(path: pathlib.Path, name: str) -> int:
  matches = []
  for line in path.read_text().splitlines():
    parts = line.split()
    if len(parts) == 3 and parts[0] == "al" and parts[2] == f"._{name}":
      matches.append(int(parts[1], 16))
  if len(matches) != 1:
    raise ValueError(f"expected exactly one label for {name}")
  return matches[0]


def main() -> int:
  parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
  parser.add_argument("--core", type=pathlib.Path,
                      default=pathlib.Path(os.environ.get("RETROARCH_CORES", DEFAULT_CORES)) /
                      f"vice_x64sc_libretro{CORE_EXT}")
  parser.add_argument("--prg", type=pathlib.Path, default=BUILD / "fixture.prg")
  parser.add_argument("--labels", type=pathlib.Path, default=BUILD / "fixture.lbl")
  parser.add_argument("--model", choices=sorted(MODELS), default="pal")
  parser.add_argument("--frames", type=int, default=DEFAULT_FRAMES,
                      help=f"maximum frames, including boot (1..{MAX_FRAMES})")
  args = parser.parse_args()
  if not 1 <= args.frames <= MAX_FRAMES:
    parser.error(f"--frames must be between 1 and {MAX_FRAMES}")
  trace = label(args.labels, "sv_fixture_trace")
  done = label(args.labels, "sv_fixture_done")
  progress = label(args.labels, "sv_fixture_progress")
  regions = sorted(((trace, STEPS * RECORD_SIZE), (done, len(DONE)), (progress, len(PROGRESS) + 1)))
  if any(start < 0x0801 or start + size > 0xA000 for start, size in regions) or any(
      start + size > next_start for (start, size), (next_start, _) in zip(regions, regions[1:])):
    raise ValueError("fixture symbols outside ordinary RAM or overlap")
  CORE_OPTIONS["vice_c64_model"] = MODELS[args.model]
  with tempfile.TemporaryDirectory() as workdir:
    emu = Emulator(args.core, pathlib.Path(workdir))
    try:
      emu.load(args.prg.resolve())
    except BaseException as error:
      try:
        emu.lib.retro_deinit()
      except BaseException as cleanup_error:
        error.add_note(f"retro_deinit also failed: {cleanup_error!r}")
      raise
    try:
      frame = run_fixture(emu, trace, done, progress, args.frames)
      print(f"c64 spectrum volume fixture: {STEPS} records matched after {frame} frames")
      return 0
    finally:
      try:
        emu.lib.retro_unload_game()
      finally:
        emu.lib.retro_deinit()


if __name__ == "__main__":
  raise SystemExit(main())
