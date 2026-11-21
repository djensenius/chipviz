#!/usr/bin/env python3
"""Create a local-only, headless macOS ARM64 renderer archive.

An archive and its included notices do not grant redistribution clearance.
Header checks do not prove runtime compatibility, rendering, signing, or notarization.
Use an existing empty, user-owned private directory beneath a private,
user-owned build/macos-arm64/ directory.
"""

from __future__ import annotations

import argparse
import contextlib
import os
import pathlib
import shutil
import stat
import struct
import sys
import zipfile
from typing import BinaryIO


ROOT = pathlib.Path(__file__).resolve().parents[1]
DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
NOTICES = ("LICENSE", "COMMERCIAL.md", "THIRD_PARTY_NOTICES/rust.md")
MAX_COMMAND_BYTES = 16 * 1024 * 1024
MAX_COMMANDS = 65536


class PackageError(Exception):
  pass


def absolute_path(path: pathlib.Path) -> pathlib.Path:
  if ".." in path.parts or (path.is_absolute() and path.anchor != "/"):
    raise PackageError(f"unsafe path: {path}")
  return path if path.is_absolute() else pathlib.Path.cwd() / path


def open_directory(path: pathlib.Path, *, root: pathlib.Path | None = None) -> int:
  """Traverse existing directories without following symlinks."""
  fd = os.open("/", DIRECTORY_FLAGS)
  current = pathlib.Path("/")
  try:
    for part in path.parts[1:]:
      current /= part
      next_fd = os.open(part, DIRECTORY_FLAGS, dir_fd=fd)
      os.close(fd)
      fd = next_fd
      if root is not None and current == root / "build":
        info = os.fstat(fd)
        if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) & 0o022:
          raise PackageError("build directory must be user-owned and not group/world-writable")
      if root is not None and current == root / "build" / "macos-arm64":
        info = os.fstat(fd)
        if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) & 0o077:
          raise PackageError("build/macos-arm64 must be user-owned and private (mkdir -m 700)")
      if root is not None and current != root and root in current.parents:
        try:
          os.stat(".git", dir_fd=fd, follow_symlinks=False)
        except FileNotFoundError:
          pass
        else:
          raise PackageError(f"output must not be in another checkout: {current}")
    return fd
  except BaseException:
    os.close(fd)
    raise


def open_input(path: pathlib.Path) -> BinaryIO:
  path = absolute_path(path)
  directory_fd = open_directory(path.parent)
  try:
    fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                 dir_fd=directory_fd)
  finally:
    os.close(directory_fd)
  try:
    if not stat.S_ISREG(os.fstat(fd).st_mode):
      raise PackageError(f"input must be a regular file, not a symlink: {path}")
    return os.fdopen(fd, "rb")
  except BaseException:
    os.close(fd)
    raise


