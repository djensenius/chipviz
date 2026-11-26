"""Build the unverified SMS candidate with independently supplied WLA-DX tools."""

import argparse
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


HOME = Path(__file__).resolve().parent
SOURCES = ("candidate.asm", "data_control.asm", "data_input.asm", "data_detector.asm",
           "spectral_levels.asm", "safe_modes.asm", "field_data.asm")


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--wla", required=True, help="path to wla-z80")
  parser.add_argument("--linker", required=True, help="path to wlalink")
  parser.add_argument("--output", required=True, type=Path)
  parser.add_argument("--symbols", type=Path, help="optional WLA symbol file")
  args = parser.parse_args()
  wla = shutil.which(args.wla)
  linker = shutil.which(args.linker)
  if not wla or not linker:
    parser.error("both wla-z80 and wlalink are required for the candidate; use make for the seed")
  wla, linker = str(Path(wla).resolve()), str(Path(linker).resolve())
  output = args.output.resolve()
  output.parent.mkdir(parents=True, exist_ok=True)
  with tempfile.TemporaryDirectory(prefix="sms-candidate-", dir=output.parent) as temp:
    work = Path(temp)
    for source in SOURCES:
      shutil.copyfile(HOME / source, work / source)
    subprocess.run([sys.executable, "-B", str(HOME / "field_assets.py"), str(work)], check=True)
    (work / "candidate.link").write_text("[objects]\ncandidate.o\n", encoding="ascii")
    subprocess.run([wla, "-o", "candidate.o", "candidate.asm"], cwd=work, check=True)
    subprocess.run([linker, "-r", "-S", "candidate.link", "candidate.sms"], cwd=work, check=True)
    rom = bytearray((work / "candidate.sms").read_bytes())
    if len(rom) != 0x8000 or rom[0x7ff0:] != bytes([0xa5] * 16):
      raise ValueError("candidate overlaps the SMS header or is not 32 KiB")
    rom[0x7ff0:] = bytes(16)
    rom[0x7ff0:0x7ff8] = b"TMR SEGA"
    rom[0x7fff] = 0x4c
    rom[0x7ffa:0x7ffc] = (sum(rom[:0x7ff0]) & 0xffff).to_bytes(2, "little")
    output.write_bytes(rom)
    if args.symbols:
      shutil.copyfile(work / "candidate.sym", args.symbols)
  print(f"Built unverified SMS candidate: {output}")


if __name__ == "__main__":
  main()
