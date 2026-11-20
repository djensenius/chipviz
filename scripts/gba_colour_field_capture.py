#!/usr/bin/env python3
"""Synthetic controller checks and captures of an unmodified production ROM.

Run against an already-built ROM and an mGBA libretro core. Every field pixel
is compared with the spec, and every checked transition frame must match a
complete expected image. No emulator memory is modified, and no experimental
flash is selected.
"""
import argparse
import ctypes
import pathlib
import struct

import nes_libretro_capture as nlc

nlc.BUTTONS.update({"l": 10, "r": 11})
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--rom", type=pathlib.Path, required=True)
parser.add_argument("--core", type=pathlib.Path, required=True)
parser.add_argument("--shots", type=pathlib.Path, required=True)
args = parser.parse_args()
SHOTS = args.shots
SHOTS.mkdir(parents=True, exist_ok=True)


class Core(nlc.NesCore):
  def _video(self, data, width, height, pitch):
    if data:
      self.raw = ctypes.string_at(data, pitch * height)
      self.width, self.height, self.pitch = width, height, pitch

  def pixels(self):
    assert (self.width, self.height, self.pixel_format) == (240, 160, 2)
    return b"".join(self.raw[y * self.pitch:y * self.pitch + 480] for y in range(160))


class Info(ctypes.Structure):
  _fields_ = [("name", ctypes.c_char_p), ("version", ctypes.c_char_p), ("extensions", ctypes.c_char_p),
              ("need_fullpath", ctypes.c_bool), ("block_extract", ctypes.c_bool)]


COMBOS = (
  (((0x105b, 0x5cc2), (0x033f, 0x5cc2), (0x105b, 0x033f), (0x5cc2, 0x7fff)),
   ((0x105b, 0x033f, 0x5cc2), (0x105b, 0x7fff, 0x5cc2),
    (0x033f, 0x0000, 0x5cc2), (0x105b, 0x0000, 0x033f))),
  (((0x001f, 0x7fe0), (0x03e0, 0x7c1f), (0x7c00, 0x03ff), (0x7fe0, 0x7c1f)),
   ((0x001f, 0x03e0, 0x7c00), (0x7fe0, 0x7c1f, 0x03ff),
    (0x001f, 0x0000, 0x7fe0), (0x7c00, 0x0000, 0x03ff))),
)


def bgr_to_565(value, level):
  r = ((value & 31) * level + 1) // 3
  g = (((value >> 5) & 31) * level + 1) // 3
  b = (((value >> 10) & 31) * level + 1) // 3
  # mGBA exports the five-bit GBA green channel in the upper five bits of
  # RGB565's six-bit green channel; it leaves the low bit clear.
  return (r << 11) | (g << 6) | b


