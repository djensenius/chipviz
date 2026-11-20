#!/usr/bin/env python3
"""Check short native overlay inputs through the production ROM in mGBA.

Runs headlessly with synthetic pad input and no memory injection. A two-frame
press starts after the first native redraw poll and is released before the
page is presented. Each overlay must appear on a complete native page and
disappear again. Requires an already-built ROM and an mGBA libretro core.
"""

import argparse
import ctypes
import pathlib

import nes_libretro_capture as libretro


class GbaCore(libretro.NesCore):
  def _video(self, data, width, height, pitch):
    if data:
      self.raw = ctypes.string_at(data, pitch * height)
      self.width, self.height, self.pitch = width, height, pitch

  def pixels(self):
    if (self.width, self.height, self.pixel_format) != (240, 160, libretro.PIXEL_RGB565):
      raise RuntimeError("expected mGBA 240x160 RGB565 output")
    return b"".join(self.raw[y * self.pitch:y * self.pitch + 480] for y in range(160))


def uniform_line(pixels, axis, coordinate):
  if axis == "row":
    line = pixels[coordinate * 480:(coordinate + 1) * 480]
    return line == line[:2] * 240
  samples = [pixels[(y * 240 + coordinate) * 2:(y * 240 + coordinate + 1) * 2] for y in range(160)]
  return all(sample == samples[0] for sample in samples)


def sample(rom, core, button=None, start=0, duration=2, count=80):
  gba = GbaCore(rom, core)
  try:
    for _ in range(120):
      gba.run_frame()
    frames = []
    for step in range(count):
      gba.run_frame((button,) if button and start <= step < start + duration else ())
      frames.append(gba.pixels())
    return frames
  finally:
    gba.close()


def native_sequence(rom, core, count=300):
  gba = GbaCore(rom, core)
  try:
    frames = []
    for _ in range(count * 10):
      gba.run_frame()
      pixels = gba.pixels()
      if not frames or pixels != frames[-1]:
        frames.append(pixels)
        # Ignore the initial black callback and mGBA's partially scanned boot
        # frame. Compare complete native pages only after startup.
        if len(frames) == count + 2:
          return frames[2:]
    raise AssertionError("native renderer did not produce enough distinct pages")
  finally:
    gba.close()


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--rom", type=pathlib.Path, required=True)
  parser.add_argument("--core", type=pathlib.Path, required=True)
  parser.add_argument("--reference-rom", type=pathlib.Path,
                      help="also compare 300 complete native pages with this prior ROM")
  args = parser.parse_args()
  if args.reference_rom:
    reference = native_sequence(args.reference_rom, args.core)
    actual = native_sequence(args.rom, args.core)
    if actual != reference:
      first = next(i for i, (a, b) in enumerate(zip(actual, reference)) if a != b)
      raise AssertionError(f"native page {first} differs from reference ROM")
    print("native production pages: 300 after startup match reference ROM pixel-for-pixel")
  baseline = sample(args.rom, args.core)
  changes = [i for i in range(1, len(baseline)) if baseline[i] != baseline[i - 1]]
  if len(changes) < 3:
    raise AssertionError("native renderer did not update")
  # A page flip completes one redraw. Two frames later is inside the next
  # redraw, not the poll on which that page's native state was first made.
  start = changes[1] + 2
  for button, axis, coordinate in (("a", "row", 12), ("b", "column", 12),
                                    ("start", "row", 78), ("select", "column", 118)):
    frames = sample(args.rom, args.core, button, start)
    if any(uniform_line(frame, axis, coordinate) for frame in baseline[start:]):
      raise AssertionError(f"{button}: baseline already has a solid overlay line")
    shown = [i for i in range(start, len(frames)) if uniform_line(frames[i], axis, coordinate)]
    if not shown:
      raise AssertionError(f"{button}: short mid-redraw press was lost")
    if shown[-1] >= len(frames) - 8:
      raise AssertionError(f"{button}: released overlay did not clear")
    print(f"{button}: short mid-redraw press shown at frames {shown[0]}..{shown[-1]}, then cleared")
  print("gba native production-loop overlay tests: 4 passed")


if __name__ == "__main__":
  main()
