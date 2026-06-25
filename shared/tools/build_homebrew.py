#!/usr/bin/env python3
"""Build minimal homebrew artifacts that do not require proprietary blobs."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

TARGETS = ("c64", "snes", "genesis", "nes", "sms")

TOKEN_FOR = 0x81
TOKEN_NEXT = 0x82
TOKEN_GOTO = 0x89
TOKEN_PRINT = 0x99
TOKEN_POKE = 0x97
TOKEN_TO = 0xA4
TOKEN_REM = 0x8F
TOKEN_STEP = 0xA9
TOKEN_AND = 0xAF


def ascii_bytes(text: str) -> bytes:
  return text.upper().encode("ascii", "replace")


def basic_line(line_number: int, payload: bytes) -> bytes:
  return line_number.to_bytes(2, "little") + payload + b"\x00"


def build_c64_prg() -> bytes:
  lines = [
    basic_line(10, bytes([TOKEN_PRINT]) + b' "' + bytes([147, 5]) + ascii_bytes("CHIPVIZ C64 HOMEBREW") + b'"'),
    basic_line(20, bytes([TOKEN_PRINT]) + b' "' + ascii_bytes("RASTER PETSCII DEMO SEED") + b'"'),
    basic_line(30, bytes([TOKEN_PRINT]) + b' "' + ascii_bytes("COLOR BARS + VIC BORDER PULSE") + b'"'),
    basic_line(35, bytes([TOKEN_REM]) + b" " + ascii_bytes("CHIPVIZ HOME BREW STYLE")),
    basic_line(
      40,
      bytes([TOKEN_FOR])
      + b" I=0 "
      + bytes([TOKEN_TO])
      + b" 15:"
      + bytes([TOKEN_POKE])
      + b" 53280,I:"
      + bytes([TOKEN_POKE])
      + b" 53281,15-I:"
      + bytes([TOKEN_NEXT]),
    ),
    basic_line(
      45,
      bytes([TOKEN_FOR])
      + b" P=1024 "
      + bytes([TOKEN_TO])
      + b" 1063:"
      + bytes([TOKEN_POKE])
      + b" P,81:"
      + bytes([TOKEN_NEXT]),
    ),
    basic_line(
     50,
     bytes([TOKEN_FOR])
     + b" C=55296 "
     + bytes([TOKEN_TO])
     + b" 55335:"
     + bytes([TOKEN_POKE])
     + b" C,(C-55296) "
     + bytes([TOKEN_AND])
     + b" 15:"
     + bytes([TOKEN_NEXT]),
    ),
    basic_line(
     55,
     bytes([TOKEN_FOR])
     + b" W=0 "
     + bytes([TOKEN_TO])
     + b" 7:"
     + bytes([TOKEN_FOR])
     + b" X=0 "
     + bytes([TOKEN_TO])
     + b" 7:"
     + bytes([TOKEN_POKE])
     + b" 1024+W*40+X,160+W:"
     + bytes([TOKEN_POKE])
     + b" 55296+W*40+X,1+W:"
     + bytes([TOKEN_NEXT])
     + b" X:"
     + bytes([TOKEN_NEXT])
     + b" W",
    ),
    basic_line(60, bytes([TOKEN_GOTO]) + b" 40"),
  ]
  program = bytearray()
  load_address = 0x0801
  next_address = load_address
  for line in lines:
    next_address += 2 + len(line)
    program += next_address.to_bytes(2, "little")
    program += line
  program += b"\x00\x00"
  return load_address.to_bytes(2, "little") + bytes(program)


def build_snes_sfc() -> bytes:
  rom = bytearray([0x00] * 0x8000)
  code = bytes(
    [
      0x78,  # sei
      0x18,  # clc
      0xFB,  # xce
      0xC2, 0x30,  # rep #$30
      0xA2, 0xFF, 0x1F,  # ldx #$1fff
      0x9A,  # txs
      0xE2, 0x20,  # sep #$20
      0xA9, 0x00,  # lda #$00
      0x8D, 0x00, 0x21,  # sta $2100 (screen off)
      0x8D, 0x00, 0x00,  # sta $0000 (frame counter)
      0x8D, 0x01, 0x00,  # sta $0001
      0xA9, 0x0F,  # lda #$0f
      0x8D, 0x00, 0x21,  # sta $2100 (full brightness)
      0xAD, 0x12, 0x42,  # lda $4212
      0x30, 0xFB,  # bmi (wait until vblank clears)
      0xAD, 0x12, 0x42,  # lda $4212
      0x10, 0xFB,  # bpl (wait for vblank)
      0xEE, 0x00, 0x00,  # inc $0000
      0xA9, 0x00,  # lda #$00
      0x8D, 0x21, 0x21,  # sta $2121 (CGRAM color 0)
      0xAD, 0x00, 0x00,  # lda $0000
      0x29, 0x1F,  # and #$1f
      0x8D, 0x02, 0x00,  # sta $0002
      0xAD, 0x00, 0x00,  # lda $0000
      0x0A,  # asl
      0x0A,  # asl
      0x29, 0xE0,  # and #$e0
      0x0D, 0x02, 0x00,  # ora $0002
      0x8D, 0x22, 0x21,  # sta $2122
      0xAD, 0x00, 0x00,  # lda $0000
      0x4A,  # lsr
      0x4A,  # lsr
      0x29, 0x7C,  # and #$7c
      0x8D, 0x22, 0x21,  # sta $2122
      0x80, 0xCD,  # bra main loop
    ]
  )
  rom[0:len(code)] = code
  title = b"CHIPVIZ HOMEBREW".ljust(21, b" ")
  header = 0x7FC0
  rom[header:header + 21] = title
  rom[0x7FD5] = 0x20  # LoROM, slow
  rom[0x7FD6] = 0x00  # ROM only
  rom[0x7FD7] = 0x05  # 32 KiB ROM size code
  rom[0x7FD8] = 0x00  # no SRAM
  rom[0x7FD9] = 0x01  # USA
  rom[0x7FDA] = 0x33  # developer/homebrew marker
  rom[0x7FDB] = 0x00
  # Native and emulation reset vectors point to $8000.
  for offset in (0x7FFC, 0x7FFE):
    rom[offset:offset + 2] = (0x8000).to_bytes(2, "little")
  checksum = sum(rom) & 0xFFFF
  complement = checksum ^ 0xFFFF
  rom[0x7FDC:0x7FDE] = complement.to_bytes(2, "little")
  rom[0x7FDE:0x7FE0] = checksum.to_bytes(2, "little")
  return bytes(rom)


def build_genesis_md() -> bytes:
  rom = bytearray([0x00] * 0x40000)
  rom[0:4] = (0x00FF0000).to_bytes(4, "big")
  rom[4:8] = (0x00000200).to_bytes(4, "big")

  def put(offset: int, text: str, size: int) -> None:
    rom[offset:offset + size] = text.encode("ascii", "replace")[:size].ljust(size, b" ")

  put(0x100, "SEGA MEGA DRIVE ", 16)
  put(0x110, "(C)CHIPVIZ 2026.JUN", 16)
  put(0x120, "CHIPVIZ GENESIS YM2612 PSG DEMO", 48)
  put(0x150, "CHIPVIZ GENESIS YM2612 PSG DEMO", 48)
  put(0x180, "GM CHIPVIZ-00", 14)
  rom[0x18E:0x190] = b"\x00\x00"
  put(0x190, "J               ", 16)
  rom[0x1A0:0x1A4] = (0x00000000).to_bytes(4, "big")
  rom[0x1A4:0x1A8] = (len(rom) - 1).to_bytes(4, "big")
  put(0x1F0, "JUE             ", 16)

  code = bytes(
    [
      0x33, 0xFC, 0x80, 0x04, 0x00, 0xC0, 0x00, 0x04,  # move.w #$8004,$c00004
      0x33, 0xFC, 0x81, 0x74, 0x00, 0xC0, 0x00, 0x04,  # move.w #$8174,$c00004
      0x33, 0xFC, 0x82, 0x30, 0x00, 0xC0, 0x00, 0x04,  # plane A nametable
      0x33, 0xFC, 0x84, 0x07, 0x00, 0xC0, 0x00, 0x04,  # plane B nametable
      0x33, 0xFC, 0x87, 0x00, 0x00, 0xC0, 0x00, 0x04,  # backdrop color 0
      0x23, 0xFC, 0xC0, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x04,  # CRAM write
      0x33, 0xFC, 0x00, 0xEE, 0x00, 0xC0, 0x00, 0x00,  # white backdrop
      0x33, 0xFC, 0x02, 0x2E, 0x00, 0xC0, 0x00, 0x00,  # cyan/pink accent
      0x33, 0xFC, 0x0E, 0x20, 0x00, 0xC0, 0x00, 0x00,  # green accent
      0x60, 0xFE,  # bra.s *
    ]
  )
  rom[0x200:0x200 + len(code)] = code
  checksum = sum(rom[0x200:]) & 0xFFFF
  rom[0x18E:0x190] = checksum.to_bytes(2, "big")
  return bytes(rom)


def build_sms_rom() -> bytes:
  rom = bytearray([0x00] * 0x8000)
  code = bytes(
    [
      0xF3,  # di
      0x31, 0xF0, 0xDF,  # ld sp,$dff0
      0x3E, 0x04, 0xD3, 0xBF, 0x3E, 0x80, 0xD3, 0xBF,  # reg0
      0x3E, 0xE0, 0xD3, 0xBF, 0x3E, 0x81, 0xD3, 0xBF,  # reg1 display
      0x3E, 0xFF, 0xD3, 0xBF, 0x3E, 0x82, 0xD3, 0xBF,  # nametable
      0x3E, 0x00, 0xD3, 0xBF, 0x3E, 0xC0, 0xD3, 0xBF,  # CRAM addr 0
      0x3E, 0x3F, 0xD3, 0xBE,  # white
      0x3E, 0x03, 0xD3, 0xBE,  # red
      0x3E, 0x0C, 0xD3, 0xBE,  # green
      0x3E, 0x30, 0xD3, 0xBE,  # blue
      0x76,  # halt
      0x18, 0xFD,  # jr halt
    ]
  )
  rom[:len(code)] = code
  header = 0x7FF0
  rom[header:header + 8] = b"TMR SEGA"
  rom[0x7FF8:0x7FFA] = b"\x00\x00"
  rom[0x7FFA:0x7FFC] = b"\x00\x00"
  rom[0x7FFC:0x7FFE] = b"\x00\x4A"
  checksum = sum(rom[:0x7FF0]) & 0xFFFF
  rom[0x7FFA:0x7FFC] = checksum.to_bytes(2, "little")
  return bytes(rom)


def build_nes_nrom() -> bytes:
  prg = bytearray([0xEA] * 0x8000)
  chr_rom = bytearray([0x00] * 0x2000)
  reset_code = bytes(
    [
      0x78, 0xD8, 0xA2, 0x40, 0x8E, 0x17, 0x40, 0xA2, 0xFF, 0x9A, 0xE8,
      0x8E, 0x00, 0x20, 0x8E, 0x01, 0x20, 0x8E, 0x10, 0x40, 0xAD, 0x02,
      0x20, 0xAD, 0x02, 0x20, 0x10, 0xFB, 0xA9, 0x3F, 0x8D, 0x06, 0x20,
      0xA9, 0x00, 0x8D, 0x06, 0x20, 0xA2, 0x00, 0xBD, 0x40, 0x80, 0x8D,
      0x07, 0x20, 0xE8, 0xE0, 0x20, 0xD0, 0xF5, 0xA9, 0x80, 0x8D, 0x00,
      0x20, 0xA9, 0x1E, 0x8D, 0x01, 0x20, 0x4C, 0x3D, 0x80,
    ]
  )
  palettes = bytes(
    [
      0x0F, 0x11, 0x21, 0x31, 0x0F, 0x05, 0x15, 0x25,
      0x0F, 0x09, 0x19, 0x29, 0x0F, 0x0C, 0x1C, 0x2C,
      0x0F, 0x10, 0x20, 0x30, 0x0F, 0x06, 0x16, 0x26,
      0x0F, 0x0A, 0x1A, 0x2A, 0x0F, 0x0F, 0x12, 0x30,
    ]
  )
  prg[:len(reset_code)] = reset_code
  prg[0x40:0x40 + len(palettes)] = palettes
  prg[-6:] = (0x8000).to_bytes(2, "little") + (0x8000).to_bytes(2, "little") + (0x8000).to_bytes(2, "little")
  for tile in range(256):
    base = tile * 16
    for row in range(8):
      chr_rom[base + row] = ((0x81 >> (row & 3)) | (tile & 0x55)) & 0xFF
      chr_rom[base + 8 + row] = ((0x18 << (row & 1)) | (tile & 0xAA)) & 0xFF
  return b"NES\x1a" + bytes([2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]) + bytes(prg) + bytes(chr_rom)


def build_status() -> dict:
  return {
    "generated": {
      "c64Fallback": {
        "artifact": "chipviz-c64.prg",
        "status": "tokenized-basic-vic-ii-petscii-fallback",
        "hardwareBootable": True,
      },
      "snesFallback": {
        "artifact": "chipviz-snes.sfc",
        "status": "visible-lorom-cgram-color-cycle-fallback",
        "hardwareBootable": True,
        "validation": "emulator/flashcart/Pocket-core dependent",
      },
      "genesisFallback": {
        "artifact": "chipviz-genesis.md",
        "status": "headered-68000-vdp-cram-color-seed",
        "hardwareBootable": True,
        "chipSource": "YM2612 plus SN76489 from chipsynth",
      },
      "nesFallback": {
        "artifact": "chipviz-nes.nes",
        "status": "ines-nrom-ppu-palette-seed",
        "hardwareBootable": True,
        "chipSource": "NES 2A03 from chipsynth",
      },
      "smsFallback": {
        "artifact": "chipviz-sms.sms",
        "status": "headered-z80-vdp-cram-color-seed",
        "hardwareBootable": True,
        "chipSource": "SN76489 from chipsynth",
      },
    },
    "sdkRequired": {
      "gba": "Hardware-bootable .gba requires a valid GBA header/logo generated by a GBA homebrew SDK; this repo will not embed proprietary boot logo bytes by hand.",
      "gb": "Hardware-bootable .gb requires RGBDS/rgbfix to generate the required Game Boy header/logo; this repo will not embed proprietary boot logo bytes by hand.",
      "n64": "Hardware-bootable .z64 requires libdragon/OpenLibDragon boot/IPL/CIC handling; use the SDK build path rather than a fake ROM blob.",
      "c64": "Complete C64 release artifacts build from cores/c64/homebrew with cc65/cl65; generated BASIC remains a fallback.",
      "snes": "Complete SNES release artifacts build from cores/snes/homebrew with PVSnesLib; generated LoROM remains a fallback.",
      "genesis": "Complete Genesis release artifacts should move to SGDK or a selected 68000/Z80 SDK; generated ROM remains a visible VDP fallback.",
      "nes": "Complete NES release artifacts build from cores/nes/homebrew with ca65/ld65; generated iNES remains a fallback.",
      "sms": "Complete SMS release artifacts should move to devkitSMS/SDCC or a selected Z80 SDK; generated ROM remains a visible VDP fallback.",
    },
  }


def build(output: Path, targets: set[str] | None = None) -> None:
  selected = set(TARGETS if targets is None else targets)
  output.mkdir(parents=True, exist_ok=True)
  if "c64" in selected:
    (output / "chipviz-c64.prg").write_bytes(build_c64_prg())
  if "snes" in selected:
    (output / "chipviz-snes.sfc").write_bytes(build_snes_sfc())
  if "genesis" in selected:
    (output / "chipviz-genesis.md").write_bytes(build_genesis_md())
  if "nes" in selected:
    (output / "chipviz-nes.nes").write_bytes(build_nes_nrom())
  if "sms" in selected:
    (output / "chipviz-sms.sms").write_bytes(build_sms_rom())
  (output / "homebrew-status.json").write_text(
    json.dumps(build_status(), indent=2) + "\n",
    encoding="utf-8",
  )


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--output", type=Path, required=True)
  parser.add_argument(
    "--target",
    action="append",
    choices=TARGETS,
    help="Generate only the selected target. May be provided multiple times.",
  )
  return parser


def main() -> int:
  args = build_parser().parse_args()
  build(args.output, set(args.target) if args.target else None)
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
