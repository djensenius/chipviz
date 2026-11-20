#!/usr/bin/env python3
"""Compare production mGBA transition frames with full native-page pixels.

Build the host-only oracle first:
  cc -std=c99 -O2 -shared -fPIC -Icores/gba/homebrew/source \
    tests/gba_native_pixel_oracle.c cores/gba/homebrew/source/colour_field.c \
    cores/gba/homebrew/source/native_prefetch.c -o /tmp/gba-native-oracle.dylib

The oracle uses the production renderer, independently of mGBA's display
buffer. No emulator memory is modified, and all controller inputs are synthetic.
"""
import argparse
import ctypes
import pathlib
import struct

import gba_native_controls_test as controls


FIELD_LEFT = (0x105b & 31) << 11 | ((0x105b >> 5) & 31) << 6 | (0x105b >> 10)
FIELD_RIGHT = (0x5cc2 & 31) << 11 | ((0x5cc2 >> 5) & 31) << 6 | (0x5cc2 >> 10)
FIELD = (struct.pack("<H", FIELD_LEFT) * 120 + struct.pack("<H", FIELD_RIGHT) * 120) * 160
KEYS = {"a": 1, "b": 2, "select": 4, "start": 8}


class Oracle:
  def __init__(self, path):
    self.lib = ctypes.CDLL(str(path))
    self.lib.gba_test_native_pixels.argtypes = [ctypes.c_uint16, ctypes.c_uint8, ctypes.c_uint8,
                                                ctypes.c_uint16, ctypes.POINTER(ctypes.c_uint16)]
    self.pixels = (ctypes.c_uint16 * (240 * 160))()
    self.cache = {}

  def page(self, frame, scene, palette, keys):
    key = frame, scene, palette, keys
    if key not in self.cache:
      self.lib.gba_test_native_pixels(*key, self.pixels)
      self.cache[key] = bytes(self.pixels)
    return self.cache[key]

  def locate(self, pixels, scene, palette=0, keys=0):
    matches = [frame for frame in range(150) if pixels == self.page(frame, scene, palette, keys)]
    if len(matches) != 1:
      raise AssertionError(f"expected one complete native page, got frame matches {matches}")
    return matches[0]


def setup(rom, core, oracle):
  gba = controls.GbaCore(rom, core)
  for _ in range(120):
    gba.run_frame()
  for _ in range(3):
    for _ in range(8):
      gba.run_frame(("up",))
    for _ in range(8):
      gba.run_frame()
  last = gba.pixels()
  native_frames = set()
  entry = None
  for step in range(60):
    gba.run_frame(("up",) if step < 8 else ())
    pixels = gba.pixels()
    if pixels == FIELD:
      entry = step
      break
    native_frames.add(pixels)
    last = pixels
  if entry is None or not 29 <= entry <= 31:
    raise AssertionError(f"field entry expected at frames 29..31, got {entry}")
  for pixels in native_frames:
    oracle.locate(pixels, 3)
  frame = oracle.locate(last, 3) + 1
  for _ in range(40):
    gba.run_frame()
    if gba.pixels() != FIELD:
      raise AssertionError("field did not remain complete after entry")
  return gba, frame, entry


def scenario(rom, core, oracle, name, schedule, start, first, second, count=100):
  gba, frame, entry = setup(rom, core, oracle)
  try:
    frames = []
    for step in range(count):
      gba.run_frame(schedule(step))
      frames.append(gba.pixels())
    departure = controls.exit_index(frames)
    controls.assert_exit_timing(departure, start)
    if any(pixels != FIELD for pixels in frames[:departure]):
      raise AssertionError(f"{name}: partial or unexpected field before exit")
    first_page = oracle.page(frame, *first)
    if frames[departure] != first_page:
      raise AssertionError(f"{name}: first native page differs from complete rendered page")
    next_page = next((pixels for pixels in frames[departure + 1:] if pixels != first_page), None)
    if next_page != oracle.page(frame + 1, *second):
      raise AssertionError(f"{name}: next native page differs from complete rendered page")
    print(f"{name}: field entry {entry}, exit {departure}; first two native pages exact")
  finally:
    gba.close()


