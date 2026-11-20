"""Checks for the mGBA cadence comparison (the production ROM test is separate)."""

import pathlib
import sys
import unittest
from unittest.mock import patch


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import gba_native_controls_test as native


def pages(count, interval, start=0):
  return [(start + i * interval, i.to_bytes(2, "little")) for i in range(count)]


class CadenceComparison(unittest.TestCase):
  def compare(self, baseline, actual):
    with patch.object(native, "native_timeline", side_effect=[baseline, actual]):
      native.check_cadence("production", "baseline", "core")

  def test_accepts_boot_alignment_and_one_frame_jitter(self):
    baseline = pages(225, 4)
    actual = [(step + 1 + (i % 2), image) for i, (step, image) in enumerate(baseline)]
    self.compare(baseline, actual)

  def test_rejects_accelerated_presentation(self):
    with self.assertRaisesRegex(AssertionError, "299 changes vs baseline 225"):
      self.compare(pages(225, 4), pages(299, 3))

  def test_rejects_timing_drift_with_same_page_count(self):
    baseline = pages(225, 4)
    actual = [(step + (2 if i >= 100 else 0), image) for i, (step, image) in enumerate(baseline)]
    with self.assertRaisesRegex(AssertionError, "2 VBlanks of relative drift"):
      self.compare(baseline, actual)

  def test_rejects_different_animation_sequence(self):
    baseline = pages(225, 4)
    actual = list(baseline)
    actual[100] = (actual[100][0], b"changed")
    with self.assertRaisesRegex(AssertionError, "no matching baseline page sequence"):
      self.compare(baseline, actual)


def overlay_frame(mask):
  pixels = bytearray(240 * 160 * 2)
  for y in range(160):
    for x in range(240):
      value = (x + y * 251) & 0xFFFF
      if mask & 1 and y == 12:
        value = 0x1234
      if mask & 2 and x == 12:
        value = 0x5678
      offset = (y * 240 + x) * 2
      pixels[offset:offset + 2] = value.to_bytes(2, "little")
  return bytes(pixels)


class OverlayComparison(unittest.TestCase):
  def test_distinguishes_real_chord_from_separate_buttons(self):
    frames = [overlay_frame(mask) for mask in range(4)]
    self.assertEqual([native.overlay_mask(frame) for frame in frames], [0, 1, 2, 3])
    native.assert_overlay_sequence([frames[i] for i in (0, 1, 0, 2, 0)], [1, 0, 2, 0], "sequential")
    native.assert_overlay_sequence([frames[i] for i in (0, 1, 3, 2, 0)], [1, 3, 2, 0], "overlapping")
    with self.assertRaisesRegex(AssertionError, "expected"):
      native.assert_overlay_sequence([frames[i] for i in (0, 3, 0)], [1, 0, 2, 0], "manufactured chord")


if __name__ == "__main__":
  unittest.main()
