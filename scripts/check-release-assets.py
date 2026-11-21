#!/usr/bin/env python3
"""Allow only known release asset names in the asset directory and combined zip.

The opt-in renderer check only identifies ELF headers; it does not establish
Linux ABI, executability or startup, provenance, native Pi/Mac or hardware
support, or safety.
"""

import argparse
import pathlib
import struct
import sys
import zipfile


RENDERER_NAMES = {"chipviz-pi5", "chipviz-m1"}
ELF64_HEADER_SIZE = 64
ET_EXEC, ET_DYN = 2, 3
EM_X86_64 = 62
EV_CURRENT = 1

# New release asset names require an explicit policy review; #109 tracks SNES.
ASSET_NAMES = {
  "LICENSE", "COMMERCIAL.md", "control-frame-v0-demo.cvz", "scale.cvz",
  "chipsynth-groove.cvz", "scale.n64joybus", "scale-usb-hid.json",
  "scale_frames.h", "homebrew-status.json", "chipviz-c64.prg",
  "chipviz-gba.gba", "chipviz-n64.z64", "chipviz-gb.gb",
  "chipviz-genesis.md", "chipviz-nes.nes", "chipviz-sms.sms",
} | RENDERER_NAMES


def forbidden(name: str) -> bool:
  name = name.replace("\\", "/").rsplit("/", 1)[-1].lower()
  return name.startswith("chipviz-snes") or pathlib.PurePath(name).suffix in {
    ".sfc", ".smc", ".fig", ".swc",
  }


def valid_x86_64_renderer_header(path: pathlib.Path) -> bool:
  with path.open("rb") as renderer:
    header = renderer.read(ELF64_HEADER_SIZE)
  # ELF magic, ELFCLASS64, ELFDATA2LSB, and the current identification version.
  if len(header) != ELF64_HEADER_SIZE or header[:7] != b"\x7fELF\x02\x01\x01":
    return False
  # ELF64 e_type, e_machine, e_version, and e_ehsize.
  object_type, machine, version = struct.unpack_from("<HHI", header, 16)
  (header_size,) = struct.unpack_from("<H", header, 52)
  return (object_type in (ET_EXEC, ET_DYN) and machine == EM_X86_64
          and version == EV_CURRENT and header_size == ELF64_HEADER_SIZE)


def main() -> int:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument(
    "--require-x86-64-renderers", action="store_true",
    help="require both renderer assets with ELF64 little-endian x86-64 ET_EXEC/ET_DYN headers",
  )
  parser.add_argument("assets_dir", type=pathlib.Path)
  parser.add_argument("combined_zip", type=pathlib.Path)
  args = parser.parse_args()
  assets, archive = args.assets_dir, args.combined_zip
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
    if args.require_x86_64_renderers:
      missing = sorted(RENDERER_NAMES - asset_names)
      if missing:
        print(f"Missing renderer assets: {', '.join(missing)}", file=sys.stderr)
        return 1
      for name in sorted(RENDERER_NAMES):
        if not valid_x86_64_renderer_header(assets / name):
          print(f"Invalid ELF64 little-endian x86-64 executable/shared-object header: {name}", file=sys.stderr)
          return 1
  except (OSError, EOFError, RuntimeError, zipfile.BadZipFile) as error:
    print(f"Cannot inspect release assets: {error}", file=sys.stderr)
    return 1
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
