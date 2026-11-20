"""Host checks for SMS spectral lookup data and normative level fixtures.

The fixture model consumes the same lookup bytes as the Z80, but does not
execute the Z80 module or validate its integration.
"""

import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("sms_assets", ROOT / "cores/sms/homebrew/field_assets.py")
assets = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(assets)


def spectral_step(levels, values, silent, lg):
  """Host model of one later mode-rule step plus the counter increment."""
  if lg >= 10:
    if silent:
      next_levels = (0, 0, 0)
    else:
      table = assets.spectral_next_levels()
      n = len(values)
      next_levels = tuple(table[levels[i] * 256 + values[i]] for i in range(n)) + (0,) * (3 - n)
    if next_levels != levels:
      levels, lg = next_levels, 0
  return levels, min(lg + 1, 10)


def trace(values, length=600):
  """Two equal bands, spectral power-on entry, no scene or palette change."""
  initial = assets.spectral_initial_levels()[values(0)]
  levels = (initial, initial, 0)
  lg = 10
  output = [levels]
  for step in range(1, length):
    v = values(step)
    levels, lg = spectral_step(levels, (v, v), False, lg)
    output.append(levels)
  return output


class SmsSpectralTests(unittest.TestCase):
  def test_all_entry_and_hysteresis_values(self):
    initial = assets.spectral_initial_levels()
    table = assets.spectral_next_levels()
    self.assertEqual(len(initial), 256)
    self.assertEqual(len(table), 1024)
    up = (48, 104, 168)
    down = (32, 88, 152)
    for value in range(256):
      self.assertEqual(initial[value], sum(value >= threshold for threshold in up))
      for level in range(4):
        expected = level
        if level < 3 and value >= up[level]:
          expected += 1
        elif level > 0 and value < down[level - 1]:
          expected -= 1
        self.assertEqual(table[level * 256 + value], expected, (level, value))

  def test_f19_digital_energy_spectral_levels(self):
    result = trace(lambda step: 200 if 320 <= step < 380 else 64)
    self.assertEqual(result[:320], [(1, 1, 0)] * 320)
    self.assertEqual(result[320:330], [(2, 2, 0)] * 10)
    self.assertEqual(result[330:380], [(3, 3, 0)] * 50)
    self.assertEqual(result[380:390], [(2, 2, 0)] * 10)
    self.assertEqual(result[390:], [(1, 1, 0)] * 210)

  def test_f20_saturation_and_field_wide_budget(self):
    result = trace(lambda step: 250 if step < 200 or step >= 400 else 10)
    self.assertEqual(result[:200], [(3, 3, 0)] * 200)
    self.assertEqual(result[200:210], [(2, 2, 0)] * 10)
    self.assertEqual(result[210:220], [(1, 1, 0)] * 10)
    self.assertEqual(result[220:400], [(0, 0, 0)] * 180)
    self.assertEqual(result[400:410], [(1, 1, 0)] * 10)
    self.assertEqual(result[410:420], [(2, 2, 0)] * 10)
    self.assertEqual(result[420:], [(3, 3, 0)] * 180)

  def test_blackout_waits_for_budget_and_does_not_repeat(self):
    levels, lg = (3, 2, 0), 0
    for _ in range(10):
      levels, lg = spectral_step(levels, (250, 250), True, lg)
      self.assertEqual(levels, (3, 2, 0))
    levels, lg = spectral_step(levels, (250, 250), True, lg)
    self.assertEqual((levels, lg), ((0, 0, 0), 1))
    levels, lg = spectral_step(levels, (250, 250), True, lg)
    self.assertEqual((levels, lg), ((0, 0, 0), 2))


if __name__ == "__main__":
  unittest.main()