def validate_macho(binary: BinaryIO) -> None:
  """Inspect bounded 64-bit load commands, not runtime or signature validity."""
  def read_exact(length: int) -> bytes:
    data = binary.read(length)
    if len(data) != length:
      raise PackageError("truncated Mach-O load command")
    return data

  size = os.fstat(binary.fileno()).st_size
  header = binary.read(32)
  if len(header) != 32:
    raise PackageError("truncated Mach-O header")
  magic, cpu, _subtype, filetype, count, command_bytes, _flags, _reserved = struct.unpack(
    "<8I", header,
  )
  if magic != 0xfeedfacf or cpu != 0x0100000c or filetype != 2:
    raise PackageError("expected a thin little-endian ARM64 Mach-O executable")
  if (count > MAX_COMMANDS or command_bytes > MAX_COMMAND_BYTES
      or command_bytes > size - 32 or count > command_bytes // 8):
    raise PackageError("invalid or oversized Mach-O load commands")
  end = 32 + command_bytes
  macos = False
  for _ in range(count):
    position = binary.tell()
    if end - position < 8:
      raise PackageError("truncated Mach-O load command")
    command, length = struct.unpack("<II", read_exact(8))
    if length < 8 or length % 8 or length > end - position:
      raise PackageError("invalid Mach-O load command size")
    if command == 0x32:  # LC_BUILD_VERSION
      if length < 24:
        raise PackageError("truncated LC_BUILD_VERSION")
      platform, minos, _sdk, tools = struct.unpack("<4I", read_exact(16))
      if length != 24 + tools * 8 or platform != 1 or minos == 0:
        raise PackageError("invalid or non-macOS LC_BUILD_VERSION")
      macos = True
    elif command == 0x24:  # LC_VERSION_MIN_MACOSX
      if length != 16:
        raise PackageError("invalid LC_VERSION_MIN_MACOSX")
      version, _sdk = struct.unpack("<II", read_exact(8))
      if not version:
        raise PackageError("invalid LC_VERSION_MIN_MACOSX version")
      macos = True
    elif command in (0x25, 0x2f, 0x30):  # iOS, tvOS, watchOS legacy commands
      raise PackageError("non-macOS platform load command")
    binary.seek(position + length)
  if binary.tell() != end or not macos:
    raise PackageError("missing macOS platform or inconsistent load commands")
  binary.seek(0)


def create_package(binary_path: pathlib.Path, output_path: pathlib.Path,
                   *, root: pathlib.Path = ROOT) -> pathlib.Path:
  """The root parameter is an internal test seam, not a CLI option."""
  root = absolute_path(root)
  output = absolute_path(output_path)
  base = root / "build" / "macos-arm64"
  if base not in output.parent.parents:
    raise PackageError("output must be in a private directory beneath build/macos-arm64/")

  with contextlib.ExitStack() as stack:
    binary = stack.enter_context(open_input(binary_path))
    notices = [stack.enter_context(open_input(root / name)) for name in NOTICES]
    validate_macho(binary)
    directory_fd = open_directory(output.parent, root=root)
    try:
      info = os.fstat(directory_fd)
      if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) & 0o077:
        raise PackageError("output directory must be user-owned and private (mkdir -m 700)")
      if os.listdir(directory_fd):
        raise PackageError("output directory must be empty; existing files were not changed")
      fd = os.open(output.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                   0o600, dir_fd=directory_fd)
      try:
        with os.fdopen(fd, "wb") as target:
          with zipfile.ZipFile(target, "w", compression=zipfile.ZIP_DEFLATED,
                               allowZip64=True) as archive:
            for name, source, mode in [
              ("chipviz-m1", binary, 0o755),
              *[(name, source, 0o644) for name, source in zip(NOTICES, notices)],
            ]:
              entry = zipfile.ZipInfo(name)
              entry.create_system = 3
              entry.external_attr = (stat.S_IFREG | mode) << 16
              entry.compress_type = zipfile.ZIP_DEFLATED
              with archive.open(entry, "w", force_zip64=True) as dest:
                shutil.copyfileobj(source, dest, length=64 * 1024)
      except BaseException:
        print("Archive may be incomplete; no files were removed. Inspect before retrying.",
              file=sys.stderr)
        raise
    finally:
      os.close(directory_fd)
  return output


def main(argv: list[str] | None = None) -> int:
  parser = argparse.ArgumentParser(description=__doc__,
                                   formatter_class=argparse.RawDescriptionHelpFormatter)
  parser.add_argument("--binary", type=pathlib.Path, required=True,
                      help="existing thin macOS ARM64 Mach-O executable")
  parser.add_argument("--output", type=pathlib.Path, required=True,
                      help="new archive in an existing empty private build/macos-arm64/ directory")
  args = parser.parse_args(argv)
  try:
    output = create_package(args.binary, args.output)
  except (PackageError, OSError, zipfile.BadZipFile) as error:
    print(f"error: {error}", file=sys.stderr)
    return 1
  except KeyboardInterrupt:
    print("error: interrupted", file=sys.stderr)
    return 130
  print(f"Created local-only archive: {output}")
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
