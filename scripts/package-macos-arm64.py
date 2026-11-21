#!/usr/bin/env python3
"""Create a local-only, headless macOS ARM64 renderer archive.

An archive and its included notices do not grant redistribution clearance.
Header checks do not prove runtime compatibility, rendering, signing, or notarization.
Use an existing empty, user-owned private directory beneath a private,
user-owned build/macos-arm64/ directory.
The optional preparation mode creates these directories without following symlinks.
The directory and source checks cannot prevent races by the same account.
On failure, an incomplete archive may remain; no output files are removed.
Snapshots stay in memory: at most 256 MiB for the binary and 8 MiB per notice.
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import io
import os
import pathlib
import pwd
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
MAX_BINARY_BYTES = 256 * 1024 * 1024
MAX_NOTICE_BYTES = 8 * 1024 * 1024


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


def snapshot_input(source: BinaryIO, label: str, limit: int,
                   *, executable: bool = False) -> tuple[io.BytesIO, int]:
  """Copy an input to bounded memory, rejecting detectable concurrent changes."""
  before = os.fstat(source.fileno())
  mode = stat.S_IMODE(before.st_mode)
  if executable and (not mode & 0o100 or mode & 0o7000):
    raise PackageError("binary must be owner-executable and have no special permission bits")
  if before.st_size > limit:
    raise PackageError(f"{label} exceeds the {limit}-byte snapshot limit")
  snapshot = io.BytesIO()
  remaining = before.st_size
  while remaining:
    data = source.read(min(64 * 1024, remaining))
    if not data:
      raise PackageError(f"{label} changed while taking snapshot")
    snapshot.write(data)
    remaining -= len(data)
  after = os.fstat(source.fileno())
  fields = ("st_dev", "st_ino", "st_size", "st_mode", "st_mtime_ns", "st_ctime_ns")
  if any(getattr(before, field) != getattr(after, field) for field in fields):
    raise PackageError(f"{label} changed while taking snapshot")
  snapshot.seek(0)
  return snapshot, mode


def validate_macho(binary: BinaryIO) -> None:
  """Inspect bounded 64-bit load commands, not runtime or signature validity."""
  def read_exact(length: int) -> bytes:
    data = binary.read(length)
    if len(data) != length:
      raise PackageError("truncated Mach-O load command")
    return data

  size = binary.seek(0, os.SEEK_END)
  binary.seek(0)
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
  declaration = None
  build_digest = None
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
      platform, minos, sdk, tools = struct.unpack("<4I", read_exact(16))
      if length != 24 + tools * 8 or platform != 1 or minos == 0:
        raise PackageError("invalid or non-macOS LC_BUILD_VERSION")
      digest = hashlib.sha256()
      remaining = tools * 8
      while remaining:
        data = read_exact(min(64 * 1024, remaining))
        digest.update(data)
        remaining -= len(data)
      if build_digest is not None and build_digest != digest.digest():
        raise PackageError("conflicting LC_BUILD_VERSION tool declarations")
      build_digest = digest.digest()
      current = (minos, sdk)
    elif command == 0x24:  # LC_VERSION_MIN_MACOSX
      if length != 16:
        raise PackageError("invalid LC_VERSION_MIN_MACOSX")
      version, sdk = struct.unpack("<II", read_exact(8))
      if not version:
        raise PackageError("invalid LC_VERSION_MIN_MACOSX version")
      current = (version, sdk)
    elif command in (0x25, 0x2f, 0x30):  # iOS, tvOS, watchOS legacy commands
      raise PackageError("non-macOS platform load command")
    else:
      current = None
    if current is not None:
      if declaration is not None and declaration != current:
        raise PackageError("conflicting macOS platform declarations")
      declaration = current
    binary.seek(position + length)
  if binary.tell() != end or declaration is None:
    raise PackageError("missing macOS platform or inconsistent load commands")
  binary.seek(0)


def verify_destination(output: pathlib.Path, root: pathlib.Path, directory_fd: int,
                       archive_fd: int | None = None) -> None:
  """Reopen the path and compare identities; this is not same-user isolation."""
  check_fd = open_directory(output.parent, root=root)
  try:
    def identity(info: os.stat_result) -> tuple[int, int]:
      return info.st_dev, info.st_ino

    if identity(os.fstat(check_fd)) != identity(os.fstat(directory_fd)):
      raise PackageError("output directory changed during packaging")
    if archive_fd is not None:
      info = os.stat(output.name, dir_fd=check_fd, follow_symlinks=False)
      if not stat.S_ISREG(info.st_mode) or identity(info) != identity(os.fstat(archive_fd)):
        raise PackageError("output archive changed during packaging")
  finally:
    os.close(check_fd)


def prepare_output_directory(path: pathlib.Path, *, root: pathlib.Path = ROOT) -> pathlib.Path:
  """Create a fresh private output directory using no-follow directory descriptors."""
  root = absolute_path(root)
  path = absolute_path(path)
  if path.parent != root / "build" / "macos-arm64":
    raise PackageError("new directory must be directly beneath build/macos-arm64/")
  fd = open_directory(root)
  try:
    for name in ("build", "macos-arm64"):
      try:
        os.mkdir(name, 0o700, dir_fd=fd)
      except FileExistsError:
        pass
      next_fd = os.open(name, DIRECTORY_FLAGS, dir_fd=fd)
      os.close(fd)
      fd = next_fd
      info = os.fstat(fd)
      forbidden = 0o022 if name == "build" else 0o077
      if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) & forbidden:
        raise PackageError(f"{name} must be user-owned and have safe permissions")
      try:
        os.stat(".git", dir_fd=fd, follow_symlinks=False)
      except FileNotFoundError:
        pass
      else:
        raise PackageError(f"{name} must not be another checkout")
    verify_destination(path, root, fd)
    os.mkdir(path.name, 0o700, dir_fd=fd)
    child_fd = os.open(path.name, DIRECTORY_FLAGS, dir_fd=fd)
    try:
      info = os.fstat(child_fd)
      if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) & 0o077:
        raise PackageError("new output directory must be user-owned and private")
      verify_destination(path / "unused", root, child_fd)
    finally:
      os.close(child_fd)
  finally:
    os.close(fd)
  return path


def check_cargo_home(path: pathlib.Path, *, account_home: pathlib.Path | None = None,
                     root: pathlib.Path = ROOT) -> None:
  """Fail closed on unsafe Cargo cache routing; this does not sandbox Cargo."""
  if account_home is None:
    account_home = pathlib.Path(pwd.getpwuid(os.geteuid()).pw_dir)
  path = absolute_path(path)
  expected = absolute_path(account_home) / ".cargo"
  if path != expected or any(part.casefold() in {"dist", "personal-starfield"}
                             for part in path.parts):
    raise PackageError("Cargo home must be the account's own ~/.cargo, outside protected paths")
  fd = open_directory(path)
  try:
    def safe(info: os.stat_result) -> bool:
      return info.st_uid == os.geteuid() and not stat.S_IMODE(info.st_mode) & 0o022

    def contains(ancestor_fd: int, descendant_fd: int) -> bool:
      ancestor = os.fstat(ancestor_fd)
      current_fd = os.dup(descendant_fd)
      try:
        for _ in range(256):
          current = os.fstat(current_fd)
          if (current.st_dev, current.st_ino) == (ancestor.st_dev, ancestor.st_ino):
            return True
          parent_fd = os.open("..", DIRECTORY_FLAGS, dir_fd=current_fd)
          try:
            parent = os.fstat(parent_fd)
          except BaseException:
            os.close(parent_fd)
            raise
          if (current.st_dev, current.st_ino) == (parent.st_dev, parent.st_ino):
            os.close(parent_fd)
            return False
          os.close(current_fd)
          current_fd = parent_fd
        raise PackageError("directory ancestry is too deep")
      finally:
        os.close(current_fd)

    root_fd = open_directory(absolute_path(root))
    try:
      if contains(fd, root_fd) or contains(root_fd, fd):
        raise PackageError("Cargo home and checkout must not overlap")
    finally:
      os.close(root_fd)

    home_fd = open_directory(path.parent)
    try:
      if not safe(os.fstat(home_fd)):
        raise PackageError("account home must be user-owned and not group/world-writable")
    finally:
      os.close(home_fd)

    def scan(directory_fd: int, depth: int) -> None:
      if depth > 64:
        raise PackageError("Cargo home directory tree is too deep")
      with os.scandir(directory_fd) as entries:
        for entry in entries:
          info = entry.stat(follow_symlinks=False)
          if not safe(info):
            raise PackageError(f"unsafe Cargo home ownership or permissions: {entry.name}")
          if stat.S_ISDIR(info.st_mode):
            child_fd = os.open(entry.name, DIRECTORY_FLAGS, dir_fd=directory_fd)
            try:
              opened = os.fstat(child_fd)
              if (opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino) or not safe(opened):
                raise PackageError(f"Cargo home directory changed: {entry.name}")
              # Rustup normally puts tool symlinks here; Cargo tools are not sandboxed.
              if depth != 0 or entry.name != "bin":
                scan(child_fd, depth + 1)
            finally:
              os.close(child_fd)
          elif not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            raise PackageError(f"Cargo home contains a link or non-regular file: {entry.name}")

    if not safe(os.fstat(fd)):
      raise PackageError("Cargo home must be user-owned and not group/world-writable")
    scan(fd, 0)
    for name in ("index", "cache", "src"):
      cache_fd = open_directory(path / "registry" / name)
      try:
        if not os.listdir(cache_fd):
          raise PackageError(f"Cargo registry {name} must be prepopulated")
      finally:
        os.close(cache_fd)
  finally:
    os.close(fd)


def create_package(binary_path: pathlib.Path, output_path: pathlib.Path,
                   *, root: pathlib.Path = ROOT) -> pathlib.Path:
  """The root parameter is an internal test seam, not a CLI option."""
  root = absolute_path(root)
  output = absolute_path(output_path)
  base = root / "build" / "macos-arm64"
  if base not in output.parent.parents:
    raise PackageError("output must be in a private directory beneath build/macos-arm64/")

  with contextlib.ExitStack() as stack:
    source = stack.enter_context(open_input(binary_path))
    notice_sources = [stack.enter_context(open_input(root / name)) for name in NOTICES]
    binary, mode = snapshot_input(source, "binary", MAX_BINARY_BYTES, executable=True)
    stack.enter_context(binary)
    notices = []
    for name, notice_source in zip(NOTICES, notice_sources):
      notice, _ = snapshot_input(notice_source, name, MAX_NOTICE_BYTES)
      notices.append(stack.enter_context(notice))
    validate_macho(binary)
    directory_fd = open_directory(output.parent, root=root)
    try:
      info = os.fstat(directory_fd)
      if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) & 0o077:
        raise PackageError("output directory must be user-owned and private (mkdir -m 700)")
      if os.listdir(directory_fd):
        raise PackageError("output directory must be empty; existing files were not changed")
      verify_destination(output, root, directory_fd)
      fd = os.open(output.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                   0o600, dir_fd=directory_fd)
      try:
        with os.fdopen(fd, "wb") as target:
          with zipfile.ZipFile(target, "w", compression=zipfile.ZIP_DEFLATED,
                               allowZip64=True) as archive:
            for name, source, mode in [
              ("chipviz-m1", binary, mode),
              *[(name, source, 0o644) for name, source in zip(NOTICES, notices)],
            ]:
              entry = zipfile.ZipInfo(name)
              entry.create_system = 3
              entry.external_attr = (stat.S_IFREG | mode) << 16
              entry.compress_type = zipfile.ZIP_DEFLATED
              with archive.open(entry, "w", force_zip64=True) as dest:
                shutil.copyfileobj(source, dest, length=64 * 1024)
          target.flush()
          verify_destination(output, root, directory_fd, target.fileno())
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
  parser.add_argument("--binary", type=pathlib.Path,
                      help="existing thin macOS ARM64 Mach-O executable")
  destinations = parser.add_mutually_exclusive_group(required=True)
  destinations.add_argument("--output", type=pathlib.Path,
                            help="new archive in an existing empty private directory")
  destinations.add_argument("--prepare-output-dir", type=pathlib.Path,
                            help="create a fresh private directory directly under build/macos-arm64/")
  destinations.add_argument("--check-cargo-home", type=pathlib.Path,
                            help="validate the account's prepopulated ~/.cargo for an offline build")
  args = parser.parse_args(argv)
  if bool(args.binary) != bool(args.output):
    parser.error("--binary and --output must be supplied together")
  try:
    if args.prepare_output_dir:
      output = prepare_output_directory(args.prepare_output_dir)
      print(f"Created private output directory: {output}")
      return 0
    if args.check_cargo_home:
      check_cargo_home(args.check_cargo_home)
      print(f"Checked Cargo home: {args.check_cargo_home}")
      return 0
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
