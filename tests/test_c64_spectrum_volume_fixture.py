"""Host-only checks of the fixture oracle and trace validation, not VICE."""
import ctypes
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import c64_spectrum_volume_fixture as fixture  # noqa: E402


class FixtureOracleTests(unittest.TestCase):
  def test_checksums(self):
    self.assertEqual(fixture.checksums(bytes([1, 2, 3])), bytes([6, 0, 3, 0]))
    self.assertEqual(fixture.checksums(bytes([255]) * 1000)[:2], (255000 % 65536).to_bytes(2, "little"))

  def test_band_and_transition_vectors(self):
    trace = fixture.expected_trace()
    self.assertEqual(len(trace), 112 * 26)

    def record(step):
      return trace[step * 26:(step + 1) * 26]

    screen = bytearray([32] * 1000)
    colour = bytearray(1000)
    for pos, ink in ((326, 6), (605, 4), (884, 5), (885, 5)):
      screen[pos] = 160
      colour[pos] = ink
    self.assertEqual(record(0)[17:21], fixture.checksums(screen))
    self.assertEqual(record(0)[21:25], fixture.checksums(colour))
    for band in range(8):
      start = band * 10
      for step, height in ((0, 1), (4, 5), (5, 4), (9, 0)):
        expected = [0] * 8
        expected[band] = height
        self.assertEqual(list(record(start + step)[:8]), expected)
    self.assertEqual(list(record(84)[:8]), [0, 0, 1, 1, 2, 4, 5, 5])
    self.assertEqual(list(record(89)[:8]), [0, 0, 0, 0, 1, 2, 2, 3])
    self.assertEqual(list(record(105)[:8]), [5] * 8)
    self.assertEqual(list(record(105)[8:16]), [3] * 8)
    self.assertEqual(list(record(110)[:8]), [0] * 8)
    self.assertEqual(list(record(111)[:17]), [0] * 8 + [7] * 8 + [1])
    self.assertEqual(record(111)[17:25], fixture.checksums(bytes([32]) * 1000) + fixture.checksums(bytes(1000)))
    self.assertTrue(all(record(step)[25] == 1 for step in range(112)))

  def test_rejects_corruption_and_truncation(self):
    trace = fixture.expected_trace()
    fixture.validate_trace(trace)
    with self.assertRaisesRegex(ValueError, "trace length"):
      fixture.validate_trace(trace[:-1])
    for offset in (0, 16, 17, 21, 25, 111 * 26 + 25):
      damaged = bytearray(trace)
      damaged[offset] ^= 1
      with self.subTest(offset=offset), self.assertRaisesRegex(ValueError, "trace step"):
        fixture.validate_trace(bytes(damaged))

  def test_label_parser_rejects_missing_and_duplicate(self):
    with tempfile.TemporaryDirectory() as directory:
      path = pathlib.Path(directory) / "fixture.lbl"
      path.write_text("al 001377 ._sv_fixture_trace\n")
      self.assertEqual(fixture.label(path, "sv_fixture_trace"), 0x1377)
      with self.assertRaisesRegex(ValueError, "exactly one"):
        fixture.label(path, "sv_fixture_done")
      path.write_text("al 001377 ._sv_fixture_trace\nal 001378 ._sv_fixture_trace\n")
      with self.assertRaisesRegex(ValueError, "exactly one"):
        fixture.label(path, "sv_fixture_trace")

  def test_frame_budget_and_timeout_diagnostics(self):
    class FakeEmulator:
      def __init__(self, mode):
        self.memory = bytearray(8192)
        self.frame = 0
        self.mode = mode
        self.trace = fixture.expected_trace()
        if mode != "not-started":
          self.memory[0xE10:0xE14] = fixture.PROGRESS

      def run(self, frames):
        self.frame += frames
        count = min(self.frame // 150, 2 if self.mode == "stalled" else 4)
        if self.mode != "not-started":
          self.memory[0xE14] = 113 if self.mode == "invalid" else count
          self.memory[0x100:0x100 + count * 26] = self.trace[:count * 26]
        if self.mode == "corrupt" and count:
          self.memory[0x100] ^= 1
        if self.mode == "complete" and self.frame == 5:
          self.memory[0x100:0x100 + len(self.trace)] = self.trace
          self.memory[0xE00:0xE04] = fixture.DONE

      def peek(self, address):
        return self.memory[address]

    self.assertEqual(fixture.run_fixture(FakeEmulator("complete"), 0x100, 0xE00, 0xE10, 5), 5)
    with self.assertRaisesRegex(RuntimeError, r"\(300, 2\), \(600, 4\)"):
      fixture.run_fixture(FakeEmulator("progress"), 0x100, 0xE00, 0xE10, 600)
    with self.assertRaisesRegex(RuntimeError, r"\(300, 2\), \(600, 2\)"):
      fixture.run_fixture(FakeEmulator("stalled"), 0x100, 0xE00, 0xE10, 600)
    with self.assertRaisesRegex(RuntimeError, r"records\): \[\]"):
      fixture.run_fixture(FakeEmulator("not-started"), 0x100, 0xE00, 0xE10, 300)
    with self.assertRaisesRegex(ValueError, "completed trace prefix differs"):
      fixture.run_fixture(FakeEmulator("corrupt"), 0x100, 0xE00, 0xE10, 300)
    with self.assertRaisesRegex(ValueError, "invalid completed-record count"):
      fixture.run_fixture(FakeEmulator("invalid"), 0x100, 0xE00, 0xE10, 300)

  def test_oracle_against_host_scene(self):
    class State(ctypes.Structure):
      _fields_ = [("height", ctypes.c_uint8 * 8), ("palette", ctypes.c_uint8 * 8),
                  ("next_palette", ctypes.c_uint8)]

    source = ROOT / "cores/c64/homebrew/src"
    with tempfile.TemporaryDirectory() as directory:
      library = pathlib.Path(directory) / "scene.so"
      subprocess.run([os.environ.get("CC", "cc"), "-std=c99", "-Wall", "-Wextra", "-Werror",
                      "-pedantic", "-shared", "-fPIC", "-I", str(source),
                      str(source / "spectrum_volume.c"), "-o", str(library)], check=True, capture_output=True)
      scene = ctypes.CDLL(str(library))
      ptr = ctypes.POINTER(ctypes.c_uint8)
      scene.sv_init.argtypes = [ctypes.POINTER(State), ctypes.c_uint8]
      scene.sv_step.argtypes = [ctypes.POINTER(State), ptr, ctypes.c_uint8, ctypes.c_uint8, ptr, ptr]
      state = State()
      screen = (ctypes.c_uint8 * 1032)()
      colour = (ctypes.c_uint8 * 1032)()
      screen_cells = ctypes.cast(ctypes.byref(screen, 16), ptr)
      colour_cells = ctypes.cast(ctypes.byref(colour, 16), ptr)
      trace = bytearray()
      for step in range(fixture.STEPS):
        reset, initial, bands, energy, palette = fixture.input_at(step)
        if reset:
          screen[:] = [0xA5] * 16 + [32] * 1000 + [0xA5] * 16
          colour[:] = [0x5A] * 16 + [0] * 1000 + [0x5A] * 16
          scene.sv_init(ctypes.byref(state), initial)
        spectrum = (ctypes.c_uint8 * 8)(*bands)
        scene.sv_step(ctypes.byref(state), spectrum, energy, palette, screen_cells, colour_cells)
        guards = screen[:16] == [0xA5] * 16 and screen[1016:] == [0xA5] * 16
        guards = guards and colour[:16] == [0x5A] * 16 and colour[1016:] == [0x5A] * 16
        trace.extend(state.height)
        trace.extend(state.palette)
        trace.append(state.next_palette)
        trace.extend(fixture.checksums(bytes(screen[16:1016])))
        trace.extend(fixture.checksums(bytes(colour[16:1016])))
        trace.append(int(guards))
      fixture.validate_trace(bytes(trace))


if __name__ == "__main__":
  unittest.main()
