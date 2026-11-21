"""Test-only RGBDS harness for the isolated Lines scene.

The fixture-build and oracle checks do not run an emulator. Actual execution
requires explicit CHIPVIZ_GB_LINES_EXECUTION=1 and a SameBoy libretro core.
"""

import contextlib
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
HOME = ROOT / "cores/gb/homebrew"
FIXTURE = ROOT / "tests/fixtures/gb_lines_execution.asm"
ROUNDTRIP = ROOT / "tests/fixtures/gb_lines_roundtrip.asm"
SIGNED = ROOT / "tests/fixtures/gb_signed_tiles.asm"
sys.path.insert(0, str(ROOT / "scripts"))

import gb_libretro_capture as glc
from test_gb_colour_field import GREY, expected_shades, region_of
from test_gb_lines import expected_assets, pixels


TOOLS = all(shutil.which(name) for name in ("rgbasm", "rgblink", "rgbfix"))
EXECUTION = os.environ.get("CHIPVIZ_GB_LINES_EXECUTION") == "1"
READY, SEQ, ACK, COMMAND, ENERGY, REQUEST, SHADOW = range(0xC000, 0xC007)
SNAPSHOT = 0xC00A
MIRROR = 0xC100


def build_fixture(directory):
  obj = directory / "lines-fixture.o"
  rom = directory / "lines-fixture.gb"
  symbols = directory / "lines-fixture.sym"
  subprocess.run(["rgbasm", "-I", str(HOME) + "/", "-o", str(obj), str(FIXTURE)],
                 check=True, capture_output=True)
  subprocess.run(["rgblink", "-n", str(symbols), "-o", str(rom), str(obj)],
                 check=True, capture_output=True)
  subprocess.run(["rgbfix", "-v", "-p", "0", "-t", "LINESFIXTURE", str(rom)],
                 check=True, capture_output=True)
  return rom, symbols


def build_roundtrip(directory, operator):
  """Build a test-only ROM with the real production routines and Lines."""
  role = "operator" if operator else "data"
  obj = directory / f"roundtrip-{role}.o"
  rom = directory / f"roundtrip-{role}.gb"
  symbols = directory / f"roundtrip-{role}.sym"
  memory_map = directory / f"roundtrip-{role}.map"
  flags = ["-D", "CHIPVIZ_INPUT_ROLE_OPERATOR"] if operator else []
  subprocess.run(["rgbasm", *flags, "-I", str(HOME) + "/", "-o", str(obj), str(ROUNDTRIP)],
                 check=True, capture_output=True)
  subprocess.run(["rgblink", "-n", str(symbols), "-m", str(memory_map), "-o", str(rom), str(obj)],
                 check=True, capture_output=True)
  addresses = {}
  for line in symbols.read_text().splitlines():
    if line and not line.startswith(";"):
      address, name = line.split()
      addresses[name] = int(address.split(":")[1], 16)
  # Redirect only this temporary test ROM, never the production source/ROM.
  image = bytearray(rom.read_bytes())
  if image[0x100:0x104] != bytes((0, 0xC3, 0x50, 0x01)):
    raise AssertionError("production entry no longer jumps to Main at $0150")
  entry = addresses["RoundtripMain"]
  image[0x102:0x104] = entry.to_bytes(2, "little")
  rom.write_bytes(image)
  subprocess.run(["rgbfix", "-v", "-p", "0", "-t", "LINESROUNDTRIP", str(rom)],
                 check=True, capture_output=True)
  return rom, addresses


def build_signed(directory, operator):
  """Build the isolated signed-tile experiment with real production routines."""
  role = "operator" if operator else "data"
  obj = directory / f"signed-{role}.o"
  rom = directory / f"signed-{role}.gb"
  symbols = directory / f"signed-{role}.sym"
  flags = ["-D", "CHIPVIZ_INPUT_ROLE_OPERATOR"] if operator else []
  subprocess.run(["rgbasm", *flags, "-I", str(HOME) + "/", "-o", str(obj), str(SIGNED)],
                 check=True, capture_output=True)
  subprocess.run(["rgblink", "-n", str(symbols), "-o", str(rom), str(obj)],
                 check=True, capture_output=True)
  addresses = {}
  for line in symbols.read_text().splitlines():
    if line and not line.startswith(";"):
      address, name = line.split()
      addresses[name] = int(address.split(":")[1], 16)
  image = bytearray(rom.read_bytes())
  if image[0x100:0x104] != bytes((0, 0xC3, 0x50, 0x01)):
    raise AssertionError("production entry no longer jumps to Main at $0150")
  image[0x102:0x104] = addresses["SignedMain"].to_bytes(2, "little")
  rom.write_bytes(image)
  subprocess.run(["rgbfix", "-v", "-p", "0", "-t", "SIGNEDFIXTURE", str(rom)],
                 check=True, capture_output=True)
  return rom, addresses


