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
    self.assertIn(bytes([build_homebrew.TOKEN_REM]), prg)
    self.assertIn(b"CHIPVIZ C64 HOMEBREW", prg)
    self.assertIn(bytes([build_homebrew.TOKEN_AND]), prg)
    self.assertIn(b"C,(C-55296) ", prg)
    self.assertIn(b" 15", prg)
    self.assertIn(b"1024+W*40+X,160+W", prg)
    self.assertIn(b"55296+W*40+X,1+W", prg)
    self.assertIn(bytes([build_homebrew.TOKEN_NEXT]) + b" X:" + bytes([build_homebrew.TOKEN_NEXT]) + b" W", prg)
    self.assertNotIn(b"C,I", prg)
    self.assertNotIn(b"+I", prg)
    self.assertTrue(prg.endswith(b"\x00\x00"))

  def test_snes_rom_has_lorom_header_and_reset_vector(self) -> None:
    rom = build_homebrew.build_snes_sfc()

    self.assertEqual(len(rom), 0x8000)
    self.assertEqual(rom[0x7FC0:0x7FC0 + 16].rstrip(), b"CHIPVIZ HOMEBREW")
    self.assertEqual(rom[0x7FD5], 0x20)
    self.assertEqual(rom[0x7FD7], 0x05)
    self.assertEqual(int.from_bytes(rom[0x7FFC:0x7FFE], "little"), 0x8000)
    self.assertIn(bytes([0xAD, 0x12, 0x42, 0x10, 0xFB]), rom[:96])
    self.assertIn(bytes([0xEE, 0x00, 0x00]), rom[:96])
    self.assertIn(bytes([0x8D, 0x22, 0x21]), rom[:96])

  def test_build_writes_artifacts_and_status(self) -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
      output = pathlib.Path(temp_dir)
      build_homebrew.build(output)
      status = json.loads((output / "homebrew-status.json").read_text(encoding="utf-8"))

      self.assertTrue((output / "chipviz-c64.prg").exists())
      self.assertTrue((output / "chipviz-snes.sfc").exists())
      self.assertTrue((output / "chipviz-genesis.md").exists())
      self.assertTrue((output / "chipviz-nes.nes").exists())
      self.assertTrue((output / "chipviz-sms.sms").exists())
      self.assertIs(status["generated"]["c64Fallback"]["hardwareBootable"], True)
      self.assertIs(status["generated"]["snesFallback"]["hardwareBootable"], True)
      self.assertIs(status["generated"]["genesisFallback"]["hardwareBootable"], True)
      self.assertIs(status["generated"]["nesFallback"]["hardwareBootable"], True)
      self.assertIs(status["generated"]["smsFallback"]["hardwareBootable"], True)
      self.assertEqual(status["generated"]["c64Fallback"]["status"], "tokenized-basic-vic-ii-petscii-fallback")
      self.assertEqual(status["generated"]["snesFallback"]["status"], "visible-lorom-cgram-color-cycle-fallback")
      self.assertEqual(status["generated"]["genesisFallback"]["chipSource"], "YM2612 plus SN76489 from chipsynth")
      self.assertEqual(status["generated"]["nesFallback"]["chipSource"], "NES 2A03 from chipsynth")
      self.assertEqual(status["generated"]["smsFallback"]["chipSource"], "SN76489 from chipsynth")
      self.assertIn("gb", status["sdkRequired"])
      self.assertIn("gba", status["sdkRequired"])
      self.assertIn("n64", status["sdkRequired"])
      self.assertIn("c64", status["sdkRequired"])
      self.assertIn("snes", status["sdkRequired"])
      self.assertIn("genesis", status["sdkRequired"])
      self.assertIn("nes", status["sdkRequired"])
      self.assertIn("sms", status["sdkRequired"])

  def test_targeted_build_writes_matching_status(self) -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
      output = pathlib.Path(temp_dir)
      build_homebrew.build(output, targets={"genesis"})
      status = json.loads((output / "homebrew-status.json").read_text(encoding="utf-8"))

      self.assertTrue((output / "chipviz-genesis.md").exists())
      self.assertFalse((output / "chipviz-c64.prg").exists())
      self.assertEqual(set(status["generated"]), {"genesisFallback"})
      self.assertEqual(status["generated"]["genesisFallback"]["artifact"], "chipviz-genesis.md")

  def test_generated_genesis_rom_has_header_and_reset_vector(self) -> None:
    rom = build_homebrew.build_genesis_md()

    self.assertEqual(len(rom), 0x40000)
    self.assertEqual(int.from_bytes(rom[4:8], "big"), 0x00000200)
    self.assertEqual(rom[0x100:0x110].rstrip(), b"SEGA MEGA DRIVE")
    self.assertIn(b"CHIPVIZ GENESIS", rom[0x120:0x150])
    self.assertIn(bytes([0x33, 0xFC, 0x81, 0x74]), rom[0x200:0x240])
    checksum = 0
    for offset in range(0x200, len(rom), 2):
      checksum = (checksum + int.from_bytes(rom[offset:offset + 2], "big")) & 0xFFFF
    self.assertEqual(int.from_bytes(rom[0x18E:0x190], "big"), checksum)

  def test_generated_nes_rom_has_ines_header_and_vectors(self) -> None:
    rom = build_homebrew.build_nes_nrom()

    self.assertEqual(rom[:4], b"NES\x1a")
    self.assertEqual(rom[4], 2)
    self.assertEqual(rom[5], 1)
    self.assertEqual(len(rom), 16 + 0x8000 + 0x2000)
    prg = rom[16:16 + 0x8000]
    self.assertEqual(int.from_bytes(prg[-4:-2], "little"), 0x8000)
    self.assertIn(bytes([0x8D, 0x00, 0x20]), prg[:96])

  def test_generated_sms_rom_has_header(self) -> None:
    rom = build_homebrew.build_sms_rom()

    self.assertEqual(len(rom), 0x8000)
    self.assertEqual(rom[0x7FF0:0x7FF8], b"TMR SEGA")
    self.assertIn(bytes([0xD3, 0xBF]), rom[:48])

  def test_sdk_homebrew_projects_are_present(self) -> None:
    self.assertTrue((ROOT / "cores" / "c64" / "homebrew" / "Makefile").exists())
    self.assertTrue((ROOT / "cores" / "c64" / "homebrew" / "src" / "main.c").exists())
    self.assertTrue((ROOT / "cores" / "snes" / "homebrew" / "Makefile").exists())
    self.assertTrue((ROOT / "cores" / "snes" / "homebrew" / "chipviz-snes.c").exists())
    self.assertTrue((ROOT / "cores" / "gb" / "homebrew" / "Makefile").exists())
    self.assertTrue((ROOT / "cores" / "gb" / "homebrew" / "main.asm").exists())
    self.assertTrue((ROOT / "cores" / "genesis" / "homebrew" / "Makefile").exists())
    self.assertTrue((ROOT / "cores" / "nes" / "homebrew" / "Makefile").exists())
    self.assertTrue((ROOT / "cores" / "nes" / "homebrew" / "src" / "main.s").exists())
    self.assertTrue((ROOT / "cores" / "sms" / "homebrew" / "Makefile").exists())


if __name__ == "__main__":
  unittest.main()
