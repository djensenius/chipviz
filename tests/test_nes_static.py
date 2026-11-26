"""Host tests for the isolated NES Static core; no ROM or emulator required."""

import ctypes
import pathlib
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "cores" / "nes" / "homebrew" / "src" / "static_scene.c"
PIXEL, GLYPH, MIXED = 0, 1, 2
NO_PATCH, PATCH_READY, INVALID_LOOK = 0, 1, 2


class State(ctypes.Structure):
  _fields_ = [("cursor", ctypes.c_uint), ("random", ctypes.c_ubyte),
              ("wait", ctypes.c_ubyte), ("beat_held", ctypes.c_ubyte)]


class Patch(ctypes.Structure):
  _fields_ = [("address", ctypes.c_uint), ("tiles", ctypes.c_ubyte * 8)]


class Oracle:
  """Independent byte-oriented model of the documented scan and noise."""

  def __init__(self):
    self.cursor, self.seed, self.wait, self.beat = 0, 0xA5, 0, False

  def step(self, look, energy, band, beat):
    if beat and not self.beat:
      self.seed = (self.seed ^ 0x5A) or 0xA5
    self.beat = bool(beat)
    if self.wait:
      self.wait -= 1
      return None
    self.wait = 3 - energy // 64
    address = 0x2000 + self.cursor
    tiles = []
    for _ in range(8):
      self.seed ^= (self.seed << 3) & 255
      self.seed ^= self.seed >> 5
      self.seed ^= (self.seed << 1) & 255
      value = self.seed ^ band ^ (band >> 3) ^ (band >> 6)
      tiles.append((8 + (value & 7)) if look == GLYPH else
                   (value & 15) if look == MIXED else (value & 7))
    self.cursor = (self.cursor + 8) % 960
    return address, tiles


