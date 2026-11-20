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


def sample(rom, core, button=None, start=0, duration=2, count=80, schedule=None):
  gba = GbaCore(rom, core)
  try:
    for _ in range(120):
      gba.run_frame()
    frames = []
    for step in range(count):
      buttons = schedule(step) if schedule else ((button,) if button and start <= step < start + duration else ())
      gba.run_frame(buttons)
      frames.append(gba.pixels())
    return frames
  finally:
    gba.close()


def overlay_mask(pixels):
  # B is drawn after A, so omit B's two-pixel vertical stripes when checking
  # A's horizontal line. This distinguishes a real A+B chord from either key.
  y = 12
  samples = [pixels[(y * 240 + x) * 2:(y * 240 + x + 1) * 2]
             for x in range(240) if x % 12 >= 2]
  a = all(sample == samples[0] for sample in samples)
  b = uniform_line(pixels, "column", 12)
  return int(a) | (int(b) << 1)


def assert_overlay_sequence(frames, expected, label):
  changes = []
  for frame in frames:
    state = overlay_mask(frame)
    if not changes or changes[-1] != state:
      changes.append(state)
  while changes and changes[0] == 0:
    changes.pop(0)
  if changes != expected:
    raise AssertionError(f"{label}: expected {expected}, observed {changes}")
  print(f"{label}: observed sampled states {changes} in order")


def field_exit_sample(rom, core, schedule=None):
  gba = GbaCore(rom, core)
  try:
    for _ in range(120):
      gba.run_frame()
    for _ in range(4):
      for _ in range(8):
        gba.run_frame(("up",))
      for _ in range(8):
        gba.run_frame()
    for _ in range(70):
      gba.run_frame()
    field = gba.pixels()
    frames = []
    for step in range(100):
      buttons = ("down",) if step < 8 else ()
      gba.run_frame(buttons + (schedule(step) if schedule else ()))
      frames.append(gba.pixels())
    return field, frames
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


def native_timeline(rom, core, warmup=120, count=900):
  """Record displayed native-page changes, not merely generated host frames."""
  gba = GbaCore(rom, core)
  try:
    for _ in range(warmup):
      gba.run_frame()
    previous = gba.pixels()
    events = []
    for step in range(count):
      gba.run_frame()
      pixels = gba.pixels()
      if pixels != previous:
        events.append((step, pixels))
      previous = pixels
    return events
  finally:
    gba.close()


def check_cadence(rom, baseline_rom, core):
  baseline = native_timeline(baseline_rom, core)
  actual = native_timeline(rom, core)
  if abs(len(actual) - len(baseline)) > 1:
    raise AssertionError(f"native cadence: {len(actual)} changes vs baseline {len(baseline)} over 900 VBlanks")
  # Initial startup and mGBA frame boundaries can shift the first visible
  # page. Align by actual image, then compare the complete overlapping
  # sequence and relative presentation times, allowing one VBlank of jitter.
  matches = []
  for i in range(min(4, len(baseline))):
    for j in range(min(4, len(actual))):
      n = min(len(baseline) - i, len(actual) - j)
      if n < min(len(baseline), len(actual)) - 4:
        continue
      if all(baseline[i + k][1] == actual[j + k][1] for k in range(n)):
        base_start, actual_start = baseline[i][0], actual[j][0]
        drift = max(abs((baseline[i + k][0] - base_start) - (actual[j + k][0] - actual_start))
                    for k in range(n))
        matches.append((drift, n))
  if not matches:
    raise AssertionError("native cadence: no matching baseline page sequence after startup")
  drift, compared = min(matches, key=lambda match: (match[0], -match[1]))
  if drift > 1:
    raise AssertionError(f"native cadence: {drift} VBlanks of relative drift across {compared} pages")
  print(f"native cadence: {len(actual)} vs baseline {len(baseline)} changes over 900 VBlanks; "
        f"{compared} matching pages, max drift {drift}")


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--rom", type=pathlib.Path, required=True)
  parser.add_argument("--core", type=pathlib.Path, required=True)
  parser.add_argument("--reference-rom", type=pathlib.Path,
                      help="also compare 300 complete native pages with this prior ROM")
  parser.add_argument("--cadence-baseline", type=pathlib.Path,
                      help="compare 900 native VBlanks with the original baseline ROM")
  args = parser.parse_args()
  if args.cadence_baseline:
    check_cadence(args.rom, args.cadence_baseline, args.core)
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
  if any(overlay_mask(frame) for frame in baseline[start:]):
    raise AssertionError("baseline already resembles an A or B overlay")

  def ordered(step):
    offset = step - start
    return ("a",) if 0 <= offset < 2 else ("b",) if 4 <= offset < 6 else ()

  def simultaneous(step):
    return ("a", "b") if start <= step < start + 2 else ()

  def overlap(step):
    offset = step - start
    return ("a",) if 0 <= offset < 2 else ("a", "b") if offset < 4 and offset >= 2 else \
           ("b",) if 4 <= offset < 6 else ()

  assert_overlay_sequence(sample(args.rom, args.core, schedule=ordered), [1, 0, 2, 0],
                          "sequential A then B")
  assert_overlay_sequence(sample(args.rom, args.core, schedule=simultaneous), [3, 0],
                          "simultaneous A+B")
  assert_overlay_sequence(sample(args.rom, args.core, schedule=overlap), [1, 3, 2, 0],
                          "overlapping A then B")

  field, exit_baseline = field_exit_sample(args.rom, args.core)
  departure = next((i for i, frame in enumerate(exit_baseline) if frame != field), None)
  if departure is None or departure < 30:
    raise AssertionError(f"field exit did not debounce: {departure}")

  def exit_inputs(step):
    return ("a",) if departure - 2 <= step < departure else \
           ("b",) if departure + 1 <= step < departure + 3 else ()

  exit_field, exit_frames = field_exit_sample(args.rom, args.core, exit_inputs)
  if exit_field != field:
    raise AssertionError("field-exit setup is not deterministic")
  native_frames = [frame for frame in exit_frames if frame != field]
  assert_overlay_sequence(native_frames, [1, 0, 2, 0], "inputs during field-exit redraw")
  print("gba native production-loop overlay tests: 8 passed")


if __name__ == "__main__":
  main()
