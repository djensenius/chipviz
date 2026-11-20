#!/usr/bin/env python3
"""Allow only known release asset names in the asset directory and combined zip."""

import pathlib
import sys
import zipfile


# New release asset names require an explicit policy review; #109 tracks SNES.
ASSET_NAMES = {
  "LICENSE", "COMMERCIAL.md", "control-frame-v0-demo.cvz", "scale.cvz",
  "chipsynth-groove.cvz", "scale.n64joybus", "scale-usb-hid.json",
  "scale_frames.h", "homebrew-status.json", "chipviz-c64.prg",
  "chipviz-gba.gba", "chipviz-n64.z64", "chipviz-gb.gb",
  "chipviz-genesis.md", "chipviz-nes.nes", "chipviz-sms.sms",
  "chipviz-pi5", "chipviz-m1",
}


def forbidden(name: str) -> bool:
  name = name.replace("\\", "/").rsplit("/", 1)[-1].lower()
  return name.startswith("chipviz-snes") or pathlib.PurePath(name).suffix in {
    ".sfc", ".smc", ".fig", ".swc",
  }


def main() -> int:
  if len(sys.argv) != 3:
    print("usage: check-release-assets.py ASSETS_DIR COMBINED_ZIP", file=sys.stderr)
    return 2
  assets, archive = map(pathlib.Path, sys.argv[1:])
  if not assets.is_dir():
    print(f"Missing release assets directory: {assets}", file=sys.stderr)
    return 1
  try:
    entries = list(assets.iterdir())
    if any(not path.is_file() or path.is_symlink() for path in entries):
      print("Release assets must be regular files, not directories or symlinks", file=sys.stderr)
      return 1
    asset_names = {path.name for path in entries}
    notices = pathlib.Path(__file__).resolve().parents[1] / "THIRD_PARTY_NOTICES"
    allowed = ASSET_NAMES | {
      f"THIRD_PARTY_NOTICES-{path.name}"
      for path in notices.iterdir() if path.is_file()
    }
    with zipfile.ZipFile(archive) as zipped:
      infos = zipped.infolist()
      zip_names = [info.filename for info in infos]
      names = asset_names | set(zip_names)
      blocked = sorted(name for name in names if forbidden(name))
      if blocked:
        print(f"SNES release assets are withheld (#109): {', '.join(blocked)}", file=sys.stderr)
        return 1
      unexpected = sorted(names - allowed)
      if unexpected:
        print(f"Unexpected release assets: {', '.join(unexpected)}", file=sys.stderr)
        return 1
      if len(zip_names) != len(set(zip_names)):
        print("Duplicate release archive entries", file=sys.stderr)
        return 1
      if asset_names != set(zip_names):
        print("Release archive entries do not match individual assets", file=sys.stderr)
        return 1
      for info in infos:
        with zipped.open(info) as archived, (assets / info.filename).open("rb") as individual:
          while True:
            chunk = archived.read(1024 * 1024)
            if chunk != individual.read(1024 * 1024):
              print(f"Release archive content differs from individual asset: {info.filename}", file=sys.stderr)
              return 1
            if not chunk:
              break
  except (OSError, EOFError, RuntimeError, zipfile.BadZipFile) as error:
    print(f"Cannot inspect release assets: {error}", file=sys.stderr)
    return 1
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
