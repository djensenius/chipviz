"""Build a test-only SMS ROM around the callable F18 fixture."""

import argparse
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

import sms_trace_tool as trace


ROOT = Path(__file__).resolve().parents[1]
HOME = ROOT / "cores/sms/homebrew"
SOURCES = ("f18_fixture_rom.asm", "data_control.asm", "data_input.asm", "spectral_levels.asm",
           "safe_modes.asm", "data_detector.asm", "detector_fixture.asm", "field_data.asm")


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--wla", required=True)
  parser.add_argument("--linker", required=True)
  parser.add_argument("--output-dir", required=True, type=Path)
  args = parser.parse_args()
  wla, linker = shutil.which(args.wla), shutil.which(args.linker)
  if not wla or not linker:
    parser.error("both wla-z80 and wlalink are required")
  args.output_dir.mkdir(parents=True, exist_ok=False)
  with tempfile.TemporaryDirectory(prefix="f18-build-", dir=args.output_dir) as directory:
    work = Path(directory)
    for source in SOURCES:
      shutil.copyfile(HOME / source, work / source)
    subprocess.run([sys.executable, "-B", str(HOME / "field_assets.py"), str(work)], check=True)
    (work / "f18.input.bin").write_bytes(trace.inputs("f18"))
    (work / "f18.link").write_text("[objects]\nf18.o\n", encoding="ascii")
    subprocess.run([str(Path(wla).resolve()), "-o", "f18.o", "f18_fixture_rom.asm"], cwd=work, check=True)
    subprocess.run([str(Path(linker).resolve()), "-r", "-S", "f18.link", "f18.sms"], cwd=work, check=True)
    rom = bytearray((work / "f18.sms").read_bytes())
    if len(rom) != 0x8000 or rom[0x7ff0:] != bytes([0xa5] * 16):
      raise ValueError("fixture overlaps header or is not 32 KiB")
    rom[0x7ff0:] = bytes(16)
    rom[0x7ff0:0x7ff8] = b"TMR SEGA"
    rom[0x7fff] = 0x4c
    rom[0x7ffa:0x7ffc] = (sum(rom[:0x7ff0]) & 0xffff).to_bytes(2, "little")
    (args.output_dir / "f18.sms").write_bytes(rom)
    shutil.copyfile(work / "f18.sym", args.output_dir / "f18.sym")
  print(f"Built test-only F18 cartridge and symbols -> {args.output_dir}")


if __name__ == "__main__":
  main()
