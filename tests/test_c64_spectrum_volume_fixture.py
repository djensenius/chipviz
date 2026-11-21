"""Host-only checks of the fixture oracle and trace validation, not VICE."""
import contextlib
import ctypes
import io
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import c64_spectrum_volume_fixture as fixture  # noqa: E402


class FixtureOracleTests(unittest.TestCase):
  def test_checksums(self):
    self.assertEqual(fixture.checksums(bytes([1, 2, 3])), bytes([6, 0, 0xad, 0xad]))
    self.assertEqual(fixture.checksums(b"123456789")[2:], bytes([0xb1, 0x29]))
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
        if mode == "old-version":
          self.memory[0xE00:0xE04] = b"SV01"
          self.memory[0xE10:0xE14] = b"SVP1"
          self.memory[0xE14] = 4
        elif mode != "not-started":
          self.memory[0xE10:0xE14] = fixture.PROGRESS

      def run(self, frames):
        self.frame += frames
        count = min(self.frame // 150, 2 if self.mode == "stalled" else 4)
        if self.mode not in ("not-started", "old-version"):
          self.memory[0xE14] = 113 if self.mode == "invalid" else count
          size = count * 26 - (1 if self.mode == "partial" and count else 0)
          self.memory[0x100:0x100 + size] = self.trace[:size]
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
    with self.assertRaisesRegex(RuntimeError, r"records\): \[\]"):
      fixture.run_fixture(FakeEmulator("old-version"), 0x100, 0xE00, 0xE10, 300)
    with self.assertRaisesRegex(ValueError, "completed trace prefix differs"):
      fixture.run_fixture(FakeEmulator("corrupt"), 0x100, 0xE00, 0xE10, 300)
    with self.assertRaisesRegex(ValueError, "completed trace prefix differs"):
      fixture.run_fixture(FakeEmulator("partial"), 0x100, 0xE00, 0xE10, 300)
    with self.assertRaisesRegex(ValueError, "invalid completed-record count"):
      fixture.run_fixture(FakeEmulator("invalid"), 0x100, 0xE00, 0xE10, 300)

  def test_cli_frame_bounds(self):
    for frames in (1, 16000):
      with self.subTest(frames=frames), mock.patch.object(sys, "argv", ["fixture", "--frames", str(frames)]), \
          mock.patch.object(fixture, "label", side_effect=RuntimeError("accepted")):
        with self.assertRaisesRegex(RuntimeError, "accepted"):
          fixture.main()
    for frames in (0, -1, 16001):
      with self.subTest(frames=frames), mock.patch.object(sys, "argv", ["fixture", "--frames", str(frames)]), \
          contextlib.redirect_stderr(io.StringIO()) as stderr:
        with self.assertRaises(SystemExit) as result:
          fixture.main()
        self.assertEqual(result.exception.code, 2)
        self.assertIn("--frames must be between 1 and 16000", stderr.getvalue())

  def test_emulator_lifecycle(self):
    for scenario in ("success", "load", "load-and-deinit", "run", "interrupt", "unload"):
      with self.subTest(scenario=scenario):
        emu = mock.Mock()
        error = None
        if scenario.startswith("load"):
          error = RuntimeError("load failed")
          emu.load.side_effect = error
          if scenario == "load-and-deinit":
            emu.lib.retro_deinit.side_effect = RuntimeError("deinit failed")
        elif scenario == "run":
          error = RuntimeError("run failed")
        elif scenario == "interrupt":
          error = KeyboardInterrupt()
        elif scenario == "unload":
          error = RuntimeError("unload failed")
          emu.lib.retro_unload_game.side_effect = error
        run_error = error if scenario in ("run", "interrupt") else None
        with mock.patch.object(sys, "argv", ["fixture"]), \
            mock.patch.object(fixture, "label", side_effect=[0x1000, 0x2000, 0x2004]), \
            mock.patch.object(fixture, "Emulator", return_value=emu), \
            mock.patch.object(fixture, "run_fixture", return_value=1, side_effect=run_error) as run, \
            contextlib.redirect_stdout(io.StringIO()):
          if error is None:
            self.assertEqual(fixture.main(), 0)
          else:
            with self.assertRaises(type(error)) as raised:
              fixture.main()
            self.assertIs(raised.exception, error)
        if scenario.startswith("load"):
          run.assert_not_called()
          self.assertEqual(emu.mock_calls, [mock.call.load(mock.ANY), mock.call.lib.retro_deinit()])
          if scenario == "load-and-deinit":
            self.assertIn("deinit failed", error.__notes__[0])
        else:
          run.assert_called_once()
          self.assertEqual(emu.mock_calls, [mock.call.load(mock.ANY), mock.call.lib.retro_unload_game(),
                                            mock.call.lib.retro_deinit()])

  def test_host_fixture_crc_and_shifted_scene_regression(self):
    class State(ctypes.Structure):
      _fields_ = [("height", ctypes.c_uint8 * 8), ("palette", ctypes.c_uint8 * 8),
                  ("next_palette", ctypes.c_uint8)]

    source = ROOT / "cores/c64/homebrew/src"
    with tempfile.TemporaryDirectory() as directory:
      library = pathlib.Path(directory) / "scene.so"
      wrapper = pathlib.Path(directory) / "fixture_wrapper.c"
      wrapper.write_text('#define main fixture_program_main\n#include "c64_spectrum_volume_fixture.c"\n'
                         '#undef main\n'
                         'void host_checksum(const uint8_t *cells, uint16_t *sum, uint16_t *crc) '
                         '{ checksum(cells, sum, crc); }\n')
      subprocess.run([os.environ.get("CC", "cc"), "-std=c99", "-Wall", "-Wextra", "-Werror",
                      "-pedantic", "-shared", "-fPIC", "-I", str(source), "-I", str(ROOT / "tests"),
                      str(source / "spectrum_volume.c"), str(wrapper), "-o", str(library)],
                     check=True, capture_output=True)
      scene = ctypes.CDLL(str(library))
      ptr = ctypes.POINTER(ctypes.c_uint8)
      scene.sv_init.argtypes = [ctypes.POINTER(State), ctypes.c_uint8]
      scene.sv_step.argtypes = [ctypes.POINTER(State), ptr, ctypes.c_uint8, ctypes.c_uint8, ptr, ptr]
      scene.host_checksum.argtypes = [ptr, ctypes.POINTER(ctypes.c_uint16), ctypes.POINTER(ctypes.c_uint16)]

      def c_checksums(cells):
        buf = (ctypes.c_uint8 * 1000).from_buffer_copy(cells)
        total, crc = ctypes.c_uint16(), ctypes.c_uint16()
        scene.host_checksum(buf, ctypes.byref(total), ctypes.byref(crc))
        return total.value.to_bytes(2, "little") + crc.value.to_bytes(2, "little")

      def old_rolling(cells):
        rolling = 0
        for cell in cells:
          rolling = (((rolling << 1) | (rolling >> 15)) & 0xFFFF) ^ cell
        return rolling

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
        screen_bytes, colour_bytes = bytes(screen[16:1016]), bytes(colour[16:1016])
        trace.extend(c_checksums(screen_bytes))
        trace.extend(c_checksums(colour_bytes))
        trace.append(int(guards))
        if step in (0, 4, 84, 105):
          for name, cells, blank, offset in (("screen", screen_bytes, 32, 17),
                                             ("colour", colour_bytes, 0, 21)):
            with self.subTest(step=step, surface=name):
              # All affected cells fit: this moves the actual scene writes,
              # including stale colours, 16 cells without discarding any.
              self.assertEqual(cells[-16:], bytes([blank]) * 16)
              shifted = cells[-16:] + cells[:-16]
              original, moved = c_checksums(cells), c_checksums(shifted)
              self.assertEqual(original[:2], moved[:2])
              if step == 0:
                self.assertEqual(old_rolling(cells), old_rolling(shifted))
              self.assertNotEqual(original[2:], moved[2:])
              damaged = bytearray(fixture.expected_trace())
              start = step * fixture.RECORD_SIZE + offset
              damaged[start:start + 4] = moved
              with self.assertRaisesRegex(ValueError, f"trace step {step}, byte"):
                fixture.validate_trace(bytes(damaged))
      fixture.validate_trace(bytes(trace))


if __name__ == "__main__":
  unittest.main()