def picture(family, layout, combo, rot, level):
  n = 3 if layout >= 2 else 2
  colours = COMBOS[family][n - 2][combo]
  pixels = bytearray()
  for y in range(160):
    for x in range(240):
      size, pos = (160, 159 - y) if layout & 1 else (240, x)
      region = sum(pos >= 8 * ((i * (size // 8)) // n) for i in range(1, n))
      pixels += struct.pack("<H", bgr_to_565(colours[(region + rot) % n], level))
  return bytes(pixels)


def save(path, pixels):
  rgb = bytearray()
  for (v,) in struct.iter_unpack("<H", pixels):
    r, g, b = (v >> 11) & 31, (v >> 5) & 63, v & 31
    rgb.extend(((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)))
  (SHOTS / path).write_bytes(nlc.Frame(240, 160, bytes(rgb)).png())


c = Core(args.rom, args.core)
info = Info()
c.lib.retro_get_system_info(ctypes.byref(info))
print("core:", info.name.decode(), info.version.decode(), flush=True)


def run(buttons=(), count=1):
  for _ in range(count):
    c.run_frame(buttons)


def transition(name, before, after, sequence):
  changed = []
  for i, buttons in enumerate(sequence):
    run(buttons)
    actual = c.pixels()
    if actual not in (before, after):
      first = next((j for j in range(0, len(actual), 2)
                    if actual[j:j + 2] not in (before[j:j + 2], after[j:j + 2])), None)
      for y in (0, 1, 79, 80, 159):
        samples = []
        for x in (0, 119, 120, 239):
          offset = (y * 240 + x) * 2
          samples.append(tuple(hex(struct.unpack_from("<H", pic, offset)[0])
                               for pic in (actual, before, after)))
        print("diagnostic y", y, samples, flush=True)
      raise AssertionError(f"{name}: incoherent frame {i}, unexpected pixel offset {first}")
    if actual == after and (not changed):
      changed.append(i)
  assert changed, f"{name}: no transition"
  assert c.pixels() == after, f"{name}: did not settle"
  return changed[0]


def tap(key, held=8, rest=8):
  run((key,), held)
  run((), rest)


run(count=120)
previous = c.pixels()
changes = []
for i in range(900):
  run()
  now = c.pixels()
  if now != previous:
    changes.append(i)
  previous = now
intervals = [b - a for a, b in zip(changes, changes[1:])]
print("native 900-vblank sample:", len(changes), "changes;",
      "intervals", min(intervals), max(intervals), flush=True)
for _ in range(4):
  tap("up")
run(count=70)
assert (c.width, c.height, c.pixel_format) == (240, 160, 2)
held_a = picture(0, 0, 0, 0, 3)
actual = c.pixels()
if actual != held_a:
  print("entry samples:", [hex(struct.unpack_from("<H", actual, (y * 240 + x) * 2)[0])
                           for x, y in ((0, 0), (119, 0), (120, 0), (239, 0))], flush=True)
  print("expected:", [hex(struct.unpack_from("<H", held_a, (y * 240 + x) * 2)[0])
                      for x, y in ((0, 0), (119, 0), (120, 0), (239, 0))], flush=True)
  raise AssertionError("scene 7 held V2 did not match every expected pixel")
save("synthetic-held-A-V2.png", actual)
print("scene ring: native 0 to scene 7, held A V2 every pixel OK", flush=True)

low = picture(0, 0, 0, 0, 1)
delay = transition("held to spectral", held_a, low, [("right",)] * 8 + [()] * 50)
save("synthetic-spectral-A-V2-low.png", c.pixels())
print("palette gate: held to spectral; transition after", delay, "vblanks", flush=True)

levels = [picture(0, 0, 0, 0, level) for level in (1, 2, 3)]
level_changes = []
last = c.pixels()
for i in range(45):
  run(("l",))
  current = c.pixels()
  assert current in levels, f"invalid spectral high frame {i}"
  if current != last:
    level_changes.append(i)
  last = current
assert c.pixels() == levels[2], "spectral high did not reach level 3"
assert len(level_changes) == 2 and level_changes[1] - level_changes[0] >= 10, level_changes
save("synthetic-spectral-A-V2-high.png", c.pixels())
high_changes = level_changes
level_changes = []
last = c.pixels()
for i in range(45):
  run()
  current = c.pixels()
  assert current in levels, f"invalid spectral low frame {i}"
  if current != last:
    level_changes.append(i)
  last = current
assert c.pixels() == levels[0] and len(level_changes) == 2 and level_changes[1] - level_changes[0] >= 10
print("spectral shoulder transitions:", high_changes, "release:", level_changes, flush=True)

delay = transition("spectral to hard cuts", low, held_a, [("right",)] * 8 + [()] * 50)
print("palette gate: spectral to hard cuts; transition after", delay, "vblanks", flush=True)
save("synthetic-hardcuts-A-V2.png", held_a)
layout = combo = rot = action = 0
current = held_a
delays = []


def cut(family, double=False):
  global layout, combo, rot, action, current
  n = 3 if layout >= 2 else 2
  if action == 1:
    combo = (combo + 1) & 3
  elif action == 3:
    layout = (layout + 1) & 3
    rot %= 3 if layout >= 2 else 2
  else:
    rot = (rot + 1) % n
  action = (action + 1) & 3
  next_picture = picture(family, layout, combo, rot, 3)
  sequence = [("a",)] * 8 + ([()] * 2 + [("a",)] * 8 if double else []) + [()] * 32
  delays.append(transition("hard cut", current, next_picture, sequence))
  current = next_picture


for target, name in ((1, "H2"), (2, "V3"), (3, "H3")):
  for index in range(4):
    cut(0, double=(target == 1 and index == 0))
  assert layout == target
  save(f"synthetic-hardcuts-A-{name}.png", current)
  print("A", name, "combo", combo, "rot", rot, "all pixels OK", flush=True)

before = current
after = picture(1, layout, combo, rot, 3)
family_sequence = []
for _ in range(4):
  family_sequence.extend([("right",)] * 8 + [()] * 8)
family_sequence += [()] * 50
delay = transition("family change", before, after, family_sequence)
current = after
action = 0
save("synthetic-hardcuts-B-H3.png", current)
print("family B H3 all pixels OK; gated change after", delay, "vblanks", flush=True)
for target, name in ((0, "V2"), (1, "H2"), (2, "V3")):
  for _ in range(4):
    cut(1)
  assert layout == target
  save(f"synthetic-hardcuts-B-{name}.png", current)
  print("B", name, "combo", combo, "rot", rot, "all pixels OK", flush=True)
print("24 hard-cut input-to-present delays:", delays, flush=True)
print("hard cuts: extra beat within CUT_MIN was dropped", flush=True)
departure = None
for i in range(100):
  run(("down",) if i < 8 else ())
  frame = c.pixels()
  if frame != current:
    departure = i
    break
assert departure is not None and 29 <= departure <= 31, f"scene exit debounce/presentation: {departure}"
native_colours = len(set(struct.iter_unpack("<H", c.pixels())))
assert native_colours > 16, f"native page has only {native_colours} colours"
print("scene ring: scene 7 to native after", departure, "vblanks;", native_colours, "native colours", flush=True)
print("atomicity: every checked frame equals the complete pre- or post-transition image", flush=True)
print("screenshots:", SHOTS, flush=True)
c.close()
