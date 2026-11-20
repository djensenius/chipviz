#!/usr/bin/env python3
"""Make an opt-in, local-only starfield bundle from user-supplied files."""

from __future__ import annotations

import argparse
import contextlib
import gzip
import os
import pathlib
import shutil
import stat
import sys
import zlib
from typing import BinaryIO


ROOT = pathlib.Path(__file__).resolve().parents[1]
BUILD = ROOT / "build"
DEFAULT_OUTPUT = BUILD / "personal-starfield"
DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
MAX_CATALOG_BYTES = 64 * 1024 * 1024

CATALOG_NAME = "hyg-v4.4.csv"
BINARY_NAME = "chipviz-renderer"
NOTICE_NAME = "HYG-NOTICE.txt"
LAUNCHER_NAME = "launch-starfield"

NOTICE = """\
LOCAL PERSONAL/NONCOMMERCIAL USE ONLY -- NOT FOR REDISTRIBUTION

Keep this bundle local. Do not upload, publish, redistribute, or use it
commercially. Commercial licensees must supply their own catalog.

HYG Database, astronexus, https://codeberg.org/astronexus/hyg
HYG v4.4 source:
https://codeberg.org/astronexus/hyg/media/branch/main/data/hyg/CURRENT/hyg_v44.csv.gz
Upstream LFS metadata for the v4.4 gzip (not verified by this tool):
13636362 bytes; SHA-256
00b349893b9a53106dd488d8371e8d2fa586043e500bb3cdb8bff3931682197d
The CURRENT URL can change; this hash describes the compressed bytes only.
Upstream offers HYG under CC BY-SA 4.0:
https://creativecommons.org/licenses/by-sa/4.0/

HYG includes data derived from the ESA Hipparcos mission. Hipparcos has an
ESA CC BY-NC 3.0 IGO provenance/licensing caveat. Credit: ESA
https://www.cosmos.esa.int/web/esdc/terms-and-conditions

HYG's stated licence does not resolve all upstream terms, provenance, or
commercial/public redistribution rights. Verify the terms for your local
personal use before using the data. The local-only restriction is a
precaution, not a claim that CC BY-SA itself prohibits commercial use.

The supplied CSV is kept separate from the renderer and is not modified
except for decompression, if needed. This tool does not verify its version,
origin, contents, or the completeness of upstream copyright and attribution
notices. Retain all notices supplied with your catalog; pass any local
upstream notice files to this tool with --upstream-notice.
"""

LAUNCHER = """\
#!/bin/sh
set -eu

script_path=$0
case $script_path in
  */*) ;;
  *)
    if [ -f "./$script_path" ]; then
      script_path=./$script_path
    else
      script_path=$(command -v "$script_path") || exit 1
    fi
    ;;
esac
if [ -L "$script_path" ]; then
  printf '%s\\n' 'Run the launcher directly, not through a symlink.' >&2
  exit 1
fi
launcher_dir=${script_path%/*}
[ -n "$launcher_dir" ] || launcher_dir=/
case $launcher_dir in
  /*) ;;
  *) launcher_dir=./$launcher_dir ;;
esac
bundle_dir=$(CDPATH='' cd -P "$launcher_dir" && printf '%s/.' "$PWD")
bundle_dir=${bundle_dir%/.}
if [ -L "$bundle_dir/chipviz-renderer" ] || [ -L "$bundle_dir/hyg-v4.4.csv" ]; then
  printf '%s\\n' 'Bundle binary and catalog must not be symlinks.' >&2
  exit 1
fi
exec "$bundle_dir/chipviz-renderer" --scene 6 --star-catalog "$bundle_dir/hyg-v4.4.csv" "$@"
"""


class BundleError(Exception):
  pass


def open_input(path: pathlib.Path, label: str, *, executable: bool = False) -> BinaryIO:
  """Open a regular input without following a final symlink or blocking on a FIFO."""
  try:
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
      raise BundleError(f"{label} must be a regular file, not a symlink: {path}")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
      opened = os.fstat(fd)
      if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
        raise BundleError(f"{label} changed while opening: {path}")
      if not stat.S_ISREG(opened.st_mode):
        raise BundleError(f"{label} must be a regular file: {path}")
      if executable and (not opened.st_mode & 0o111 or not opened.st_size):
        raise BundleError(f"{label} must be a nonempty executable file: {path}")
      return os.fdopen(fd, "rb")
    except BaseException:
      os.close(fd)
      raise
  except OSError as error:
    raise BundleError(f"cannot read {label} {path}: {error.strerror}") from error


def checked_output(path: pathlib.Path) -> pathlib.Path:
  if ".." in path.parts:
    raise BundleError("output path must not contain '..'")
  if path.is_absolute() and path.anchor != "/":
    raise BundleError("output path must use a single leading slash")
  if not path.is_absolute():
    path = pathlib.Path.cwd() / path
  if any(part.casefold() in {".git", "dist", "release", "releases"} for part in path.parts):
    raise BundleError("output must not be in a Git metadata, dist, or release path")
  if BUILD not in path.parents:
    raise BundleError("output must be below this checkout's ignored build/ directory")
  return path


def reject_other_checkout(fd: int, path: pathlib.Path) -> None:
  try:
    os.stat(".git", dir_fd=fd, follow_symlinks=False)
  except FileNotFoundError:
    return
  if path != ROOT:
    raise BundleError(f"output must not be inside another Git checkout: {path}")


