#!/usr/bin/env python3
"""Compare the isolated mGBA Lines fixture's video transport with a host oracle."""

import argparse
import pathlib
import struct
import sys

ROOT = pathlib.Path(__file__).resolve().parents[5]
sys.path.insert(0, str(ROOT / "scripts"))
import gba_native_controls_test as controls

WIDTH, HEIGHT = 240, 160
PAGE = WIDTH * HEIGHT
HEADER = 32
PAYLOAD = PAGE - HEADER
EVENTS = 13
REGULAR = (0, 480, 511, 543, 960, 1023)
STALLED = (0, 512, 543, 575, 1024, 1087)


def keys_at(scenario, step):
  return ((3 if not scenario and step and step % 30 == 0 else 0) |
          (0x200 if (not scenario and 200 <= step < 700) or
           (scenario and 520 <= step < 900) else 0))


class ScalarState:
  """One-sample-at-a-time reference, independent of the C implementation."""
  def __init__(self, keys):
    self.phase = self.fold = self.target = self.beat_age = self.section_age = self.phrase = 0
    self.energy, self.previous = 16, keys

  def advance(self, keys):
    self.phase = (self.phase + 1) % 48
    self.section_age = min(self.section_age + 1, 192)
    self.beat_age = min(self.beat_age + 1, 512)
    boundary = False
    if keys & 1 and not self.previous & 1:
      self.beat_age = 0
      self.phrase += 1
      if self.phrase == 16:
        self.phrase, boundary = 0, True
    if self.section_age == 192 and (boundary or self.beat_age == 512):
      self.target = 64 - self.target
      self.beat_age = self.section_age = self.phrase = 0
    self.fold += (self.fold < self.target) - (self.fold > self.target)
    energy_target = 32 if keys & 0x200 else 16
    self.energy += (self.energy < energy_target) - (self.energy > energy_target)
    self.previous = keys

  def palette(self):
    energy = self.energy
    colors = [0x0801, (energy // 4) | ((energy // 2) << 5) | ((energy * 3 // 4) << 10),
              (energy // 2) | ((energy * 3 // 4) << 5) | ((energy * 3 // 4) << 10)]
    return struct.pack("<256H", *(colors + [0] * 253))

  def pixels(self):
    pixels = bytearray(PAGE)
    for y in range(HEIGHT):
      dy = abs(2 * y - 159)
      for x in range(WIDTH):
        dx = abs(2 * x - 239)
        radial = dx * 2 // 3 if dx > dy * 3 // 2 else dy
        depth = (dy * (64 - self.fold) + radial * self.fold) // 64
        pixels[y * WIDTH + x] = (2 if (depth + self.phase // 2) % 24 < 2 else
                                 1 if dx % (12 + depth // 8) < 2 else 0)
    return bytes(pixels)


def oracle_events():
  """Independent scalar state and pixel oracle for the fixed fixture script."""
  events = []
  for scenario, steps in enumerate((REGULAR, STALLED)):
    state, last = ScalarState(keys_at(scenario, 0)), 0
    for item, step in enumerate(steps):
      for t in range(last + 1, step + 1):
        state.advance(keys_at(scenario, t))
      last = step
      palette_bytes, pixels = state.palette(), state.pixels()
      cancellation = scenario == 0 and item == 2
      for partial in ((True, False) if cancellation else (False,)):
        progress = 185 if partial else PAGE // 2
        batches = 5 if partial else (80 if item & 1 else 519)
        flags = 7 if cancellation else 5
        header = struct.pack("<4sBBBBIHBBBBHHHHHBBH", b"GLFX", 1, len(events), 0, scenario,
                             step, state.phase, state.fold, state.energy, state.target, state.phrase,
                             state.beat_age, state.section_age,
                             progress, 0x5AA5, 0xA55A, flags, int(not partial), batches)
        page = pixels[:370] + b"\xa5" * (PAGE - 370) if partial else pixels
        events.append((header, page, palette_bytes))
  assert len(events) == EVENTS
  return events


def decode(raw):
  if len(raw) % 2:
    raise AssertionError("odd RGB565 frame length")
  result = bytearray(len(raw) // 2)
  for i, (value,) in enumerate(struct.iter_unpack("<H", raw)):
    index = ((value >> 11) & 31) | (((value >> 6) & 7) << 5)
    if value != ((index & 31) << 11) | ((index >> 5) << 6):
      raise AssertionError("unexpected transport palette/pixel format")
    result[i] = index
  return bytes(result)


class Collector:
  def __init__(self):
    self.expected = oracle_events()
    self.index = 0
    self.previous = None
    self.last_key = self.last_frame = None
    self.done = False

  def feed(self, frame):
    if frame is None:
      self.previous = None
      return
    if len(frame) != PAGE * 2:
      raise AssertionError("expected 240x160 RGB565 frame")
    try:
      header = decode(frame[:HEADER * 2])
    except AssertionError:
      self.previous = None
      return
    if header[:5] != b"GLFX\x01":
      self.previous = None
      return
    if self.previous != frame:
      self.previous = frame
      return
    key = header[5], header[6]
    if key == self.last_key:
      if frame != self.last_frame:
        raise AssertionError(f"corrupt repeated record: {key}")
      return
    event, chunk = divmod(self.index, 3)
    if event == EVENTS:
      expected = b"GLFX\x01" + bytes((EVENTS, 3)) + bytes(25)
      if header != expected:
        raise AssertionError("missing or malformed completion record")
      self.done = True
      self.last_key, self.last_frame = key, frame
      return
    if key != (event, chunk):
      raise AssertionError(f"missing or out-of-order record: expected {(event, chunk)}, got {key}")
    expected_header, page, palette = self.expected[event]
    expected_header = expected_header[:6] + bytes((chunk,)) + expected_header[7:]
    if header != expected_header:
      raise AssertionError(f"trace mismatch at event {event} chunk {chunk}")
    body = decode(frame[HEADER * 2:])
    expected_body = ((page[:PAYLOAD] if chunk == 0 else page[PAYLOAD:]) if chunk < 2 else palette)
    expected_body = expected_body.ljust(PAYLOAD, b"\0")
    if body != expected_body:
      offset = next(i for i, (a, b) in enumerate(zip(body, expected_body)) if a != b)
      raise AssertionError(f"page/palette mismatch at event {event} chunk {chunk} byte {offset}")
    self.index += 1
    self.last_key, self.last_frame = key, frame


class FixtureCore(controls.GbaCore):
  def run_frame(self, buttons=()):
    self.video_fresh = False
    self.video_null = False
    super().run_frame(buttons)

  def _video(self, data, width, height, pitch):
    if data:
      super()._video(data, width, height, pitch)
      self.video_fresh = True
    else:
      self.video_null = True

  def pixels(self):
    return super().pixels() if self.video_fresh and not self.video_null else None


def consume(core, max_frames):
  collector = Collector()
  for frame_number in range(1, max_frames + 1):
    core.run_frame()
    collector.feed(core.pixels())
    if collector.done:
      print(f"gba lines fixture: {EVENTS} events, full VRAM pages and palettes match; "
            f"completion at libretro frame {frame_number}")
      return
  raise AssertionError(f"fixture did not complete within {max_frames} libretro frames "
                       f"(accepted {collector.index}/{EVENTS * 3} chunks)")


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--rom", type=pathlib.Path, required=True)
  parser.add_argument("--core", type=pathlib.Path, required=True)
  parser.add_argument("--max-frames", type=int, default=12000)
  args = parser.parse_args()
  if args.max_frames <= 0:
    parser.error("--max-frames must be positive")
  core = FixtureCore(args.rom, args.core)
  try:
    consume(core, args.max_frames)
  finally:
    core.close()


if __name__ == "__main__":
  main()
