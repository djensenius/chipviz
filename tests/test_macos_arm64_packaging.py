import contextlib
import importlib.util
import io
import os
import pathlib
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/package-macos-arm64.py"
SPEC = importlib.util.spec_from_file_location("package_macos_arm64", SCRIPT)
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


def build_version(platform: int = 1, *, version: int = 0x000b0000,
                  sdk: int = 0, tools: bytes = b"") -> bytes:
  return struct.pack("<6I", 0x32, 24 + len(tools), platform, version, sdk,
                     len(tools) // 8) + tools


def legacy_version(command: int = 0x24, *, version: int = 0x000b0000,
                   sdk: int = 0) -> bytes:
  return struct.pack("<4I", command, 16, version, sdk)


def macho(commands: bytes, *, cpu: int = 0x0100000c, kind: int = 2,
          count: int = 1, command_bytes: int | None = None,
          endian: str = "<") -> bytes:
  # Synthetic headers only; these are not runnable executables.
  return (struct.pack(endian + "8I", 0xfeedfacf, cpu, 0, kind, count,
                      len(commands) if command_bytes is None else command_bytes, 0, 0)
          + commands + b"synthetic payload")


class MacosArm64PackagingTests(unittest.TestCase):
  def setUp(self) -> None:
    self.temp = tempfile.TemporaryDirectory()
    self.addCleanup(self.temp.cleanup)
    self.root = pathlib.Path(self.temp.name).resolve() / "checkout"
    (self.root / "scripts").mkdir(parents=True)
    shutil.copyfile(SCRIPT, self.root / "scripts" / SCRIPT.name)
    self.notices = {
      "LICENSE": b"license fixture\n",
      "COMMERCIAL.md": b"commercial fixture\n",
      "THIRD_PARTY_NOTICES/rust.md": b"rust notices fixture\n",
    }
    for name, contents in self.notices.items():
      path = self.root / name
      path.parent.mkdir(parents=True, exist_ok=True)
      path.write_bytes(contents)
    self.binary = self.root / "renderer"
    self.binary.write_bytes(macho(build_version()))
    self.binary.chmod(0o755)
    self.base = self.root / "build/macos-arm64"
    self.output_dir = self.base / "fresh"
    self.output_dir.mkdir(parents=True, mode=0o700)
    self.base.chmod(0o700)
    self.output = self.output_dir / "local.zip"
    self.dist = self.root / "dist/sentinel"
    self.dist.parent.mkdir()
    self.dist.write_bytes(b"dist unchanged")
    self.personal = self.root / "build/personal-starfield/sentinel"
    self.personal.parent.mkdir()
    self.personal.write_bytes(b"personal unchanged")

  def run_package(self, *, binary: pathlib.Path | None = None,
                  output: pathlib.Path | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
      [sys.executable, str(self.root / "scripts" / SCRIPT.name),
       "--binary", str(binary or self.binary), "--output", str(output or self.output)],
      cwd=self.root, capture_output=True, text=True, check=False,
    )

  def assert_rejected(self, data: bytes) -> None:
    self.binary.write_bytes(data)
    result = self.run_package()
    self.assertNotEqual(result.returncode, 0, result.stdout)
    self.assertFalse(self.output.exists(), result.stderr)

  def test_archive_contents_modes_and_untouched_sentinels(self) -> None:
    original = self.binary.read_bytes()
    result = self.run_package()
    self.assertEqual(result.returncode, 0, result.stderr)
    self.assertEqual(stat.S_IMODE(self.output.stat().st_mode) & 0o077, 0)
    with zipfile.ZipFile(self.output) as archive:
      self.assertEqual(set(archive.namelist()), {
        "chipviz-m1", "LICENSE", "COMMERCIAL.md", "THIRD_PARTY_NOTICES/rust.md",
      })
      self.assertEqual(archive.read("chipviz-m1"), original)
      self.assertEqual(archive.getinfo("chipviz-m1").create_system, 3)
      self.assertEqual(archive.getinfo("chipviz-m1").external_attr >> 16,
                       stat.S_IFREG | 0o755)
      for name, contents in self.notices.items():
        self.assertEqual(archive.read(name), contents)
    self.assertEqual(self.binary.read_bytes(), original)
    self.assertEqual(self.dist.read_bytes(), b"dist unchanged")
    self.assertEqual(self.personal.read_bytes(), b"personal unchanged")

  def test_valid_platform_declarations(self) -> None:
    for commands in ((build_version(),), (legacy_version(),),
                     (build_version(), legacy_version()),
                     (legacy_version(), legacy_version()),
                     (build_version(), build_version()),
                     (build_version(tools=struct.pack("<2I", 1, 0x10000)),
                      build_version(tools=struct.pack("<2I", 1, 0x10000)))):
      with self.subTest(commands=commands):
        self.binary.write_bytes(macho(b"".join(commands), count=len(commands)))
        result = self.run_package()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.output.unlink()

  def test_conflicting_platform_declarations(self) -> None:
    for commands in (
      (build_version(), build_version(version=0x000c0000)),
      (legacy_version(), legacy_version(version=0x000c0000)),
      (legacy_version(), build_version(version=0x000c0000)),
      (build_version(), legacy_version(sdk=0x000d0000)),
      (build_version(), build_version(sdk=0x000d0000)),
      (build_version(tools=struct.pack("<2I", 1, 1)),
       build_version(tools=struct.pack("<2I", 1, 2))),
      (build_version(), build_version(2)),
    ):
      with self.subTest(commands=commands):
        self.assert_rejected(macho(b"".join(commands), count=len(commands)))

  def test_binary_permissions_are_preserved_and_unsafe_modes_rejected(self) -> None:
    for mode in (0o700, 0o750, 0o755):
      with self.subTest(mode=oct(mode)):
        self.binary.chmod(mode)
        result = self.run_package()
        self.assertEqual(result.returncode, 0, result.stderr)
        with zipfile.ZipFile(self.output) as archive:
          self.assertEqual(archive.getinfo("chipviz-m1").external_attr >> 16,
                           stat.S_IFREG | mode)
        self.output.unlink()
    for mode in (0o600, 0o641, 0o644, 0o4755, 0o2755, 0o1755):
      with self.subTest(mode=oct(mode)):
        self.binary.chmod(mode)
        self.assertEqual(stat.S_IMODE(self.binary.stat().st_mode), mode)
        result = self.run_package()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertFalse(self.output.exists())

  def test_snapshot_is_validated_and_archived_even_if_source_changes_afterward(self) -> None:
    original = self.binary.read_bytes()
    copy = shutil.copyfileobj
    mutated = False

    def mutate_then_copy(source, destination, length=0):
      nonlocal mutated
      if not mutated:
        self.binary.write_bytes(b"not a Mach-O")
        mutated = True
      return copy(source, destination, length)

    with mock.patch.object(PACKAGE.shutil, "copyfileobj", side_effect=mutate_then_copy):
      PACKAGE.create_package(self.binary, self.output, root=self.root)
    self.assertTrue(mutated)
    with zipfile.ZipFile(self.output) as archive:
      self.assertEqual(archive.read("chipviz-m1"), original)
    self.assertEqual(self.binary.read_bytes(), b"not a Mach-O")

  def test_source_change_during_snapshot_is_rejected(self) -> None:
    original = macho(build_version()) + b"x" * (128 * 1024)
    for replacement in (b"changed", b"y" * len(original)):
      with self.subTest(same_size=len(replacement) == len(original)):
        self.binary.write_bytes(original)
        with self.binary.open("rb") as source:
          reader = mock.Mock(wraps=source)
          mutated = False

          def read_and_mutate(length):
            nonlocal mutated
            data = source.read(length)
            if not mutated:
              self.binary.write_bytes(replacement)
              mutated = True
            return data

          reader.read.side_effect = read_and_mutate
          with self.assertRaisesRegex(PACKAGE.PackageError, "changed while taking snapshot"):
            PACKAGE.snapshot_input(reader, "binary", 256 * 1024 * 1024, executable=True)
    self.assertFalse(self.output.exists())

  def test_oversized_binary_is_rejected_before_copying(self) -> None:
    with self.binary.open("r+b") as source:
      source.truncate(256 * 1024 * 1024 + 1)
    result = self.run_package()
    self.assertNotEqual(result.returncode, 0, result.stdout)
    self.assertIn("268435456-byte snapshot limit", result.stderr)
    self.assertFalse(self.output.exists())

  def test_tmpdir_protected_directories_have_no_transient_writes_or_removals(self) -> None:
    original_open = os.open
    original_unlink = os.unlink
    for protected in (self.dist.parent, self.personal.parent):
      with self.subTest(tmpdir=protected):
        events = []
        protected_id = (protected.stat().st_dev, protected.stat().st_ino)

        def in_protected(path, dir_fd):
          if dir_fd is not None:
            info = os.fstat(dir_fd)
            return (info.st_dev, info.st_ino) == protected_id
          candidate = pathlib.Path(path)
          return candidate.is_absolute() and (candidate == protected or protected in candidate.parents)

        def watched_open(path, flags, mode=0o777, *, dir_fd=None):
          if flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC):
            if in_protected(path, dir_fd):
              events.append(("open", str(path)))
          return original_open(path, flags, mode, dir_fd=dir_fd)

        def watched_unlink(path, *, dir_fd=None):
          if in_protected(path, dir_fd):
            events.append(("unlink", str(path)))
          return original_unlink(path, dir_fd=dir_fd)

        with mock.patch.dict(os.environ, {"TMPDIR": str(protected)}), \
             mock.patch.object(tempfile, "tempdir", None), \
             mock.patch.object(os, "open", side_effect=watched_open), \
             mock.patch.object(os, "unlink", side_effect=watched_unlink):
          PACKAGE.create_package(self.binary, self.output, root=self.root)
        self.assertEqual(events, [])
        self.output.unlink()
    self.assertEqual(self.dist.read_bytes(), b"dist unchanged")
    self.assertEqual(self.personal.read_bytes(), b"personal unchanged")

  def test_notice_changes_during_snapshot_fail_without_output(self) -> None:
    original_open = PACKAGE.open_input
    for name in self.notices:
      path = self.root / name
      original = b"notice fixture" + b"x" * (128 * 1024)
      for replacement in (b"short", b"y" * len(original)):
        with self.subTest(name=name, same_size=len(original) == len(replacement)):
          path.write_bytes(original)
          mutated = False

          def instrumented_open(input_path):
            source = original_open(input_path)
            if input_path != path:
              return source
            reader = mock.MagicMock(wraps=source)
            reader.__enter__.return_value = reader
            reader.__exit__.side_effect = lambda *args: source.close()

            def read_and_mutate(length):
              nonlocal mutated
              data = source.read(length)
              if not mutated:
                path.write_bytes(replacement)
                mutated = True
              return data

            reader.read.side_effect = read_and_mutate
            return reader

          with mock.patch.object(PACKAGE, "open_input", side_effect=instrumented_open):
            with self.assertRaisesRegex(PACKAGE.PackageError, "changed while taking snapshot"):
              PACKAGE.create_package(self.binary, self.output, root=self.root)
          self.assertTrue(mutated)
          self.assertFalse(self.output.exists())
      path.write_bytes(self.notices[name])

  def test_notice_snapshots_are_archived_after_sources_change(self) -> None:
    copy = shutil.copyfileobj
    mutated = False

    def mutate_then_copy(source, destination, length=0):
      nonlocal mutated
      if not mutated:
        for name in self.notices:
          (self.root / name).write_bytes(b"changed")
        mutated = True
      return copy(source, destination, length)

    with mock.patch.object(PACKAGE.shutil, "copyfileobj", side_effect=mutate_then_copy):
      PACKAGE.create_package(self.binary, self.output, root=self.root)
    self.assertTrue(mutated)
    with zipfile.ZipFile(self.output) as archive:
      for name, original in self.notices.items():
        self.assertEqual(archive.read(name), original)

  def test_oversized_notice_is_rejected_before_output(self) -> None:
    with (self.root / "THIRD_PARTY_NOTICES/rust.md").open("r+b") as notice:
      notice.truncate(8 * 1024 * 1024 + 1)
    result = self.run_package()
    self.assertNotEqual(result.returncode, 0, result.stdout)
    self.assertIn("8388608-byte snapshot limit", result.stderr)
    self.assertFalse(self.output.exists())

  def test_renamed_output_directory_is_detected_before_and_after_creation(self) -> None:
    moved = self.base / "moved"
    original_listdir = os.listdir

    def replace_before_create(fd):
      result = original_listdir(fd)
      self.output_dir.rename(moved)
      self.output_dir.mkdir(mode=0o700)
      (self.output_dir / "sentinel").write_bytes(b"replacement")
      return result

    with mock.patch.object(PACKAGE.os, "listdir", side_effect=replace_before_create):
      with self.assertRaisesRegex(PACKAGE.PackageError, "directory changed"):
        PACKAGE.create_package(self.binary, self.output, root=self.root)
    self.assertEqual((self.output_dir / "sentinel").read_bytes(), b"replacement")
    self.assertEqual(list(moved.iterdir()), [])
    (self.output_dir / "sentinel").unlink()
    self.output_dir.rmdir()
    moved.rename(self.output_dir)

    copy = shutil.copyfileobj
    replaced = False

    def replace_during_copy(source, destination, length=0):
      nonlocal replaced
      if not replaced:
        self.output_dir.rename(moved)
        self.output_dir.mkdir(mode=0o700)
        (self.output_dir / "sentinel").write_bytes(b"replacement")
        replaced = True
      return copy(source, destination, length)

    with contextlib.redirect_stderr(io.StringIO()), \
         mock.patch.object(PACKAGE.shutil, "copyfileobj", side_effect=replace_during_copy):
      with self.assertRaisesRegex(PACKAGE.PackageError, "directory changed"):
        PACKAGE.create_package(self.binary, self.output, root=self.root)
    self.assertTrue(replaced)
    self.assertEqual((self.output_dir / "sentinel").read_bytes(), b"replacement")
    self.assertTrue((moved / "local.zip").is_file())
    self.assertEqual(self.dist.read_bytes(), b"dist unchanged")
    self.assertEqual(self.personal.read_bytes(), b"personal unchanged")

  def test_replaced_archive_and_partial_write_are_not_removed(self) -> None:
    copy = shutil.copyfileobj
    replaced = False
    moved = self.output_dir / "moved.zip"

    def replace_archive(source, destination, length=0):
      nonlocal replaced
      if not replaced:
        self.output.rename(moved)
        self.output.write_bytes(b"replacement")
        replaced = True
      return copy(source, destination, length)

    with contextlib.redirect_stderr(io.StringIO()), \
         mock.patch.object(PACKAGE.shutil, "copyfileobj", side_effect=replace_archive):
      with self.assertRaisesRegex(PACKAGE.PackageError, "archive changed"):
        PACKAGE.create_package(self.binary, self.output, root=self.root)
    self.assertEqual(self.output.read_bytes(), b"replacement")
    self.assertTrue(moved.is_file())
    self.output.unlink()
    moved.unlink()

    def fail_copy(source, destination, length=0):
      destination.write(b"partial")
      raise OSError("injected write failure")

    with contextlib.redirect_stderr(io.StringIO()), \
         mock.patch.object(PACKAGE.shutil, "copyfileobj", side_effect=fail_copy):
      with self.assertRaisesRegex(OSError, "injected write failure"):
        PACKAGE.create_package(self.binary, self.output, root=self.root)
    self.assertTrue(self.output.is_file())

  def test_cli_partial_write_failure_reports_error_without_success(self) -> None:
    # Inject a failure in the subprocess, then run the real CLI entry point.
    runner = """
import runpy, shutil, sys
script = sys.argv.pop(1)
sys.argv[0] = script
def fail_copy(source, destination, length=0):
  destination.write(b"partial")
  raise OSError("injected write failure")
shutil.copyfileobj = fail_copy
runpy.run_path(script, run_name="__main__")
"""
    result = subprocess.run(
      [sys.executable, "-c", runner, str(self.root / "scripts" / SCRIPT.name),
       "--binary", str(self.binary), "--output", str(self.output)],
      cwd=self.root, capture_output=True, text=True, check=False,
    )
    self.assertEqual(result.returncode, 1, result.stderr)
    self.assertEqual(result.stdout, "")
    self.assertIn("Archive may be incomplete; no files were removed.", result.stderr)
    self.assertIn("error: injected write failure", result.stderr)
    self.assertTrue(self.output.is_file())
    with zipfile.ZipFile(self.output) as archive:
      self.assertEqual(archive.namelist(), ["chipviz-m1"])
      self.assertEqual(archive.read("chipviz-m1"), b"partial")
    self.assertEqual(self.dist.read_bytes(), b"dist unchanged")
    self.assertEqual(self.personal.read_bytes(), b"personal unchanged")

  def test_rejects_wrong_platform_architecture_type_and_endian(self) -> None:
    fixtures = (
      macho(build_version(2)), macho(build_version(7)),
      macho(legacy_version(0x25)), macho(build_version(), cpu=0x01000007),
      macho(build_version(), kind=6), macho(build_version(), endian=">"),
      macho(struct.pack("<2I", 0x19, 8)),
      macho(build_version() + legacy_version(0x25), count=2),
    )
    for data in fixtures:
      with self.subTest(data=data):
        self.assert_rejected(data)

  def test_rejects_truncated_and_malformed_commands(self) -> None:
    fixtures = (
      b"\xcf\xfa\xed\xfe", macho(build_version())[:40],
      macho(build_version(), command_bytes=0xffffffff),
      macho(build_version(), count=0xffffffff),
      macho(struct.pack("<2I", 0x32, 0)),
      macho(struct.pack("<2I", 0x32, 12) + bytes(4)),
      macho(struct.pack("<2I", 0x32, 24) + bytes(8)),
      macho(struct.pack("<6I", 0x32, 24, 1, 0x000b0000, 0, 1)),
      macho(struct.pack("<4I", 0x24, 16, 0, 0)),
      macho(build_version(), count=0),
      macho(build_version() + bytes(8)),
    )
    for data in fixtures:
      with self.subTest(data=data):
        self.assert_rejected(data)

  def test_missing_and_nonregular_or_symlink_inputs(self) -> None:
    notice = self.root / "THIRD_PARTY_NOTICES/rust.md"
    notice.unlink()
    self.assertNotEqual(self.run_package().returncode, 0)
    notice.symlink_to(self.root / "LICENSE")
    self.assertNotEqual(self.run_package().returncode, 0)
    notice.unlink()
    notice.write_bytes(b"notices")
    directory = self.root / "THIRD_PARTY_NOTICES"
    moved = self.root / "real-notices"
    directory.rename(moved)
    directory.symlink_to(moved, target_is_directory=True)
    self.assertNotEqual(self.run_package().returncode, 0)
    directory.unlink()
    moved.rename(directory)
    notice.unlink()
    notice.mkdir()
    self.assertNotEqual(self.run_package().returncode, 0)
    notice.rmdir()
    notice.write_bytes(b"notices")
    link = self.root / "binary-link"
    link.symlink_to(self.binary)
    self.assertNotEqual(self.run_package(binary=link).returncode, 0)
    fifo = self.root / "fifo"
    os.mkfifo(fifo)
    self.assertNotEqual(self.run_package(binary=fifo).returncode, 0)
    self.assertFalse(self.output.exists())

  def test_output_collision_symlinks_escape_and_permissions(self) -> None:
    self.output.write_bytes(b"existing")
    self.assertNotEqual(self.run_package().returncode, 0)
    self.assertEqual(self.output.read_bytes(), b"existing")
    self.output.unlink()
    self.output.symlink_to(self.dist)
    self.assertNotEqual(self.run_package().returncode, 0)
    self.output.unlink()
    alias = self.base / "alias"
    alias.symlink_to(self.output_dir, target_is_directory=True)
    self.assertNotEqual(self.run_package(output=alias / "local.zip").returncode, 0)
    self.assertNotEqual(self.run_package(output=self.base / "local.zip").returncode, 0)
    self.assertNotEqual(self.run_package(output=self.root / "dist/local.zip").returncode, 0)
    escape = self.output_dir / ".." / ".." / "personal-starfield" / "local.zip"
    self.assertNotEqual(self.run_package(output=escape).returncode, 0)
    self.output_dir.chmod(0o755)
    self.assertNotEqual(self.run_package().returncode, 0)
    self.output_dir.chmod(0o700)
    self.base.chmod(0o755)
    self.assertNotEqual(self.run_package().returncode, 0)
    self.assertEqual(self.dist.read_bytes(), b"dist unchanged")
    self.assertEqual(self.personal.read_bytes(), b"personal unchanged")

  def test_help_explains_scope_and_no_clearance(self) -> None:
    result = subprocess.run([sys.executable, str(SCRIPT), "--help"], capture_output=True,
                            text=True, check=False)
    self.assertEqual(result.returncode, 0, result.stderr)
    for phrase in ("headless", "redistribution clearance", "runtime compatibility",
                   "rendering", "signing", "notarization"):
      self.assertIn(phrase, result.stdout)


if __name__ == "__main__":
  unittest.main()
