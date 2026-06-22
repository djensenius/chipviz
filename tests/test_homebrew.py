import json
import pathlib
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "shared" / "tools"))

import build_homebrew


class HomebrewArtifactTests(unittest.TestCase):
  def test_c64_prg_has_basic_load_address(self) -> None:
    prg = build_homebrew.build_c64_prg()

    self.assertEqual(prg[:2], b"\x01\x08")
    self.assertIn(bytes([build_homebrew.TOKEN_PRINT]), prg)
    self.assertIn(bytes([build_homebrew.TOKEN_FOR]), prg)
    self.assertIn(bytes([build_homebrew.TOKEN_POKE]), prg)
    self.assertIn(b"CHIPVIZ C64 HOMEBREW", prg)
    self.assertTrue(prg.endswith(b"\x00\x00"))

  def test_snes_rom_has_lorom_header_and_reset_vector(self) -> None:
    rom = build_homebrew.build_snes_sfc()

    self.assertEqual(len(rom), 0x8000)
    self.assertEqual(rom[0x7FC0:0x7FC0 + 16].rstrip(), b"CHIPVIZ HOMEBREW")
    self.assertEqual(rom[0x7FD5], 0x20)
    self.assertEqual(rom[0x7FD7], 0x05)
    self.assertEqual(int.from_bytes(rom[0x7FFC:0x7FFE], "little"), 0x8000)
    self.assertIn(bytes([0x8D, 0x22, 0x21]), rom[:64])

  def test_build_writes_artifacts_and_status(self) -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
      output = pathlib.Path(temp_dir)
      build_homebrew.build(output)
      status = json.loads((output / "homebrew-status.json").read_text(encoding="utf-8"))

      self.assertTrue((output / "chipviz-c64.prg").exists())
      self.assertTrue((output / "chipviz-snes.sfc").exists())
      self.assertIs(status["generated"]["c64"]["hardwareBootable"], True)
      self.assertIs(status["generated"]["snes"]["hardwareBootable"], True)
      self.assertEqual(status["generated"]["c64"]["status"], "tokenized-basic-homebrew-prg")
      self.assertIn("gba", status["sdkRequired"])
      self.assertIn("n64", status["sdkRequired"])


if __name__ == "__main__":
  unittest.main()