def open_output_directory(path: pathlib.Path) -> int:
  """Open the trusted destination through directory descriptors, without symlinks."""
  fd = os.open("/", DIRECTORY_FLAGS)
  current = pathlib.Path("/")
  try:
    reject_other_checkout(fd, current)
    for part in path.parts[1:]:
      current /= part
      try:
        next_fd = os.open(part, DIRECTORY_FLAGS, dir_fd=fd)
      except OSError as error:
        raise BundleError(
          f"output directory and ancestors must exist and contain no symlinks: "
          f"{current} ({error.strerror})"
        ) from error
      os.close(fd)
      fd = next_fd
      reject_other_checkout(fd, current)
    info = os.fstat(fd)
    if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) & 0o077:
      raise BundleError("output directory must be owned by this user and private (mkdir -m 700)")
    if os.listdir(fd):
      raise BundleError("output directory must be empty; existing files were not changed")
    return fd
  except BaseException:
    os.close(fd)
    raise


def create_file(fd: int, name: str, mode: int) -> BinaryIO:
  try:
    file_fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                      mode, dir_fd=fd)
  except FileExistsError as error:
    raise BundleError(f"refusing to overwrite destination entry: {name}") from error
  try:
    return os.fdopen(file_fd, "wb")
  except BaseException:
    os.close(file_fd)
    raise


def copy_catalog(source: BinaryIO, dest: BinaryIO, limit: int) -> None:
  """Copy at most limit bytes and detect the first byte beyond it."""
  total = 0
  while True:
    remaining = limit - total
    data = source.read(min(64 * 1024, remaining + 1))
    if not data:
      if total == 0:
        raise BundleError("catalog is empty after decompression")
      return
    if len(data) > remaining:
      dest.write(data[:remaining])
      raise BundleError(f"catalog exceeds {limit} bytes; use a smaller catalog")
    dest.write(data)
    total += len(data)


def create_bundle(catalog_path: pathlib.Path, binary_path: pathlib.Path,
                  output_path: pathlib.Path, notice_paths: list[pathlib.Path]) -> pathlib.Path:
  output = checked_output(output_path)
  filename = catalog_path.name.lower()
  compressed = filename.endswith(".csv.gz")
  if not compressed and not filename.endswith(".csv"):
    raise BundleError("catalog filename must end in .csv or .csv.gz")

  with contextlib.ExitStack() as stack:
    catalog = stack.enter_context(open_input(catalog_path, "catalog"))
    binary = stack.enter_context(open_input(binary_path, "renderer", executable=True))
    notices = [stack.enter_context(open_input(path, "upstream notice")) for path in notice_paths]
    magic = catalog.read(2)
    catalog.seek(0)
    if not magic:
      raise BundleError("catalog is empty")
    if compressed and magic != b"\x1f\x8b":
      raise BundleError("invalid gzip catalog")
    if not compressed and magic == b"\x1f\x8b":
      raise BundleError("gzip catalog filename must end in .csv.gz")

    directory_fd = open_output_directory(output)
    try:
      try:
        with create_file(directory_fd, CATALOG_NAME, 0o600) as dest:
          try:
            if compressed:
              with gzip.GzipFile(fileobj=catalog, mode="rb") as source:
                copy_catalog(source, dest, MAX_CATALOG_BYTES)
            else:
              copy_catalog(catalog, dest, MAX_CATALOG_BYTES)
          except (gzip.BadGzipFile, EOFError, zlib.error) as error:
            raise BundleError(f"invalid gzip catalog: {error}") from error

        with create_file(directory_fd, BINARY_NAME, 0o700) as dest:
          shutil.copyfileobj(binary, dest)

        notice_names = []
        for index, source in enumerate(notices, start=1):
          name = f"HYG-UPSTREAM-NOTICE-{index}.txt"
          with create_file(directory_fd, name, 0o600) as dest:
            shutil.copyfileobj(source, dest)
          notice_names.append(name)

        if notice_names:
          additional = "\nUser-supplied upstream notices (not verified by this tool):\n"
          additional += "".join(f"- {name}\n" for name in notice_names)
        else:
          additional = "\nNo additional upstream notice files were supplied to this tool.\n"
        with create_file(directory_fd, NOTICE_NAME, 0o600) as dest:
          dest.write((NOTICE + additional).encode("utf-8"))

        with create_file(directory_fd, LAUNCHER_NAME, 0o700) as dest:
          if stat.S_IMODE(os.fstat(dest.fileno()).st_mode) != 0o700:
            raise BundleError("launcher mode was restricted by umask; set umask 077 and retry "
                              "in another empty directory")
          dest.write(LAUNCHER.encode("utf-8"))
      except BaseException:
        print(f"Bundle incomplete: partial files may remain in the opened directory {output} "
              "(or wherever it was moved). No files were removed; inspect before retrying.",
              file=sys.stderr)
        raise
    finally:
      os.close(directory_fd)
  return output


def main(argv: list[str] | None = None) -> int:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--catalog", type=pathlib.Path, required=True,
                      help="locally supplied HYG v4.4 .csv or .csv.gz")
  parser.add_argument("--binary", type=pathlib.Path, required=True,
                      help="already-built modern renderer executable")
  parser.add_argument("--output", type=pathlib.Path, default=DEFAULT_OUTPUT,
                      help=f"existing empty private directory under build/ (default: {DEFAULT_OUTPUT})")
  parser.add_argument("--upstream-notice", type=pathlib.Path, action="append", default=[],
                      help="local upstream attribution/copyright file; may be repeated")
  args = parser.parse_args(argv)
  try:
    output = create_bundle(args.catalog, args.binary, args.output, args.upstream_notice)
  except (BundleError, OSError) as error:
    print(f"error: {error}", file=sys.stderr)
    return 1
  except KeyboardInterrupt:
    print("error: interrupted", file=sys.stderr)
    return 130
  print(f"Created LOCAL-ONLY personal/noncommercial bundle: {output}")
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