def repeated_entries(rom, core, oracle):
  gba, frame, entry = setup(rom, core, oracle)
  try:
    transitions = []
    native_pages = set()
    was_field = True
    for step in range(175):
      if any(start <= step < start + 5 for start in (0, 95)):
        buttons = ("down",)
      elif any(start <= step < start + 5 for start in (30, 125)):
        buttons = ("up",)
      else:
        buttons = ()
      gba.run_frame(buttons)
      pixels = gba.pixels()
      is_field = pixels == FIELD
      if not is_field:
        native_pages.add(pixels)
      if is_field != was_field:
        transitions.append((step, is_field))
        was_field = is_field
    if len(transitions) != 4:
      raise AssertionError(f"repeated entry/exit transitions: {transitions}")
    for (step, is_field), target, expected_field in zip(transitions, (30, 60, 125, 155),
                                                         (False, True, False, True)):
      controls.assert_exit_timing(step, target - 30)
      if is_field != expected_field:
        raise AssertionError(f"repeated entry/exit order: {transitions}")
    if oracle.page(frame, 3, 0, 0) not in native_pages:
      raise AssertionError("first post-field native page was not presented")
    for pixels in native_pages:
      oracle.locate(pixels, 3)
    print(f"repeated entry/exit: initial entry {entry}, transitions {transitions}; all pages complete")
  finally:
    gba.close()


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--rom", type=pathlib.Path, required=True)
  parser.add_argument("--core", type=pathlib.Path, required=True)
  parser.add_argument("--oracle", type=pathlib.Path, required=True)
  args = parser.parse_args()
  oracle = Oracle(args.oracle)

  def plain(step):
    return ("down",) if step < 8 else ()

  def early_a(step):
    return ("down", "a") if step < 8 else ()

  def snapshot_a(step):
    return ("down",) if step < 8 else ("a",) if 23 <= step < 29 else ()

  def full_chord(step):
    return ("down",) if step < 8 else ("a", "b", "start", "select") if 20 <= step < 40 else ()

  def palette_before(step):
    return ("down",) if step < 8 else ("right",) if 24 <= step < 29 else ()

  def palette_after(step):
    return ("down",) if step < 8 else ("right",) if 29 <= step < 35 else ()

  def scene_on_display(step):
    return ("down",) if step < 8 or 30 <= step < 36 else ()

  def cancel_at_30(step):
    return ("down",) if step < 8 or 29 <= step < 35 else ()

  def reversals(step):
    if any(start <= step < start + 3 for start in (0, 40, 80, 120)):
      return ("down",)
    if any(start <= step < start + 3 for start in (20, 60, 100)):
      return ("up",)
    return ()

  def alternate_scene(count):
    last = (count - 1) * 10
    def buttons(step):
      result = ("down",) if any(start <= step < start + 3 for start in range(0, last + 1, 10)) else ()
      if last + 3 <= step < last + 6:
        result += ("left",)
      if last + 20 <= step < last + 40:
        result += ("a", "b", "start", "select")
      return result
    return buttons

  scenario(args.rom, args.core, oracle, "plain", plain, 0, (3, 0, 0), (3, 0, 0))
  scenario(args.rom, args.core, oracle, "released A", early_a, 0, (3, 0, 0), (3, 0, 0))
  scenario(args.rom, args.core, oracle, "snapshotted A", snapshot_a, 0, (3, 0, KEYS["a"]), (3, 0, 0))
  scenario(args.rom, args.core, oracle, "full chord", full_chord, 0, (3, 0, 15), (3, 0, 15))
  scenario(args.rom, args.core, oracle, "late palette", palette_before, 0, (3, 1, 0), (3, 1, 0))
  scenario(args.rom, args.core, oracle, "post-snapshot palette", palette_after, 0, (3, 0, 0), (3, 1, 0))
  scenario(args.rom, args.core, oracle, "scene on display poll", scene_on_display, 0, (3, 0, 0), (2, 0, 0))
  scenario(args.rom, args.core, oracle, "scene change on 30th poll", cancel_at_30, 29, (2, 0, 0), (2, 0, 0))
  scenario(args.rom, args.core, oracle, "three reversals", reversals, 120, (3, 0, 0), (3, 0, 0), 180)
  for count, scene in ((2, 2), (3, 1), (4, 0)):
    scenario(args.rom, args.core, oracle, f"scene {scene} palette 7 full chord", alternate_scene(count),
             (count - 1) * 10, (scene, 7, 15), (scene, 7, 15))
  repeated_entries(args.rom, args.core, oracle)
  print("gba mGBA entry/exit integrity: 13 scenarios passed")


if __name__ == "__main__":
  main()
