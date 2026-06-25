#!/usr/bin/env python3
"""Launch built ROMs through RetroArch briefly and write verbose logs."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path


TESTS = [
  ("gba", "mgba_libretro.dylib", "cores/gba/homebrew/chipviz-gba.gba"),
  ("gb", "sameboy_libretro.dylib", "cores/gb/homebrew/chipviz-gb.gb"),
  ("n64", "mupen64plus_next_libretro.dylib", "cores/n64/homebrew/chipviz-n64.z64"),
  ("snes", "snes9x_libretro.dylib", "cores/snes/homebrew/chipviz-snes.sfc"),
  ("c64", "vice_x64sc_libretro.dylib", str(Path("cores/c64/homebrew/chipviz-c64.prg").resolve())),
  ("genesis", "genesis_plus_gx_libretro.dylib", "cores/genesis/homebrew/chipviz-genesis.md"),
  ("nes", "mesen_libretro.dylib", "cores/nes/homebrew/chipviz-nes.nes"),
  ("sms", "genesis_plus_gx_libretro.dylib", "cores/sms/homebrew/chipviz-sms.sms"),
]


def main() -> int:
  retroarch = Path(os.environ.get("RETROARCH_BIN", "/Applications/RetroArch.app/Contents/MacOS/RetroArch"))
  cores = Path(os.environ.get("RETROARCH_CORES", str(Path.home() / "Library/Application Support/RetroArch/cores")))
  logdir = Path("build/retroarch-smoke")
  logdir.mkdir(parents=True, exist_ok=True)

  for name, core, rom in TESTS:
    log = logdir / f"{name}.log"
    print(f"=== {name} ===")
    if not Path(rom).exists():
      print(f"missing rom: {rom}")
      continue

    with log.open("wb") as output:
      process = subprocess.Popen(
        [str(retroarch), "-L", str(cores / core), rom, "--verbose"],
        stdout=output,
        stderr=subprocess.STDOUT,
      )
      try:
        process.wait(timeout=8)
        print(f"exited early rc={process.returncode}")
      except subprocess.TimeoutExpired:
        process.terminate()
        try:
          process.wait(timeout=3)
        except subprocess.TimeoutExpired:
          process.kill()
          process.wait(timeout=3)
        print("launched for 8s")

  print(f"logs: {logdir}")
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
