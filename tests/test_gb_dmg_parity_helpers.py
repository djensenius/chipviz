"""Portable checks for the opt-in DMG fixture capture protocol."""

import pathlib
import sys
import tempfile
import types
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import gb_dmg_parity_capture as capture
import gb_libretro_capture as glc


class DmgParityHelpers(unittest.TestCase):
  def test_freshness_and_frame_shape(self):
    frame = glc.Frame(160, 144, bytes(capture.FRAME_BYTES))
    self.assertEqual(capture.fresh_frame([frame]), frame.rgb)
    for callbacks in ([], [None], [frame, frame], [glc.Frame(160, 144, b"")],
                      [glc.Frame(144, 160, frame.rgb)]):
      with self.subTest(callbacks=callbacks), self.assertRaises(ValueError):
        capture.fresh_frame(callbacks)

  def test_a_missing_new_callback_cannot_reuse_the_previous_frame(self):
    old = glc.Frame(160, 144, bytes(capture.FRAME_BYTES))
    fake = types.SimpleNamespace(frame=old, callbacks=[old], run_frame=lambda: None)
    with self.assertRaisesRegex(ValueError, "missing"):
      capture.CaptureCore.capture_run(fake)
    self.assertIsNone(fake.frame)

  def test_palette_boundaries_and_wrap(self):
    for step, expected in (
      (1, (128, 1, 0xE4)), (15, (0x87, 15, 0xE4)),
      (16, (8, 16, 0xEC)), (31, (0x8F, 31, 0xEC)),
      (32, (16, 32, 0xF4)), (47, (0x97, 47, 0xF4)),
      (48, (24, 48, 0xFC)), (63, (0x9F, 63, 0xFC)),
      (64, (32, 64, 0xE4)), (255, (255, 255, 0xFC)),
      (256, (0, 0, 0xE4)), (257, (128, 1, 0xE4)),
    ):
      with self.subTest(step=step):
        self.assertEqual(capture.registers(step), expected)

  def test_missing_symbol_is_rejected(self):
    with tempfile.TemporaryDirectory() as directory:
      path = pathlib.Path(directory) / "fixture.sym"
      path.write_text("00:c000 wFrame\n")
      with self.assertRaisesRegex(ValueError, "missing symbols"):
        capture.symbols(path)


if __name__ == "__main__":
  unittest.main()
