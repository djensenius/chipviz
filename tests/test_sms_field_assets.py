"""SMS field asset checks against the normative geometry and CRAM table."""

import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("sms_field_assets", ROOT / "cores/sms/homebrew/field_assets.py")
assets = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(assets)


class SmsFieldAssetsTests(unittest.TestCase):
  def test_patterns_are_solid_palette_indices(self):
    data = assets.patterns()
    self.assertEqual(len(data), 128)
    for index in range(4):
      for y in range(8):
        planes = data[index * 32 + y * 4:index * 32 + y * 4 + 4]
        for bit in range(8):
          pixel = sum(((plane >> bit) & 1) << p for p, plane in enumerate(planes))
          self.assertEqual(pixel, index)

  def test_all_map_cells_and_padding(self):
    # Explicit boundaries in pixels, measured from the left or bottom.
    cases = ((256, (128,)), (192, (96,)), (256, (80, 168)), (192, (64, 128)))
    for layout, (size, bounds) in enumerate(cases):
      data = assets.name_map(layout)
      self.assertEqual(len(data), 2048)
      for row in range(28):
        for col in range(32):
          offset = (row * 32 + col) * 2
          if row < 24:
            pos = col * 8 if layout % 2 == 0 else size - 1 - row * 8
            expected = 1 + sum(pos >= boundary for boundary in bounds)
          else:
            expected = 1
          self.assertEqual(data[offset:offset + 2], bytes((expected, 0)), (layout, col, row))
      self.assertEqual(data[28 * 32 * 2:], bytes(2048 - 28 * 32 * 2))

  def test_cram_scaling_and_spec_colours(self):
    colours = (0x03, 0x0B, 0x24, 0x3F, 0x00, 0x03, 0x0C, 0x30, 0x3C, 0x33, 0x0F)
    data = assets.cram_levels()
    self.assertEqual(len(data), len(colours) * 4)
    for index, colour in enumerate(colours):
      for level in range(4):
        channels = [(colour >> shift) & 3 for shift in (0, 2, 4)]
        expected = sum(((channel * level + 1) // 3) << shift
                       for channel, shift in zip(channels, (0, 2, 4)))
        self.assertEqual(data[index * 4 + level], expected)
      self.assertEqual(data[index * 4], 0)
      self.assertEqual(data[index * 4 + 3], colour)

  def test_combo_colours_and_rotation(self):
    # Spec colours by family and number of regions, independent of asset indices.
    expected = (
      ((0x03, 0x24), (0x0B, 0x24), (0x03, 0x0B), (0x24, 0x3F)),
      ((0x03, 0x0B, 0x24), (0x03, 0x3F, 0x24), (0x0B, 0x00, 0x24), (0x03, 0x00, 0x0B)),
      ((0x03, 0x3C), (0x0C, 0x33), (0x30, 0x0F), (0x3C, 0x33)),
      ((0x03, 0x0C, 0x30), (0x3C, 0x33, 0x0F), (0x03, 0x00, 0x3C), (0x30, 0x00, 0x0F)),
    )
    table, levels = assets.combos(), assets.cram_levels()
    self.assertEqual(len(table), 64)
    for group, combinations in enumerate(expected):
      for combo, colours in enumerate(combinations):
        indices = table[group * 16 + combo * 4:group * 16 + combo * 4 + 4]
        self.assertEqual(indices[len(colours):], bytes([0xFF] * (4 - len(colours))))
        for rot in range(len(colours)):
          actual = tuple(levels[indices[(i + rot) % len(colours)] * 4 + 3] for i in range(len(colours)))
          self.assertEqual(actual, colours[rot:] + colours[:rot])

  def test_invalid_coordinates_and_layouts(self):
    for args in ((4, 0, 0), (-1, 0, 0), (0, 32, 0), (0, 0, 24), (0, -1, 0)):
      with self.assertRaises(ValueError):
        assets.region(*args)
    with self.assertRaises(ValueError):
      assets.name_map(4)

  def test_pad_direction_edges_exhaustively(self):
    table = assets.pad_edges()
    self.assertEqual(len(table), 256)
    for previous in range(16):
      for current in range(16):
        state = table[previous * 16 + current]
        up = bool(current & 1) and not bool(previous & 1)
        down = bool(current & 2) and not bool(previous & 2)
        left = bool(current & 4) and not bool(previous & 4)
        right = bool(current & 8) and not bool(previous & 8)
        self.assertEqual(bool(state & 1), (up or down) and (current & 3) != 3)
        self.assertEqual(bool(state & 2), left and (current & 12) != 12)
        self.assertEqual(bool(state & 4), right and (current & 12) != 12)
        self.assertEqual(state & ~7, 0)

  def test_f24_relative_ring_pulses(self):
    table = assets.pad_edges()
    previous = 0
    ring = 0
    values = []
    for step in range(600):
      current = 8 if step in (10, 20, 30) else 4 if step == 130 else 0
      edges = table[previous * 16 + current]
      if edges & 2:
        ring = (ring - 1) & 7
      if edges & 4:
        ring = (ring + 1) & 7
      values.append(ring)
      previous = current
    self.assertEqual(values[:10], [0] * 10)
    self.assertEqual(values[10:20], [1] * 10)
    self.assertEqual(values[20:30], [2] * 10)
    self.assertEqual(values[30:130], [3] * 100)
    self.assertEqual(values[130:], [2] * 470)


if __name__ == "__main__":
  unittest.main()
