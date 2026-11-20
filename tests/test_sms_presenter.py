"""Host-only checks of the candidate's VRAM and trace-to-colour arithmetic."""

import importlib.util
from itertools import product
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("sms_assets", ROOT / "cores/sms/homebrew/field_assets.py")
assets = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(assets)


class SmsPresenterTests(unittest.TestCase):
  def test_vram_regions_do_not_overlap(self):
    patterns = set(range(0, 128))
    native = set(range(0x1800, 0x1f00))
    sprite_end = {0x1f00}
    maps = [set(range(0x2000 + i * 0x800, 0x2800 + i * 0x800)) for i in range(4)]
    regions = [patterns, native, sprite_end, *maps]
    for i, first in enumerate(regions):
      for second in regions[i + 1:]:
        self.assertTrue(first.isdisjoint(second))
    self.assertEqual([(0xf9 + 2 * i & 0x0e) << 10 for i in range(4)],
                     [0x2000, 0x2800, 0x3000, 0x3800])
    self.assertEqual((0xf7 & 0x0e) << 10, 0x1800)

  def test_trace_palette_address_arithmetic(self):
    combos, levels = assets.combos(), assets.cram_levels()
    for family, layout, combo in product(range(2), range(4), range(4)):
      count = 3 if layout & 2 else 2
      offset = family * 32 + (layout & 2) * 8 + combo * 4
      for rotation in range(count):
        for region_levels in product(range(4), repeat=count):
          staged = []
          for i in range(count):
            index = i + rotation
            if index >= count:
              index -= count
            staged.append(levels[combos[offset + index] * 4 + region_levels[i]])
          expected = []
          for i, level in enumerate(region_levels):
            colour = assets.COLOURS[assets.COMBOS[family * 2 + count - 2][combo][(i + rotation) % count]]
            expected.append(sum(((((colour >> s) & 3) * level + 1) // 3) << s for s in (0, 2, 4)))
          self.assertEqual(staged, expected, (family, layout, combo, rotation, region_levels))


if __name__ == "__main__":
  unittest.main()
