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


if __name__ == "__main__":
  unittest.main()
