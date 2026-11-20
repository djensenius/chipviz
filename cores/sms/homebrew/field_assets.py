"""Prepare SMS mode-4 colour-field patterns, maps and CRAM lookup values.

These assets are not yet used by the SDK-free seed ROM.
"""

import argparse
from pathlib import Path


WIDTH, HEIGHT, TILE = 256, 192, 8
COLUMNS, ROWS = 32, 28
MAP_SIZE = 0x800

# Specification order: A.R, A.Y, A.B, W, K, B.R, B.G, B.B, B.C, B.M, B.Y.
COLOURS = (0x03, 0x0B, 0x24, 0x3F, 0x00, 0x03, 0x0C, 0x30, 0x3C, 0x33, 0x0F)
# Indices into COLOURS, in family, region-count and combo order.
COMBOS = (
  ((0, 2), (1, 2), (0, 1), (2, 3)),
  ((0, 1, 2), (0, 3, 2), (1, 4, 2), (0, 4, 1)),
  ((5, 8), (6, 9), (7, 10), (8, 9)),
  ((5, 6, 7), (8, 9, 10), (5, 4, 8), (7, 4, 10)),
)


def patterns():
  """Tiles 0..3: each pixel's palette index equals its tile number."""
  out = bytearray()
  for index in range(4):
    row = bytes(0xFF if index & (1 << plane) else 0 for plane in range(4))
    out.extend(row * TILE)
  return bytes(out)


def region(layout, column, row):
  """Return the region for a visible tile, with horizontal region 0 at bottom."""
  if layout not in range(4) or column not in range(COLUMNS) or row not in range(HEIGHT // TILE):
    raise ValueError("invalid layout or visible tile")
  count = 3 if layout >= 2 else 2
  size = COLUMNS if layout % 2 == 0 else HEIGHT // TILE
  pos = column if layout % 2 == 0 else size - 1 - row
  return next(i for i in range(count) if (i * size) // count <= pos < ((i + 1) * size) // count)


def name_map(layout):
  """A 2 KiB-aligned map, with solid pattern 1..3 for each region."""
  if layout not in range(4):
    raise ValueError("invalid layout")
  data = bytearray()
  for row in range(ROWS):
    for column in range(COLUMNS):
      index = region(layout, column, row) + 1 if row < HEIGHT // TILE else 1
      data.extend((index, 0))
  data.extend(bytes(MAP_SIZE - len(data)))
  return bytes(data)


def cram_levels():
  """Colour-major CRAM values, levels 0..3, rounding each 2-bit channel."""
  out = bytearray()
  for colour in COLOURS:
    for level in range(4):
      value = 0
      for shift in (0, 2, 4):
        channel = (colour >> shift) & 3
        value |= ((channel * level + 1) // 3) << shift
      out.append(value)
  return bytes(out)


def combos():
  """Padded lookup indexed by family * 32 + (regions - 2) * 16 + combo * 4."""
  out = bytearray()
  for group in COMBOS:
    for combo in group:
      out.extend(combo)
      out.extend(bytes([0xFF] * (4 - len(combo))))
  return bytes(out)


def pad_edges():
  """Direction edges indexed by previous * 16 + current, active high.

  Output bits: scene pulse, palette previous, palette next. Opposing held
  directions cancel even if only one of them is a new edge.
  """
  out = bytearray()
  for previous in range(16):
    for current in range(16):
      new = current & ~previous
      scene = bool(new & 3) and (current & 3) != 3
      left = bool(new & 4) and (current & 12) != 12
      right = bool(new & 8) and (current & 12) != 12
      out.append(int(scene) | (int(left) << 1) | (int(right) << 2))
  return bytes(out)


def procedural_inputs():
  """One 256-frame period of the shared procedural input's 13 base bytes."""
  out = bytearray()
  for frame in range(256):
    phase = (frame * 17) & 0xFF
    out.extend((80 + phase // 2, 255 - phase, 64 + ((frame * 9) & 0x7F),
                (frame * 23) & 0xFF))
    out.extend((phase + band * 29) & 0xFF for band in range(8))
    out.append(int(phase < 17))
  return bytes(out)


def spectral_initial_levels():
  """Entry levels for all byte-valued bands, in band-value order."""
  return bytes(sum(value >= threshold for threshold in (48, 104, 168)) for value in range(256))


def spectral_next_levels():
  """Hysteresis lookup indexed by old level * 256 + byte-valued band."""
  up = (0, 48, 104, 168)
  down = (0, 32, 88, 152)
  out = bytearray()
  for level in range(4):
    for value in range(256):
      if level < 3 and value >= up[level + 1]:
        out.append(level + 1)
      elif level > 0 and value < down[level]:
        out.append(level - 1)
      else:
        out.append(level)
  return bytes(out)


def cut_actions():
  """Hard-cut action ring: rotate, combo, rotate, layout."""
  return bytes((0, 1, 0, 2))


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("output", type=Path, help="directory for generated binary assets")
  args = parser.parse_args()
  args.output.mkdir(parents=True, exist_ok=True)
  (args.output / "field-patterns.bin").write_bytes(patterns())
  (args.output / "field-maps.bin").write_bytes(b"".join(name_map(i) for i in range(4)))
  (args.output / "field-cram-levels.bin").write_bytes(cram_levels())
  (args.output / "field-combos.bin").write_bytes(combos())
  (args.output / "field-pad-edges.bin").write_bytes(pad_edges())
  (args.output / "field-procedural.bin").write_bytes(procedural_inputs())
  (args.output / "field-spectral-initial.bin").write_bytes(spectral_initial_levels())
  (args.output / "field-spectral-next.bin").write_bytes(spectral_next_levels())
  (args.output / "field-cut-actions.bin").write_bytes(cut_actions())


if __name__ == "__main__":
  main()
