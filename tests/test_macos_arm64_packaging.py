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


ROOT = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/package-macos-arm64.py"


def build_version(platform: int = 1) -> bytes:
  return struct.pack("<6I", 0x32, 24, platform, 0x000b0000, 0, 0)


def legacy_version(command: int = 0x24) -> bytes:
  return struct.pack("<4I", command, 16, 0x000b0000, 0)


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
    for command in (build_version(), legacy_version(),
                    build_version() + legacy_version()):
      with self.subTest(command=command):
        self.binary.write_bytes(macho(command, count=2 if len(command) == 40 else 1))
        result = self.run_package()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.output.unlink()

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
