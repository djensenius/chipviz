"""Assemble the isolated scene and check its actual tile/map section bytes.

These are not execution or emulator tests. RGBDS tools are optional in
the normal host-test environment, like the existing GB emulator dependencies.
"""

import pathlib
import re
import shutil
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
HOME = ROOT / "cores/gb/homebrew"
TOOLS = shutil.which("rgbasm") and shutil.which("rgblink")


def expected_assets():
  """Independent pixel/map oracle for the target's static artwork."""
  glyphs = []
  for tile in range(32):
    points = set()
    if tile & 1:
      points.update((x, 3) for x in range(8))
    if tile & 2:
      points.update((4 if tile & 16 else 3, y) for y in range(8))
    if tile & 4:
      points.update((7 - y, y) for y in range(8))
    if tile & 8:
      points.update((y, y) for y in range(8))
    glyphs.append(points)
  tiles = bytes(value for glyph in glyphs for y in range(8)
                for value in (0, sum(1 << (7 - x) for x, py in glyph if py == y)))
  maps = []
  for hallway in (False, True):
    tilemap = bytearray(1024)
    for y in range(19):
      for x in range(10):
        horizontal = vertical = diagonal = False
        if hallway:
          for edge in (0, 2, 4, 6, 8):
            horizontal |= y in (edge, 18 - edge) and x >= edge
            vertical |= x == edge and edge <= y <= 18 - edge
          diagonal = x <= 8 and x in (y, 18 - y)
        elif y >= 4:
          rays = [9 - (y - 4) * slope // 14 for slope in (9, 5, 2)]
          horizontal = x >= rays[0] and y in (5, 7, 10, 14, 18)
          vertical = x == 9
          diagonal = x in rays
        glyph = int(horizontal) | (int(vertical) << 1) | (int(diagonal) << 2)
        tilemap[y * 32 + x] = glyph
        tilemap[y * 32 + 19 - x] = (
            int(horizontal) | (int(vertical) << 1) | (int(diagonal) << 3) | (int(vertical) << 4)
        )
    maps.append(bytes(tilemap))
  return tiles, *maps


def pixels(tiles, tilemap):
  image = []
  for y in range(19 * 8):
    row = []
    for x in range(160):
      tile = tilemap[(y // 8) * 32 + x // 8]
      offset = tile * 16 + (y % 8) * 2
      bit = 7 - x % 8
      row.append(((tiles[offset] >> bit) & 1) | (((tiles[offset + 1] >> bit) & 1) << 1))
    image.append(row)
  return image


@unittest.skipUnless(TOOLS, "rgbasm and rgblink are required")
class LinesTargetAssetsTests(unittest.TestCase):
  @classmethod
  def setUpClass(cls):
    cls.temp = tempfile.TemporaryDirectory()
    cls.addClassCleanup(cls.temp.cleanup)
    directory = pathlib.Path(cls.temp.name)
    path, rom, symbols = (directory / name for name in ("lines.o", "lines.gb", "lines.sym"))
    # Standalone link: only the display shadows normally supplied by main.asm
    # are external. They are never executed in this asset test.
    flags = [arg for i, name in enumerate(("wShBGP", "wShLCDC", "wShSCX", "wShSCY"))
             for arg in ("-D", f"{name}={0xC000 + i}")]
    subprocess.run(["rgbasm", *flags, "-o", str(path), str(HOME / "lines.asm")],
                   check=True, capture_output=True)
    subprocess.run(["rgblink", "-n", str(symbols), "-o", str(rom), str(path)],
                   check=True, capture_output=True)
    addresses = {name: int(address.split(":")[1], 16)
                 for address, name in (line.split() for line in symbols.read_text().splitlines()
                                       if line and not line.startswith(";"))}
    image = rom.read_bytes()
    cls.sections = {section: image[addresses[start]:addresses[end]]
                    for section, start, end in (
                        ("Lines tiles", "LinesTiles", "LinesTilesEnd"),
                        ("Lines plane map", "LinesPlane", "LinesPlaneEnd"),
                        ("Lines hallway map", "LinesHallway", "LinesHallwayEnd"))}

  def test_target_bytes_match_pixel_oracle(self):
    actual = tuple(self.sections[name] for name in ("Lines tiles", "Lines plane map", "Lines hallway map"))
    self.assertEqual(actual, expected_assets())
    self.assertEqual(tuple(map(len, actual)), (512, 1024, 1024))

  def test_target_pixels_are_sparse_mirrored_and_distinct(self):
    tiles = self.sections["Lines tiles"]
    maps = [self.sections["Lines plane map"], self.sections["Lines hallway map"]]
    self.assertNotEqual(*maps)
    for tilemap in maps:
      self.assertLess(max(tilemap), len(tiles) // 16)
      image = pixels(tiles, tilemap)
      self.assertTrue(all(row == row[::-1] for row in image))
      self.assertEqual({pixel for row in image for pixel in row}, {0, 2})
      for scroll in range(8):
        visible = image[scroll:scroll + 144]
        count = sum(pixel != 0 for row in visible for pixel in row)
        self.assertGreater(count, 100)
        self.assertLess(count, 160 * 144 // 4)

  def test_plane_grid_and_nested_corridor_structure(self):
    tiles = self.sections["Lines tiles"]
    plane = pixels(tiles, self.sections["Lines plane map"])
    hallway = pixels(tiles, self.sections["Lines hallway map"])
    # The grid bars widen progressively towards the viewer.
    for y, start in ((43, 72), (59, 64), (83, 48), (115, 24)):
      self.assertTrue(all(pixel == 2 for pixel in plane[y][start:160 - start]))
    # Five complete nested frames and their continuous left edges.
    for edge in (0, 2, 4, 6, 8):
      top, bottom, left = edge * 8 + 3, (18 - edge) * 8 + 3, edge * 8 + 3
      self.assertTrue(all(pixel == 2 for pixel in hallway[top][edge * 8:160 - edge * 8]))
      self.assertTrue(all(pixel == 2 for pixel in hallway[bottom][edge * 8:160 - edge * 8]))
      self.assertTrue(all(hallway[y][left] == 2 for y in range(edge * 8, (19 - edge) * 8)))

  def test_map_swap_changes_only_sparse_pixels_at_bounded_scrolls(self):
    tiles = self.sections["Lines tiles"]
    plane = pixels(tiles, self.sections["Lines plane map"])
    hallway = pixels(tiles, self.sections["Lines hallway map"])
    for before in range(8):
      for after in range(max(0, before - 1), min(7, before + 1) + 1):
        changed = sum(a != b for y in range(144)
                      for a, b in zip(plane[y + before], hallway[y + after]))
        self.assertLess(changed, 160 * 144 // 4)

  def test_transition_and_vram_source_contract(self):
    source = (HOME / "lines.asm").read_text()
    step = source.split("\nLinesStep:\n", 1)[1].split('SECTION "Lines tiles"', 1)[0]
    self.assertRegex(source, r"(?m)^DEF LINES_HOLD EQU 240$")
    for threshold in ("LINES_HOLD", "85", "170", "14"):
      self.assertRegex(step, rf"(?m)^  cp {threshold}$")
    self.assertEqual(set(re.findall(r"(?m)^  ld hl, (\w+)$", step)),
                     {"wLinesHold", "wLinesClock", "wLinesPhase"})
    self.assertEqual(set(re.findall(r"(?m)^  ld \[(\w+)\]", step)),
                     {"hl", "wLinesLook", "wShBGP", "wShLCDC", "wShSCX", "wShSCY"})
    self.assertNotRegex(step, r"(?m)^  ldh ")
    self.assertIn("ld a, $91", step)
    self.assertIn("ld a, $99", step)

  def test_scene_source_assembles_with_all_existing_roles(self):
    for flags in ([], ["-D", "CHIPVIZ_INPUT_ROLE_OPERATOR"], ["-D", "CHIPVIZ_FIXTURE"]):
      with self.subTest(flags=flags):
        path = pathlib.Path(self.temp.name) / ("role-" + str(len(flags)) + ".o")
        subprocess.run(
            ["rgbasm", *flags, "-I", str(HOME) + "/", "-P", str(HOME / "lines.asm"),
             "-o", str(path), str(HOME / "main.asm")],
            check=True, capture_output=True,
        )


if __name__ == "__main__":
  unittest.main()
