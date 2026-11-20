#!/usr/bin/env python3
"""Check native snapshots, field exits and cadence in the production ROM.

Runs headlessly with synthetic pad input and no memory injection. Requires an
already-built ROM and an mGBA libretro core; baseline comparisons require the
original ROM. Emulator callback boundaries can add one frame of phase jitter.
"""

import argparse
import ctypes
import pathlib

import nes_libretro_capture as libretro

libretro.BUTTONS.update({"l": 10, "r": 11})


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


def field_exit_sample(rom, core, schedule=None, spectral=False):
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
    if spectral:
      for _ in range(8):
        gba.run_frame(("right",))
      for _ in range(50):
        gba.run_frame()
    field = gba.pixels()
    frames = []
    for step in range(100):
      buttons = schedule(step) if schedule else (("down",) if step < 8 else ())
      gba.run_frame(buttons)
      frames.append(gba.pixels())
    return field, frames
  finally:
    gba.close()


def is_native(pixels):
  return len(set(zip(pixels[::2], pixels[1::2]))) > 16


def exit_index(frames):
  return next((i for i, frame in enumerate(frames) if is_native(frame)), None)


def assert_exit_timing(departure, start=0):
  # One frame accounts for the callback/key-sampling phase, not an extra
  # native redraw. The C state-machine test checks the exact step.
  if departure is None or not start + 29 <= departure <= start + 31:
    raise AssertionError(f"field exit expected at frames {start + 29}..{start + 31}, got {departure}")


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
  def ordered(step):
    return ("a",) if 8 <= step < 24 else ("b",) if 40 <= step < 56 else ()

  def simultaneous(step):
    return ("a", "b") if 8 <= step < 24 else ()

  def overlap(step):
    return ("a",) if 8 <= step < 24 else ("a", "b") if 24 <= step < 40 else \
           ("b",) if 40 <= step < 56 else ()

  for schedule, expected, label in ((ordered, [1, 0, 2, 0], "sequential A then B"),
                                    (simultaneous, [3, 0], "simultaneous A+B"),
                                    (overlap, [1, 3, 2, 0], "overlapping A then B")):
    assert_overlay_sequence(sample(args.rom, args.core, schedule=schedule), expected, label)
    if args.cadence_baseline:
      assert_overlay_sequence(sample(args.cadence_baseline, args.core, schedule=schedule),
                              expected, f"baseline {label}")

  if args.cadence_baseline:
    # Align to each ROM's own visible page flip. Short presses may be missed;
    # what matters is that the new ROM samples like the original, not that it
    # queues or combines states seen during the rest of the redraw.
    for button, axis, coordinate in (("a", "row", 12), ("b", "column", 12),
                                      ("start", "row", 78), ("select", "column", 118)):
      outcomes = []
      for rom in (args.cadence_baseline, args.rom):
        idle = sample(rom, args.core)
        changes = [i for i in range(1, len(idle)) if idle[i] != idle[i - 1]]
        if len(changes) < 3:
          raise AssertionError(f"{button}: native renderer did not update")
        if any(uniform_line(frame, axis, coordinate) for frame in idle):
          raise AssertionError(f"{button}: idle already has a solid overlay line")
        outcomes.append([any(uniform_line(frame, axis, coordinate)
                             for frame in sample(rom, args.core, button, changes[1] + offset))
                         for offset in (1, 2, 3, 4)])
      if outcomes[0] != outcomes[1]:
        raise AssertionError(f"{button}: short-press sampling differs from baseline: {outcomes}")
      print(f"{button}: short-press page snapshots match baseline {outcomes[0]}")

  field, exit_baseline = field_exit_sample(args.rom, args.core)
  departure = exit_index(exit_baseline)
  assert_exit_timing(departure)
  if any(frame != field for frame in exit_baseline[:departure]):
    raise AssertionError("held field changed before native presentation")

  def exit_inputs(step):
    return ("down", "a") if step < 8 else ("b",) if 20 <= step < 40 else ()

  exit_field, exit_frames = field_exit_sample(args.rom, args.core, exit_inputs)
  if exit_field != field:
    raise AssertionError("field-exit setup is not deterministic")
  assert_exit_timing(exit_index(exit_frames))
  assert_overlay_sequence(exit_frames[exit_index(exit_frames):], [2, 0],
                          "released early A is not replayed; page-start B and release")

  def exit_chord(step):
    return ("down",) if step < 8 else ("a", "b") if 20 <= step < 40 else ()

  _, chord_frames = field_exit_sample(args.rom, args.core, exit_chord)
  assert_exit_timing(exit_index(chord_frames))
  assert_overlay_sequence(chord_frames[exit_index(chord_frames):], [3, 0], "real exit chord")

  def palette_at(start):
    return lambda step: (("down",) if step < 8 else ()) + (("right",) if start <= step < start + 3 else ())

  _, early_palette = field_exit_sample(args.rom, args.core, palette_at(10))
  _, late_palette = field_exit_sample(args.rom, args.core, palette_at(22))
  for frames in (early_palette, late_palette):
    assert_exit_timing(exit_index(frames))
  if early_palette[exit_index(early_palette)] != late_palette[exit_index(late_palette)]:
    raise AssertionError("palette changes before page start did not produce the same first native page")

  _, post_snapshot_palette = field_exit_sample(args.rom, args.core, palette_at(29))
  assert_exit_timing(exit_index(post_snapshot_palette))
  if exit_baseline[departure] != post_snapshot_palette[exit_index(post_snapshot_palette)]:
    raise AssertionError("palette change after page start altered the already sampled page")

  def cancelled(step):
    return ("down",) if step < 8 or 30 <= step < 33 else ("up",) if 25 <= step < 28 else ()

  _, cancelled_frames = field_exit_sample(args.rom, args.core, cancelled)
  assert_exit_timing(exit_index(cancelled_frames), 30)

  def energy_during_prepare(step):
    return (("down",) if step < 8 else ()) + (("l",) if 10 <= step < 28 else ())

  spectral_field, spectral_frames = field_exit_sample(args.rom, args.core, energy_during_prepare, spectral=True)
  spectral_exit = exit_index(spectral_frames)
  assert_exit_timing(spectral_exit)
  if all(frame == spectral_field for frame in spectral_frames[:spectral_exit]):
    raise AssertionError("field palette did not update during native preparation")
  print(f"field exit: next-step presentation at frame {departure}; cancellation and live field updates OK")
  print("gba native production-loop snapshot/control checks passed")


if __name__ == "__main__":
  main()
