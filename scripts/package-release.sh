#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dist="$root/dist"
assets="$dist/assets"

rm -rf "$dist"
mkdir -p "$assets"

cd "$root"
mise exec -- just check

mkdir -p build/release
python3 host/bridge/chipviz_bridge.py --frames 120 --output "$assets/control-frame-v0-demo.cvz"
python3 host/bridge/chipviz_encode.py --music host/fixtures/musical/scale.musicsource.json --output "$assets/scale.cvz"
python3 host/bridge/chipviz_encode.py --chipsynth-log host/fixtures/chipsynth/groove.csv0 --output "$assets/chipsynth-groove.cvz"
python3 host/bridge/n64_joybus.py --input "$assets/scale.cvz" --output "$assets/scale.n64joybus"
python3 host/bridge/usb_hid.py --input "$assets/scale.cvz" --output "$assets/scale-usb-hid.json"
python3 shared/tools/cvz_to_c.py --input "$assets/scale.cvz" --symbol chipviz_scale_frames --output "$assets/scale_frames.h"
python3 shared/tools/build_homebrew.py --output "$assets"

if command -v gbafix >/dev/null 2>&1 && [ -n "${DEVKITARM:-}" ]; then
  make -C cores/gba/homebrew
  cp cores/gba/homebrew/chipviz-gba.gba "$assets/"
else
  echo "Skipping GBA SDK build; install devkitPro/devkitARM and tools-gba for chipviz-gba.gba"
fi

if [ -n "${N64_INST:-}" ] && [ -f "$N64_INST/include/n64.mk" ]; then
  make -C cores/n64/homebrew
  cp cores/n64/homebrew/chipviz-n64.z64 "$assets/"
else
  echo "Skipping N64 SDK build; install libdragon and set N64_INST for chipviz-n64.z64"
fi

if command -v cl65 >/dev/null 2>&1; then
  make -C cores/c64/homebrew
  cp cores/c64/homebrew/chipviz-c64.prg "$assets/"
else
  echo "Keeping generated C64 fallback; install cc65/cl65 for SDK-built chipviz-c64.prg"
fi

if [ -n "${PVSNESLIB_HOME:-}" ] && [ -f "$PVSNESLIB_HOME/devkitsnes/snes_rules" ]; then
  make -C cores/snes/homebrew
  cp cores/snes/homebrew/chipviz-snes.sfc "$assets/"
else
  echo "Keeping generated SNES fallback; install PVSnesLib and set PVSNESLIB_HOME for SDK-built chipviz-snes.sfc"
fi

cargo build --manifest-path renderers/modern/Cargo.toml --release --bins
cp renderers/modern/target/release/chipviz-pi5 "$assets/"
cp renderers/modern/target/release/chipviz-m1 "$assets/"

python3 - <<'PY'
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

assets = Path("dist/assets")
with ZipFile("dist/chipviz-assets.zip", "w", ZIP_DEFLATED) as archive:
  for path in sorted(assets.iterdir()):
    archive.write(path, path.name)
PY

echo "release assets written to $dist"