class NesStaticTests(unittest.TestCase):
  @classmethod
  def setUpClass(cls):
    cls.temp = tempfile.TemporaryDirectory()
    library = pathlib.Path(cls.temp.name) / "static.dylib"
    subprocess.run(["cc", "-std=c99", "-Wall", "-Wextra", "-Werror",
                    "-shared", "-fPIC", str(SOURCE), "-o", str(library)], check=True)
    cls.lib = ctypes.CDLL(str(library))
    cls.lib.nes_static_init.argtypes = [ctypes.POINTER(State)]
    cls.lib.nes_static_step.argtypes = [ctypes.POINTER(State), ctypes.c_ubyte,
                                      ctypes.c_ubyte, ctypes.c_ubyte, ctypes.c_ubyte,
                                      ctypes.POINTER(Patch)]
    cls.lib.nes_static_step.restype = ctypes.c_ubyte

  @classmethod
  def tearDownClass(cls):
    cls.temp.cleanup()

  def start(self):
    state = State()
    self.lib.nes_static_init(ctypes.byref(state))
    return state

  def step(self, state, look, energy, band, beat):
    patch = Patch()
    status = self.lib.nes_static_step(ctypes.byref(state), look, energy, band,
                                      beat, ctypes.byref(patch))
    self.assertIn(status, (NO_PATCH, PATCH_READY))
    return (patch.address, list(patch.tiles)) if status == PATCH_READY else None

  def test_chr_and_palette(self):
    chr_data = bytes((ctypes.c_ubyte * (17 * 16)).in_dll(self.lib, "nes_static_chr"))
    tiles = [chr_data[i:i + 16] for i in range(0, len(chr_data), 16)]
    self.assertEqual(len(set(tiles)), 17)
    for tile in tiles[:16]:
      self.assertEqual(sum(byte.bit_count() for byte in tile[:8]), 8)
      self.assertEqual(tile[8:], bytes(8))
    self.assertEqual(tiles[16], bytes(16))
    self.assertEqual(list((ctypes.c_ubyte * 4).in_dll(self.lib, "nes_static_palette")),
                     [0x0F, 0x10, 0x0F, 0x0F])

  def test_oracle_and_bounded_transitions(self):
    state, oracle = self.start(), Oracle()
    screen = [16] * 960
    for frame in range(5000):
      # Includes rapid look switches, extreme energy and held beats.
      look = (frame // 137) % 3
      energy = [0, 63, 64, 127, 128, 191, 192, 255][(frame // 73) % 8]
      band = (frame * 37) & 255
      beat = frame % 19 < 5
      expected = oracle.step(look, energy, band, beat)
      actual = self.step(state, look, energy, band, beat)
      self.assertEqual(actual, expected, frame)
      if actual is None:
        continue
      address, tiles = actual
      offset = address - 0x2000
      self.assertEqual(offset % 8, 0)
      self.assertLessEqual(offset + 8, 960)
      before = screen[:]
      screen[offset:offset + 8] = tiles
      self.assertLessEqual(sum(a != b for a, b in zip(before, screen)), 8)
      if look == GLYPH:
        self.assertTrue(all(8 <= tile < 16 for tile in tiles))
      elif look == PIXEL:
        self.assertTrue(all(tile < 8 for tile in tiles))
    self.assertNotIn(16, screen)

  def test_cadence_beat_band_and_reset(self):
    for look in (PIXEL, GLYPH, MIXED):
      for energy, period in ((0, 4), (64, 3), (128, 2), (192, 1), (255, 1)):
        state = self.start()
        emitted = [i for i in range(32) if self.step(state, look, energy, 0, 0)]
        self.assertEqual(emitted, list(range(0, 32, period)))
    def run(band, beats):
      state = self.start()
      return [self.step(state, 2, 255, band, beats[i]) for i in range(40)]
    quiet = run(0, [0] * 40)
    self.assertEqual(quiet, run(0, [0] * 40))
    for band in (3, 64, 128, 192, 255):
      self.assertNotEqual(quiet, run(band, [0] * 40))
    self.assertNotEqual(quiet, run(0, [1] * 40))
    self.assertEqual(run(0, [1] * 40), run(0, [1] + [0] * 39))
    self.assertNotEqual(run(0, [1] * 40), run(0, [i % 2 for i in range(40)]))

  def test_cursor_wrap_and_beat_while_waiting(self):
    state, oracle = self.start(), Oracle()
    for index in range(121):
      expected = oracle.step(MIXED, 255, 0, 0)
      actual = self.step(state, MIXED, 255, 0, 0)
      self.assertEqual(actual, expected)
      self.assertEqual(actual[0], 0x2000 + (index % 120) * 8)

    state, oracle = self.start(), Oracle()
    for beat in (0, 1, 1, 0, 0):
      self.assertEqual(self.step(state, PIXEL, 0, 64, beat),
                       oracle.step(PIXEL, 0, 64, beat))
    self.assertEqual(state.random, oracle.seed)
    self.lib.nes_static_init(ctypes.byref(state))
    self.assertEqual((state.cursor, state.random, state.wait, state.beat_held),
                     (0, 0xA5, 0, 0))

  def test_invalid_look_is_atomic_and_distinct_from_no_patch(self):
    for waiting in (False, True):
      for held in (0, 1):
        state = self.start()
        if waiting:
          self.step(state, PIXEL, 0, 0, 0)  # wait=3
        state.beat_held = held
        baseline = State.from_buffer_copy(bytes(state))
        patch = Patch()
        ctypes.memset(ctypes.byref(patch), 0xA5, ctypes.sizeof(patch))
        original_patch = bytes(patch)
        for look in range(3, 256):
          status = self.lib.nes_static_step(ctypes.byref(state), look, 255, 255,
                                            1 - held, ctypes.byref(patch))
          self.assertEqual(status, INVALID_LOOK, look)
          self.assertEqual(bytes(state), bytes(baseline), look)
          self.assertEqual(bytes(patch), original_patch, look)
        # A rejected beat and look must not affect a subsequent valid step.
        self.assertEqual(self.step(state, GLYPH, 0, 64, 1 - held),
                         self.step(baseline, GLYPH, 0, 64, 1 - held))
        self.assertEqual(bytes(state), bytes(baseline))

    state = self.start()
    self.step(state, PIXEL, 0, 0, 0)
    before = bytes(state)
    patch = Patch()
    ctypes.memset(ctypes.byref(patch), 0xA5, ctypes.sizeof(patch))
    original_patch = bytes(patch)
    status = self.lib.nes_static_step(ctypes.byref(state), PIXEL, 0, 0, 1,
                                      ctypes.byref(patch))
    self.assertEqual(status, NO_PATCH)
    self.assertNotEqual(bytes(state), before)
    self.assertEqual(bytes(patch), original_patch)


if __name__ == "__main__":
  unittest.main()