def expected_scroll(step, interval):
  """Closed-form scroll for a constant energy after a reload; step is 1-based."""
  phase = (step // interval) % 14
  return min(phase, 14 - phase)


def expected_frame(vram, shadow):
  """Decode fixture VRAM and DMG shadows to 160x144 RGB, without the core."""
  if len(vram) != 512 + 2048:
    raise ValueError("fixture requires 32 tiles and two complete maps")
  bgp, lcdc, scx, scy = shadow
  if lcdc not in (0x91, 0x99):
    raise ValueError("oracle supports only the fixture's BG-only unsigned-tile modes")
  tiles = vram[:512]
  start = 512 + (1024 if lcdc & 8 else 0)
  tilemap = vram[start:start + 1024]
  if max(tilemap) >= 32:
    raise ValueError("map references a tile outside fixture VRAM")
  shades = (255, 170, 85, 0)
  rgb = bytearray()
  for y in range(144):
    sy = (y + scy) & 255
    for x in range(160):
      sx = (x + scx) & 255
      tile = tilemap[(sy // 8) * 32 + sx // 8]
      offset = tile * 16 + (sy % 8) * 2
      bit = 7 - sx % 8
      colour = ((tiles[offset] >> bit) & 1) | (((tiles[offset + 1] >> bit) & 1) << 1)
      intensity = shades[(bgp >> (colour * 2)) & 3]
      rgb.extend((intensity, intensity, intensity))
  return bytes(rgb)


def expected_warning_frame():
  """Spec text in the DMG's 5x7 font, white on black, spaced 16px apart."""
  glyphs = {
      "A": "0e11111f111111", "B": "1e11111e11111e", "C": "0e11101010110e",
      "E": "1f10101e10101f", "F": "1f10101e101010", "G": "0e11101711110f",
      "H": "1111111f111111", "I": "0e04040404040e", "L": "1010101010101f",
      "M": "111b1515111111", "N": "11111915131111", "O": "0e11111111110e",
      "P": "1e11111e101010", "R": "1e11111e141211", "S": "0f10100e01011e",
      "T": "1f040404040404", "U": "1111111111110e", "V": "11111111110a04",
      "W": "1111111515150a", "Y": "11110a04040404", "Z": "1f01020408101f",
      ".": "00000000000c0c", ":": "000c0c000c0c00", "=": "00001f001f0000",
  }
  lines = ("WARNING: FLASHING", "LIGHT. MAY TRIGGER", "SEIZURES IN PEOPLE",
           "WITH PHOTOSENSITIVE", "EPILEPSY.", "START = ENABLE", "OTHER = CANCEL")
  image = bytearray(160 * 144 * 3)
  for line, text in enumerate(lines):
    for column, char in enumerate(text):
      if char == " ":
        continue
      for y, bits in enumerate(bytes.fromhex(glyphs[char])):
        for x in range(5):
          if bits & (1 << (4 - x)):
            offset = ((line * 16 + y) * 160 + column * 8 + x + 1) * 3
            image[offset:offset + 3] = b"\xff\xff\xff"
  return bytes(image)


@unittest.skipUnless(TOOLS, "rgbasm, rgblink and rgbfix are required")
class FixtureBuildTests(unittest.TestCase):
  def test_fixture_links_actual_lines_source(self):
    with tempfile.TemporaryDirectory() as temp:
      rom, symbols = build_fixture(pathlib.Path(temp))
      self.assertEqual(len(rom.read_bytes()), 32768)
      entries = [line.split() for line in symbols.read_text().splitlines() if line and not line.startswith(";")]
      addresses = {name: int(address.split(":")[1], 16) for address, name in entries}
      self.assertTrue({"FixtureMain", "FixtureMain.present", "LinesLoad", "LinesStep",
                       "LinesTiles", "LinesPlane", "LinesHallway"}
                      <= addresses.keys())
      for name, address in (("wReady", READY), ("wSeq", SEQ), ("wAck", ACK),
                            ("wCommand", COMMAND), ("wEnergy", ENERGY), ("wRequest", REQUEST),
                            ("wShBGP", SHADOW), ("wSnapshot", SNAPSHOT), ("wMirror", MIRROR)):
        self.assertEqual(addresses[name], address)

  def test_roundtrip_and_signed_fixtures_link_for_both_roles(self):
    with tempfile.TemporaryDirectory() as temp:
      for operator in (False, True):
        with self.subTest(operator=operator):
          for build, entry in ((build_roundtrip, "RoundtripMain"), (build_signed, "SignedMain")):
            rom, symbols = build(pathlib.Path(temp), operator)
            self.assertEqual(len(rom.read_bytes()), 32768)
            self.assertIn(entry, symbols)
            self.assertEqual("WarnStep" in symbols, operator)


class ScrollOracleTests(unittest.TestCase):
  def test_closed_form_has_expected_period_and_one_pixel_bound(self):
    for interval in (2, 4, 8):
      values = [expected_scroll(step, interval) for step in range(28 * interval + 1)]
      self.assertEqual(values[:interval], [0] * interval)
      self.assertEqual(values[7 * interval], 7)
      self.assertEqual(values[14 * interval], 0)
      self.assertTrue(all(0 <= value <= 7 for value in values))
      self.assertTrue(all(abs(a - b) <= 1 for a, b in zip(values, values[1:])))


class DisplayOracleTests(unittest.TestCase):
  def test_tile_planes_palette_maps_and_scroll_wrap(self):
    vram = bytearray(512 + 2048)
    vram[16] = 0xC0                 # tile 1, row 0: low plane at x=0,1
    vram[17] = 0x40                 # colour 3 at x=1
    vram[32 + 1] = 0x80            # tile 2, row 0: colour 2 at x=0
    vram[32 + 15] = 0x01           # tile 2, row 7: colour 2 at x=7
    vram[512] = 1                  # plane map (0,0)
    vram[512 + 1023] = 2           # plane map (31,31)
    vram[512 + 1024] = 2           # hallway map (0,0)

    def pixel(frame, x, y):
      start = (y * 160 + x) * 3
      return tuple(frame[start:start + 3])

    plane = expected_frame(vram, (0xE4, 0x91, 0, 0))
    self.assertEqual(len(plane), 160 * 144 * 3)
    self.assertEqual(pixel(plane, 0, 0), (170,) * 3)
    self.assertEqual(pixel(plane, 1, 0), (0,) * 3)
    self.assertEqual(pixel(plane, 2, 0), (255,) * 3)
    hallway = expected_frame(vram, (0xE4, 0x99, 0, 0))
    self.assertEqual(pixel(hallway, 0, 0), (85,) * 3)
    wrapped = expected_frame(vram, (0xE4, 0x91, 255, 255))
    self.assertEqual(pixel(wrapped, 0, 0), (85,) * 3)
    self.assertEqual(pixel(wrapped, 1, 1), (170,) * 3)
    inverted = expected_frame(vram, (0x1B, 0x91, 0, 0))
    self.assertEqual(pixel(inverted, 0, 0), (85,) * 3)
    self.assertEqual(pixel(inverted, 1, 0), (255,) * 3)
    with self.assertRaises(ValueError):
      expected_frame(vram, (0xE4, 0x81, 0, 0))
    vram[512] = 32
    with self.assertRaises(ValueError):
      expected_frame(vram, (0xE4, 0x91, 0, 0))

  def test_target_artwork_matches_existing_pixel_model(self):
    tiles, plane, hallway = expected_assets()
    vram = tiles + plane + hallway
    for tilemap, lcdc, scroll in ((plane, 0x91, 3), (hallway, 0x99, 4)):
      with self.subTest(lcdc=lcdc):
        image = pixels(tiles, tilemap)
        expected = bytes(channel for row in image[scroll:scroll + 144]
                         for colour in row for channel in ((255, 170, 85, 0)[colour],) * 3)
        self.assertEqual(expected_frame(vram, (0xE4, lcdc, 0, scroll)), expected)


class Fixture:
  def __init__(self, rom):
    self.gb = glc.GbCore(rom, glc.default_core(), video=False)
    self.seq = 0

  def boot(self):
    self.gb.reset()
    self.gb.write(READY, b"\0")
    for _ in range(600):
      self.gb.run_frame()
      if self.gb.read(READY)[0] == 0xA5:
        self.seq = self.gb.read(SEQ)[0]
        return
    raise AssertionError("Lines fixture never became ready")

  def command(self, command, energy=0, look=0):
    self.gb.write(COMMAND, bytes((command, energy, look)))
    self.seq = (self.seq + 1) & 0xFF
    self.gb.write(SEQ, bytes((self.seq,)))
    for _ in range(12):
      self.gb.run_frame()
      if self.gb.read(ACK)[0] == self.seq:
        return tuple(self.gb.read(SHADOW, 4)), tuple(self.gb.read(SNAPSHOT, 4))
    raise AssertionError(f"Lines fixture did not acknowledge sequence {self.seq}")

  def close(self):
    self.gb.close()


@unittest.skipUnless(EXECUTION, "set CHIPVIZ_GB_LINES_EXECUTION=1 to run SameBoy")
class LinesExecutionTests(unittest.TestCase):
  @classmethod
  def setUpClass(cls):
    if not TOOLS:
      raise RuntimeError("execution requested but RGBDS tools are unavailable")
    if not glc.default_core().is_file():
      raise RuntimeError(f"execution requested but SameBoy core is missing: {glc.default_core()}")
    cls.temp = tempfile.TemporaryDirectory()
    cls.addClassCleanup(cls.temp.cleanup)
    cls.rom, _ = build_fixture(pathlib.Path(cls.temp.name))

  def setUp(self):
    self.fixture = Fixture(self.rom)
    self.addCleanup(self.fixture.close)
    self.fixture.boot()

  def test_load_and_reload_restore_assets_and_reset_state(self):
    assets = b"".join(expected_assets())
    gb = self.fixture.gb
    self.assertEqual(gb.read(MIRROR, len(assets)), assets)
    self.assertEqual(gb.read(SHADOW, 4), bytes((0x12, 0x34, 0x56, 0x78)))
    self.assertEqual(gb.read(SNAPSHOT, 4), bytes(4))
    for _ in range(253):
      shadow, _ = self.fixture.command(1, 170, 1)
    self.assertEqual(shadow[1], 0x99)
    before = shadow
    shadow, state = self.fixture.command(2)
    self.assertEqual(shadow, before)  # LinesLoad does not prepare display shadows.
    self.assertEqual(state, (0, 0, 0, 0))
    self.assertEqual(gb.read(MIRROR, len(assets)), assets)
    for _ in range(239):
      shadow, _ = self.fixture.command(1, 84, 1)
      self.assertEqual(shadow[1], 0x91)
    self.assertEqual(self.fixture.command(1, 84, 1)[0][1], 0x99)

  def test_look_changes_are_held_for_240_steps(self):
    for _ in range(239):
      shadow, state = self.fixture.command(1, 85, 255)
      self.assertEqual(shadow[1], 0x91)
    self.assertEqual(state[:2], (0, 239))
    shadow, state = self.fixture.command(1, 85, 255)
    self.assertEqual((shadow[1], state[:2]), (0x99, (1, 0)))
    for _ in range(239):
      shadow, state = self.fixture.command(1, 85, 0)
      self.assertEqual(shadow[1], 0x99)
    self.assertEqual(state[:2], (1, 239))
    shadow, state = self.fixture.command(1, 85, 0)
    self.assertEqual((shadow[1], state[:2]), (0x91, (0, 0)))
    for _ in range(260):
      _, state = self.fixture.command(1, 85, 0)
    self.assertEqual(state[:2], (0, 240))
    self.assertEqual(self.fixture.command(1, 85, 1)[0][1], 0x99)

  def test_exact_speed_boundaries_and_display_shadows(self):
    for energy, interval in ((84, 8), (85, 4), (169, 4), (170, 2)):
      with self.subTest(energy=energy):
        self.fixture.command(2)
        previous = 0
        for step in range(1, 28 * interval + 1):
          shadow, _ = self.fixture.command(1, energy, 0)
          self.assertEqual(shadow, (0xE4, 0x91, 0, expected_scroll(step, interval)))
          self.assertLessEqual(abs(shadow[3] - previous), 1)
          previous = shadow[3]

  def test_alternating_energy_keeps_motion_and_palette_bounded(self):
    previous = 0
    for step in range(600):
      shadow, _ = self.fixture.command(1, (0, 84, 85, 169, 170, 255)[step % 6], 0)
      self.assertEqual(shadow[:3], (0xE4, 0x91, 0))
      self.assertLessEqual(shadow[3], 7)
      self.assertLessEqual(abs(shadow[3] - previous), 1)
      previous = shadow[3]

  def test_displayed_plane_and_hallway_pixels(self):
    # Present the target's own shadows, then wait two frames so the video
    # callback cannot still contain the frame preceding the VBlank writes.
    gb = self.fixture.gb
    gb.video = True
    vram = gb.read(MIRROR, 512 + 2048)
    self.assertEqual(vram, b"".join(expected_assets()))
    for _ in range(239):
      shadow, _ = self.fixture.command(1, 85, 1)
    self.assertEqual(shadow, (0xE4, 0x91, 0, 3))

    def check_frame(shadow):
      presented, _ = self.fixture.command(3)
      self.assertEqual(presented, shadow)
      gb.run_frame()
      gb.run_frame()
      self.assertIsNotNone(gb.frame)
      self.assertEqual((gb.frame.width, gb.frame.height), (160, 144))
      self.assertEqual(gb.frame.rgb, expected_frame(vram, shadow))

    check_frame(shadow)
    shadow, state = self.fixture.command(1, 85, 1)
    self.assertEqual(shadow, (0xE4, 0x99, 0, 4))
    self.assertEqual(state[:2], (1, 0))
    check_frame(shadow)


@unittest.skipUnless(EXECUTION, "set CHIPVIZ_GB_LINES_EXECUTION=1 to run SameBoy")
class LinesRoundtripTests(unittest.TestCase):
  """Test-only interposition, not production Lines dispatch or timing."""

  @classmethod
  def setUpClass(cls):
    if not TOOLS:
      raise RuntimeError("execution requested but RGBDS tools are unavailable")
    if not glc.default_core().is_file():
      raise RuntimeError(f"execution requested but SameBoy core is missing: {glc.default_core()}")
    cls.temp = tempfile.TemporaryDirectory()
    cls.addClassCleanup(cls.temp.cleanup)
    cls.images = {role: build_roundtrip(pathlib.Path(cls.temp.name), role == "operator")
                  for role in ("data", "operator")}

  def start(self, role):
    rom, symbols = self.images[role]
    self.assertEqual(symbols["wRoundReady"], 0xC040)
    self.assertEqual(symbols["wRoundUnknown"], 0xC200)
    self.assertEqual(symbols["wRoundVram"], 0xD000)
    self.assertEqual(symbols["wRoundPending"], 0xC2C0)
    self.assertEqual(symbols["wRoundFreeze"], 0xC04A)
    self.assertEqual("WarnStep" in symbols, role == "operator")
    for name in ("wFrame", "wLinesLook", "wLinesHold", "wLinesClock", "wLinesPhase"):
      self.assertFalse(0xC040 <= symbols[name] < 0xC04B)
      self.assertFalse(0xC200 <= symbols[name] < 0xC2CD)
    gb = glc.GbCore(rom, glc.default_core(), video=False)
    try:
      gb.reset()
      gb.write(symbols["wRoundReady"], b"\0")
      self.wait(gb, lambda: gb.read(symbols["wRoundReady"])[0] == 0xA5, "boot")
    except BaseException:
      gb.close()
      raise
    return gb, symbols

  def wait(self, gb, condition, name, limit=180):
    for _ in range(limit):
      if condition():
        return
      gb.run_frame()
    self.fail(f"roundtrip fixture timed out waiting for {name}")

  def pulse(self, gb, button):
    for _ in range(2):
      gb.run_frame((button,))
    for _ in range(2):
      gb.run_frame()

  def value(self, gb, symbols, name):
    return gb.read(symbols[name])[0]

  def field(self, gb, symbols):
    self.pulse(gb, "up")
    self.wait(gb, lambda: self.value(gb, symbols, "wShown7") == 1
              and self.value(gb, symbols, "wShowMode") == 1, "field entry")

  def native(self, gb, symbols):
    self.pulse(gb, "up")
    self.wait(gb, lambda: self.value(gb, symbols, "wShown7") == 0
              and self.value(gb, symbols, "wTraceShow") == 0, "native output")

  def enter_lines(self, gb, symbols):
    gb.write(symbols["wRoundRequest"], b"\1")
    self.wait(gb, lambda: self.value(gb, symbols, "wRoundActive") == 1, "Lines entry")
    self.assertEqual(self.value(gb, symbols, "wRoundRefused"), 0)
    original = gb.read(symbols["wRoundVram"], 512 + 2048)
    self.assertEqual(original[64:256], bytes((0x35 + 17 * i) & 255 for i in range(192)))
    return original

  def assert_restored(self, gb, symbols, original):
    self.wait(gb, lambda: self.value(gb, symbols, "wRoundRestores") == 1, "restoration")
    self.assertEqual(gb.read(symbols["wRoundVram"], 512 + 2048), original)
    self.assertEqual(gb.read(symbols["wRoundHardware"], 4),
                     gb.read(symbols["wRoundPending"], 4))
    self.assertEqual(self.value(gb, symbols, "wLatch"), 0)

  def test_native_roundtrip_restores_vram_and_prepared_registers(self):
    for role in ("data", "operator"):
      with self.subTest(role=role), contextlib.ExitStack() as stack:
        gb, symbols = self.start(role)
        stack.callback(gb.close)
        original = self.enter_lines(gb, symbols)
        for _ in range(8):
          gb.run_frame()
        gb.write(symbols["wRoundRequest"], b"\0")
        self.assert_restored(gb, symbols, original)

  def test_native_roundtrip_restores_displayed_pixels(self):
    # Freeze all steps, not just native animation, so both captures present
    # the same prepared output. This pauses scene-7 budgets and is not a
    # production handoff. Entry/restore turn the LCD off and use the real
    # loaders; wait for complete video callbacks after each LCD restart.
    for role in ("data", "operator"):
      for after_field in (False, True):
        with self.subTest(role=role, after_field=after_field), contextlib.ExitStack() as stack:
          gb, symbols = self.start(role)
          stack.callback(gb.close)
          if after_field:
            self.field(gb, symbols)
            self.native(gb, symbols)
          else:
            for _ in range(8):
              gb.run_frame()
          gb.write(symbols["wRoundFreeze"], b"\1")
          gb.video = True

          def frame():
            # Allow the last VBlank's writes to become a complete frame.
            # Keep the last callback in case the core duplicates a frame;
            # the intervening Lines comparison must differ from native.
            gb.run_frame()
            gb.run_frame()
            self.assertIsNotNone(gb.frame)
            self.assertEqual((gb.frame.width, gb.frame.height), (160, 144))
            return gb.frame.rgb

          before = frame()
          self.assertEqual(frame(), before)
          self.assertGreater(len(set(zip(before[0::3], before[1::3], before[2::3]))), 1)
          for name in ("wShowMode", "wShown7", "wShownShow", "wTraceShow", "wLatch"):
            self.assertEqual(self.value(gb, symbols, name), 0)
          native = gb.read(symbols["wShBGP"], 4)
          frozen = gb.read(symbols["wFrame"]), gb.read(symbols["wSinceScene"])

          original = self.enter_lines(gb, symbols)
          self.assertEqual(gb.read(symbols["wRoundPending"], 4), native)
          self.assertNotEqual(frame(), before)
          gb.write(symbols["wRoundRequest"], b"\0")
          self.assert_restored(gb, symbols, original)
          self.assertEqual(gb.read(symbols["wRoundPending"], 4), native)
          self.assertEqual((gb.read(symbols["wFrame"]), gb.read(symbols["wSinceScene"])), frozen)
          self.assertEqual(frame(), before)
          self.assertEqual(frame(), before)

  def test_scene7_exit_and_reentry_use_real_gates(self):
    for role in ("data", "operator"):
      with self.subTest(role=role), contextlib.ExitStack() as stack:
        gb, symbols = self.start(role)
        stack.callback(gb.close)
        self.field(gb, symbols)
        if role == "operator":
          for _ in range(3):
            self.pulse(gb, "right")
          self.wait(gb, lambda: self.value(gb, symbols, "wShowMode") == 2
                    and self.value(gb, symbols, "wWarnVis") == 1, "visible warning")
          self.pulse(gb, "start")
          self.wait(gb, lambda: self.value(gb, symbols, "wLatch") == 1, "operator arming")
        self.native(gb, symbols)
        self.assertEqual(self.value(gb, symbols, "wLatch"), 0)
        original = self.enter_lines(gb, symbols)
        before = self.value(gb, symbols, "wSinceScene")
        for _ in range(8):
          gb.run_frame()
        self.assertGreater(self.value(gb, symbols, "wSinceScene"), before)
        self.pulse(gb, "up")
        if role == "data":
          self.assertEqual(self.value(gb, symbols, "wShown7"), 0)
        self.assert_restored(gb, symbols, original)
        self.wait(gb, lambda: self.value(gb, symbols, "wShown7") == 1
                  and self.value(gb, symbols, "wShowMode") == 1, "field re-entry")
        self.assertEqual(self.value(gb, symbols, "wPowerOn"), 0)
        self.assertLess(self.value(gb, symbols, "wSinceCut"), 20)
        self.assertLess(self.value(gb, symbols, "wLg"), 10)
        shadow = gb.read(symbols["wShBGP"], 4)
        self.assertIn(shadow[1], (0x89, 0x99))
        self.assertEqual(shadow[2:], b"\0\0")

  def test_operator_warning_refuses_lines_and_preserves_warning(self):
    gb, symbols = self.start("operator")
    self.addCleanup(gb.close)
    self.field(gb, symbols)
    for _ in range(3):
      self.pulse(gb, "right")
    self.wait(gb, lambda: self.value(gb, symbols, "wShowMode") == 2
              and self.value(gb, symbols, "wWarnVis") == 1, "visible warning")
    gb.write(symbols["wRoundRequest"], b"\1")
    self.wait(gb, lambda: self.value(gb, symbols, "wRoundRefused") == 1, "refusal")
    self.assertEqual(self.value(gb, symbols, "wRoundActive"), 0)
    self.assertEqual(self.value(gb, symbols, "wRoundLoads"), 0)
    self.assertEqual(self.value(gb, symbols, "wShowMode"), 2)
    self.assertEqual(self.value(gb, symbols, "wWarnVis"), 1)
    self.assertEqual(self.value(gb, symbols, "wLatch"), 0)
    self.pulse(gb, "up")  # WarnStep cancels; it does not advance the ring.
    self.assertEqual(self.value(gb, symbols, "wShown7"), 1)
    self.assertEqual(self.value(gb, symbols, "wLatch"), 0)


@unittest.skipUnless(EXECUTION, "set CHIPVIZ_GB_LINES_EXECUTION=1 to run SameBoy")
class SignedTilesExecutionTests(unittest.TestCase):
  """Ownership experiment only: no production selection or live tile writes."""

  wait = LinesRoundtripTests.wait
  pulse = LinesRoundtripTests.pulse
  value = LinesRoundtripTests.value
  field = LinesRoundtripTests.field
  native = LinesRoundtripTests.native

  @classmethod
  def setUpClass(cls):
    if not TOOLS or not glc.default_core().is_file():
      raise RuntimeError("execution requested but RGBDS or SameBoy is unavailable")
    cls.temp = tempfile.TemporaryDirectory()
    cls.addClassCleanup(cls.temp.cleanup)
    cls.images = {role: build_signed(pathlib.Path(cls.temp.name), role == "operator")
                  for role in ("data", "operator")}

  def test_signed_tiles_restore_native_and_yield_to_field_and_warning(self):
    for role in ("data", "operator"):
      with self.subTest(role=role), contextlib.ExitStack() as stack:
        rom, symbols = self.images[role]
        self.assertEqual(symbols["wSignedAfter"], 0xC300)
        self.assertEqual(symbols["wSignedBefore"], 0xD000)
        self.assertEqual(symbols["wSignedReady"], 0xC040)
        self.assertEqual("WarnStep" in symbols, role == "operator")
        gb = glc.GbCore(rom, glc.default_core(), video=False)
        stack.callback(gb.close)
        gb.reset()
        gb.write(symbols["wSignedReady"], b"\0")
        self.wait(gb, lambda: self.value(gb, symbols, "wSignedReady") == 0xA5, "signed boot")
        baseline = gb.read(symbols["wSignedBefore"], 3200)
        self.assertEqual(baseline[64:256], bytes((0x35 + 17 * i) & 255 for i in range(192)))
        self.assertEqual(baseline[1152:2176], bytes(i % 4 for i in range(1024)))
        for _ in range(8):
          gb.run_frame()
        gb.write(symbols["wSignedFreeze"], b"\1")
        gb.video = True

        def frame():
          # Two frames allow pending VBlank output and LCD restart to settle.
          # Discard the previous callback so an old frame cannot be reused.
          gb.frame = None
          gb.run_frame()
          gb.run_frame()
          self.assertIsNotNone(gb.frame)
          self.assertEqual((gb.frame.width, gb.frame.height), (160, 144))
          return gb.frame.rgb

        before = frame()
        self.assertEqual(frame(), before)
        native_shadow = gb.read(symbols["wShBGP"], 4)
        self.assertEqual(native_shadow[1], 0x91)
        gb.write(symbols["wSignedRequest"], b"\1")
        self.wait(gb, lambda: self.value(gb, symbols, "wSignedActive") == 1, "signed output")
        self.assertEqual(self.value(gb, symbols, "wShLCDC"), 0x81)
        shown = frame()
        self.assertNotEqual(shown, before)
        bgp, _, scx, _ = native_shadow
        colours = (2, 1, 3, 0)
        shades = (255, 170, 85, 0)
        row = bytes(channel for x in range(160)
                    for channel in (shades[(bgp >> (colours[((x + scx) & 255) // 8 % 4] * 2)) & 3],) * 3)
        self.assertEqual(shown, row * 144)
        gb.write(symbols["wSignedRequest"], b"\0")
        self.wait(gb, lambda: self.value(gb, symbols, "wSignedActive") == 0, "native output")
        self.assertEqual(gb.read(symbols["wShBGP"], 4), native_shadow)
        self.assertEqual(frame(), before)

        # Resume real input/Step before exercising scene-7 priority.
        gb.write(symbols["wSignedFreeze"], b"\0")
        gb.write(symbols["wSignedRequest"], b"\1")
        self.wait(gb, lambda: self.value(gb, symbols, "wSignedActive") == 1, "signed again")
        gb.write(symbols["wSignedFreeze"], b"\1")
        diagnostic = frame()
        self.assertNotEqual(diagnostic, before)
        gb.write(symbols["wSignedFreeze"], b"\0")
        for _ in range(2):
          gb.run_frame(("up",))
        if role == "operator":
          self.assertEqual(self.value(gb, symbols, "wShown7"), 1)
          self.assertEqual(self.value(gb, symbols, "wSignedActive"), 0)
        else:
          self.assertEqual(self.value(gb, symbols, "wShown7"), 0)
          self.assertEqual(self.value(gb, symbols, "wSignedActive"), 1)
        for _ in range(2):
          gb.run_frame()
        self.wait(gb, lambda: self.value(gb, symbols, "wShown7") == 1
                  and self.value(gb, symbols, "wShowMode") == 1, "field entry")
        self.assertEqual(self.value(gb, symbols, "wSignedActive"), 0)
        self.assertEqual(self.value(gb, symbols, "wSignedRequest"), 0)
        self.assertEqual(self.value(gb, symbols, "wSignedRefused"), 1)
        self.wait(gb, lambda: self.value(gb, symbols, "wShownShow") == 1, "field presented")
        # Freeze the prepared field, then compare every displayed pixel to
        # the independent spec geometry and DMG shade oracle.
        gb.write(symbols["wSignedFreeze"], b"\1")
        field = frame()
        trace = tuple(gb.read(symbols["wShownShow"], 9))
        self.assertEqual(trace[:4], (1, 0, 0, 0))  # field, held mode, family 0, V2
        self.assertIn(trace[4], (0, 1, 2))  # held-mode combo follows the dominant band
        self.assertEqual(trace[5:], (0, 3, 3, 0))
        field_shades = expected_shades(trace[1:])
        bgp = field_shades[0] * 5 + field_shades[1] * 80
        self.assertEqual(gb.read(symbols["wShBGP"], 4), bytes((bgp, 0x99, 0, 0)))
        expected = bytes(channel for y in range(144) for x in range(160)
                         for channel in GREY[field_shades[region_of(trace[3], x, y)]])
        self.assertEqual(field, expected)
        self.assertNotEqual(field, before)
        self.assertNotEqual(field, diagnostic)
        gb.write(symbols["wSignedFreeze"], b"\0")
        if role == "operator":
          for _ in range(3):
            self.pulse(gb, "right")
          self.wait(gb, lambda: self.value(gb, symbols, "wShowMode") == 2
                    and self.value(gb, symbols, "wWarnVis") == 1, "visible warning")
          gb.write(symbols["wSignedFreeze"], b"\1")
          warning = frame()
          self.assertEqual(self.value(gb, symbols, "wShownShow"), 2)
          self.assertEqual(gb.read(symbols["wShBGP"], 4), bytes((0x3F, 0x89, 0, 144)))
          self.assertEqual(warning, expected_warning_frame())
          self.assertNotEqual(warning, before)
          self.assertNotEqual(warning, diagnostic)
          self.assertNotEqual(warning, field)
          gb.write(symbols["wSignedRequest"], b"\1")
          self.wait(gb, lambda: self.value(gb, symbols, "wSignedRefused") == 2, "warning refusal")
          self.assertEqual(self.value(gb, symbols, "wSignedActive"), 0)
          self.assertEqual(self.value(gb, symbols, "wSignedRequest"), 0)
          self.assertEqual(self.value(gb, symbols, "wShowMode"), 2)
          self.assertEqual(self.value(gb, symbols, "wWarnVis"), 1)
          self.assertEqual(self.value(gb, symbols, "wLatch"), 0)
          self.assertEqual(frame(), warning)
          gb.write(symbols["wSignedFreeze"], b"\0")
          self.pulse(gb, "up")  # WarnStep cancels; it does not change the ring.
          self.assertEqual(self.value(gb, symbols, "wShown7"), 1)
        self.native(gb, symbols)
        self.assertEqual(self.value(gb, symbols, "wSignedRequest"), 0)
        self.assertEqual(self.value(gb, symbols, "wSignedActive"), 0)
        gb.write(symbols["wSignedFreeze"], b"\1")
        frame()
        self.assertEqual(self.value(gb, symbols, "wShLCDC"), 0x91)
        gb.write(symbols["wSignedInspect"], b"\1")
        self.wait(gb, lambda: self.value(gb, symbols, "wSignedDone") == 1, "VRAM inspection")
        self.assertEqual(gb.read(symbols["wSignedAfter"], 3200), baseline)


if __name__ == "__main__":
  unittest.main()
