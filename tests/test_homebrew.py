import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock


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
      self.assertEqual(set(status["sdkRequired"]), {"genesis"})
      self.assertEqual(status["generated"]["genesisFallback"]["artifact"], "chipviz-genesis.md")

  def test_generated_genesis_rom_has_header_and_reset_vector(self) -> None:
    rom = build_homebrew.build_genesis_md()

    self.assertEqual(len(rom), 0x40000)
    self.assertEqual(int.from_bytes(rom[:4], "big"), 0x00FF0000)
    self.assertEqual(int.from_bytes(rom[4:8], "big"), 0x00000200)
    self.assertEqual(rom[0x100:0x110].rstrip(), b"SEGA MEGA DRIVE")
    self.assertIn(b"CHIPVIZ GENESIS", rom[0x120:0x150])
    self.assertEqual(int.from_bytes(rom[0x1A0:0x1A4], "big"), 0)
    self.assertEqual(int.from_bytes(rom[0x1A4:0x1A8], "big"), len(rom) - 1)
    self.assertEqual(rom[0x1F0:0x200].rstrip(), b"JUE")
    checksum = 0
    for offset in range(0x200, len(rom), 2):
      checksum = (checksum + int.from_bytes(rom[offset:offset + 2], "big")) & 0xFFFF
    self.assertEqual(int.from_bytes(rom[0x18E:0x190], "big"), checksum)

  def test_generated_genesis_tmss_precedes_vdp_access(self) -> None:
    rom = build_homebrew.build_genesis_md()
    pc = int.from_bytes(rom[4:8], "big")

    # SGDK src/boot/sega.s: low version nibble at $A10001, skip on zero.
    # https://github.com/Stephane-D/SGDK/tree/9a78a44143a867cd8c5053b95cd061a21f0b73c2
    self.assertEqual(rom[pc:pc + 6], bytes.fromhex("1039 00a1 0001"))  # move.b abs.l,d0
    pc += 6
    self.assertEqual(rom[pc:pc + 4], bytes.fromhex("0200 000f"))  # andi.b #$0f,d0
    pc += 4
    self.assertEqual(rom[pc], 0x67)  # BEQ: Z set by ANDI, not region bits.
    # M68000 PRM (1992), Bcc: signed byte displacement relative to opcode+2.
    displacement = int.from_bytes(rom[pc + 1:pc + 2], "big", signed=True)
    target = pc + 2 + displacement
    pc += 2
    self.assertEqual(rom[pc:pc + 10], bytes.fromhex("23fc 5345 4741 00a1 4000"))
    pc += 10  # move.l #$53454741,$a14000 ("SEGA"), only for nonzero version.
    self.assertEqual(target, pc)  # Skip exactly the entire TMSS write.
    self.assertEqual(pc, 0x216)
    self.assertEqual(rom[pc:pc + 8], bytes.fromhex("33fc 8004 00c0 0004"))

  def test_generated_genesis_sequential_palette_and_static_loop(self) -> None:
    rom = build_homebrew.build_genesis_md()
    # Both TMSS paths converge here; the preceding test checks reachability.
    pc = int.from_bytes(rom[4:8], "big") + 22

    # Sega Retro VDP registers: reg $0F increments the byte address after
    # each data-port access; two bytes per color, set BEFORE the CRAM transfer.
    # https://segaretro.org/index.php?title=Sega_Mega_Drive/VDP_registers&oldid=743918
    for register_word in (0x8004, 0x8174, 0x8230, 0x8407, 0x8700, 0x8F02):
      with self.subTest(register=hex(register_word)):
        self.assertEqual(int.from_bytes(rom[pc:pc + 2], "big"), 0x33FC)
        self.assertEqual(int.from_bytes(rom[pc + 2:pc + 4], "big"), register_word)
        self.assertEqual(int.from_bytes(rom[pc + 4:pc + 8], "big"), 0xC00004)
      pc += 8
    # Long control-port command selects CRAM byte address zero.
    self.assertEqual(rom[pc:pc + 10], bytes.fromhex("23fc c000 0000 00c0 0004"))
    pc += 10
    # SGDK inc/pal.h: xxxxBBBxGGGxRRRx; white backdrop and unchanged accents.
    # Consecutive word writes now reach CRAM byte addresses 0, 2 and 4.
    for rgb in ((7, 7, 7), (7, 1, 1), (0, 1, 7)):
      with self.subTest(rgb=rgb):
        self.assertEqual(int.from_bytes(rom[pc:pc + 2], "big"), 0x33FC)
        color = int.from_bytes(rom[pc + 2:pc + 4], "big")
        self.assertEqual(color & ~0x0EEE, 0)
        self.assertEqual(((color >> 1) & 7, (color >> 5) & 7, (color >> 9) & 7), rgb)
        self.assertEqual(int.from_bytes(rom[pc + 4:pc + 8], "big"), 0xC00000)
      pc += 8
    self.assertEqual(rom[pc:pc + 2], bytes.fromhex("60fe"))  # BRA.S, not animation.
    self.assertEqual(pc + 2 + int.from_bytes(rom[pc + 1:pc + 2], "big", signed=True), pc)

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

    # https://segaretro.org/ROM_header#Mark_III/Master_System_and_Game_Gear
    self.assertEqual(len(rom), 0x8000)
    self.assertEqual(rom[0x7FF0:0x7FF8], b"TMR SEGA")
    self.assertIn(bytes([0xD3, 0xBF]), rom[:48])
    with self.subTest(field="region"):
      self.assertEqual(rom[0x7FFF] >> 4, 4)  # Export Master System.
    with self.subTest(field="size"):
      self.assertEqual(rom[0x7FFF] & 0x0F, 0xC)  # 32 KiB.
    with self.subTest(field="product/revision"):
      self.assertEqual(rom[0x7FFC:0x7FFF], b"\x00\x00\x00")

  def test_generated_sms_checksum_excludes_entire_header(self) -> None:
    rom = build_homebrew.build_sms_rom()

    # devkitSMS tools/makesms/src/makesms.c: sum bytes before 0x7FF0,
    # store low byte at header+10, high byte at header+11.
    # https://github.com/sverx/devkitSMS/tree/533ae572c897cf44f1da865013ebf690134301a3
    checksum = (sum(rom) - sum(rom[0x7FF0:0x8000])) % 65536
    self.assertEqual(rom[0x7FFA:0x7FFC], checksum.to_bytes(2, "little"))
    self.assertEqual(rom[0x7FFA:0x7FFC], b"\xDE\x1E")  # Static seed body.

  def test_generated_sms_checksum_covers_padding_and_wraps(self) -> None:
    # Nonzero fill exposes truncated coverage hidden by the seed's zero padding.
    # It also exercises 16-bit overflow and nonzero excluded header bytes.
    filled = bytearray([0xFF] * 0x8000)
    with mock.patch.object(build_homebrew, "bytearray", return_value=filled, create=True):
      rom = build_homebrew.build_sms_rom()

    self.assertEqual(rom[0x7FEF], 0xFF)
    checksum = sum(rom) - sum(rom[0x7FF0:0x8000])
    self.assertGreater(checksum, 0xFFFF)
    self.assertEqual(rom[0x7FFA:0x7FFC], (checksum % 65536).to_bytes(2, "little"))

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
