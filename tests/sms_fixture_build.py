"""Build a test-only callable Z80 image and symbols for an emulator adapter."""

import argparse
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
HOME = ROOT / "cores/sms/homebrew"
SOURCES = ("fixture.asm", "data_control.asm", "data_input.asm", "spectral_levels.asm",
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
  wla, linker = str(Path(wla).resolve()), str(Path(linker).resolve())
  args.output_dir.mkdir(parents=True, exist_ok=True)
  with tempfile.TemporaryDirectory(prefix="sms-fixture-", dir=args.output_dir) as directory:
    work = Path(directory)
    for source in SOURCES:
      shutil.copyfile(HOME / source, work / source)
    subprocess.run([sys.executable, "-B", str(HOME / "field_assets.py"), str(work)], check=True)
    (work / "fixture.link").write_text("[objects]\nfixture.o\n", encoding="ascii")
    subprocess.run([wla, "-o", "fixture.o", "fixture.asm"], cwd=work, check=True)
    subprocess.run([linker, "-r", "-S", "fixture.link", "fixture.bin"], cwd=work, check=True)
    for name in ("fixture.bin", "fixture.sym"):
      shutil.copyfile(work / name, args.output_dir / name)
  print("Built callable SMS fixture and symbols (not a bootable ROM)")


if __name__ == "__main__":
  main()
