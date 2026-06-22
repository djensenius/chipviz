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
