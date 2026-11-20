import contextlib
import gzip
import importlib.util
import io
import json
import os
import pathlib
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from typing import BinaryIO
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "personal-starfield-bundle.py"
SPEC = importlib.util.spec_from_file_location("personal_starfield_bundle", SCRIPT)
BUNDLE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUNDLE)
CSV = b"id,ra,dec,mag,ci,proper\r\n1,12.5,-30,2.1,0.2,Synthetic\r\n"


class PersonalStarfieldBundleTests(unittest.TestCase):
  def setUp(self) -> None:
    self.temp = tempfile.TemporaryDirectory()
    self.addCleanup(self.temp.cleanup)
    self.base = pathlib.Path(self.temp.name).resolve()
    self.checkout = self.base / "checkout with spaces"
    (self.checkout / "scripts").mkdir(parents=True)
    (self.checkout / ".git").write_text("gitdir: synthetic\n", encoding="utf-8")
    self.script = self.checkout / "scripts" / SCRIPT.name
    shutil.copyfile(SCRIPT, self.script)
    self.inputs = self.base / "inputs with spaces"
    self.inputs.mkdir()
    self.catalog = self.inputs / "synthetic.csv"
    self.catalog.write_bytes(CSV)
    self.binary = self.inputs / "fake renderer"
    self.binary.write_text(
      "#!/bin/sh\n"
      f"exec {shlex.quote(sys.executable)} -c "
      "'import json, sys; print(json.dumps(sys.argv[1:]))' \"$@\"\n",
      encoding="utf-8",
    )
    self.binary.chmod(0o755)
    self.output = self.base / "output with spaces" / "personal bundle"
    self.output.parent.mkdir()

  def run_bundle(self, *args: str, catalog: pathlib.Path | None = None,
                 binary: pathlib.Path | None = None,
                 output: pathlib.Path | None = None) -> subprocess.CompletedProcess:
    command = [sys.executable, str(self.script),
               "--catalog", str(catalog or self.catalog),
               "--binary", str(binary or self.binary)]
    if output is not None:
      command += ["--output", str(output)]
    return subprocess.run(command + list(args), cwd=self.base, capture_output=True,
                          text=True, check=False)

  def assert_success(self, result: subprocess.CompletedProcess) -> None:
    self.assertEqual(result.returncode, 0, result.stderr)

  def test_csv_default_is_ignored_build_directory_and_inputs_are_unchanged(self) -> None:
    binary_bytes = self.binary.read_bytes()
    result = self.run_bundle()
    self.assert_success(result)
    output = self.checkout / "build" / "personal-starfield"
    self.assertIn(str(output), result.stdout)
    self.assertEqual({path.name for path in output.iterdir()}, {
      "chipviz-renderer", "hyg-v4.4.csv", "HYG-NOTICE.txt", "launch-starfield",
    })
    self.assertEqual((output / "hyg-v4.4.csv").read_bytes(), CSV)
    self.assertEqual((output / "chipviz-renderer").read_bytes(), binary_bytes)
    self.assertEqual(self.catalog.read_bytes(), CSV)
    self.assertEqual(self.binary.read_bytes(), binary_bytes)
    self.assertFalse((self.base / "build").exists())
    self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o700)
    for name in ("chipviz-renderer", "launch-starfield"):
      self.assertEqual(stat.S_IMODE((output / name).stat().st_mode), 0o700)
    for name in ("hyg-v4.4.csv", "HYG-NOTICE.txt"):
      self.assertEqual(stat.S_IMODE((output / name).stat().st_mode), 0o600)
    repeat = self.run_bundle()
    self.assertNotEqual(repeat.returncode, 0)
    self.assertIn("already exists", repeat.stderr)
    self.assertEqual((output / "hyg-v4.4.csv").read_bytes(), CSV)

  def test_gzip_is_uncompressed_byte_for_byte_into_explicit_output(self) -> None:
    compressed = self.inputs / "synthetic.csv.gz"
    original = gzip.compress(CSV, mtime=0)
    compressed.write_bytes(original)
    result = self.run_bundle(catalog=compressed, output=self.output)
    self.assert_success(result)
    self.assertEqual((self.output / "hyg-v4.4.csv").read_bytes(), CSV)
    self.assertEqual(compressed.read_bytes(), original)
    self.assertIn(str(self.output), result.stdout)

  def test_notice_has_provenance_terms_and_preserves_supplied_notices(self) -> None:
    first = self.inputs / "upstream copyright.txt"
    second = self.inputs / "upstream license.txt"
    first.write_bytes(b"Copyright 2024 Synthetic Authors\r\n")
    second.write_bytes(b"Synthetic upstream terms\n")
    result = self.run_bundle("--upstream-notice", str(first),
                             "--upstream-notice", str(second), output=self.output)
    self.assert_success(result)
    notice = (self.output / "HYG-NOTICE.txt").read_text(encoding="utf-8")
    for text in (
      "LOCAL PERSONAL/NONCOMMERCIAL USE ONLY",
      "NOT FOR REDISTRIBUTION",
      "Do not upload, publish, redistribute, or use it",
      "Commercial licensees must supply their own catalog",
      "HYG Database, astronexus, https://codeberg.org/astronexus/hyg",
      "https://codeberg.org/astronexus/hyg/media/branch/main/data/hyg/CURRENT/hyg_v44.csv.gz",
      "00b349893b9a53106dd488d8371e8d2fa586043e500bb3cdb8bff3931682197d",
      "13636362 bytes",
      "CC BY-SA 4.0",
      "https://creativecommons.org/licenses/by-sa/4.0/",
      "Hipparcos",
      "CC BY-NC 3.0 IGO",
      "Credit: ESA",
      "does not resolve all upstream terms",
      "does not verify its version",
      "HYG-UPSTREAM-NOTICE-1.txt",
      "HYG-UPSTREAM-NOTICE-2.txt",
    ):
      self.assertIn(text, notice)
    self.assertEqual((self.output / "HYG-UPSTREAM-NOTICE-1.txt").read_bytes(), first.read_bytes())
    self.assertEqual((self.output / "HYG-UPSTREAM-NOTICE-2.txt").read_bytes(), second.read_bytes())

  def test_launcher_forwards_exact_arguments_with_spaces_and_no_implicit_location_or_time(self) -> None:
    self.assert_success(self.run_bundle(output=self.output))
    launcher = self.output / "launch-starfield"
    arguments = ["--location", "0,0", "--time", "0", "a path with spaces", "",
                 "*", "$(not a command)", "line\nbreak", "--"]
    env = os.environ.copy()
    env["CDPATH"] = str(self.base)
    result = subprocess.run([str(launcher), *arguments], cwd=self.base, env=env,
                            capture_output=True, text=True, check=False)
    self.assert_success(result)
    defaults = ["--scene", "6", "--star-catalog", str(self.output / "hyg-v4.4.csv")]
    self.assertEqual(json.loads(result.stdout), defaults + arguments)

    env["PATH"] = str(self.output) + os.pathsep + env["PATH"]
    result = subprocess.run(["launch-starfield"], cwd=self.base, env=env,
                            capture_output=True, text=True, check=False)
    self.assert_success(result)
    self.assertEqual(json.loads(result.stdout), defaults)

  def test_launcher_follows_bundle_when_moved(self) -> None:
    self.assert_success(self.run_bundle(output=self.output))
    moved = self.output.parent / "moved personal bundle"
    self.output.rename(moved)
    result = subprocess.run([str(moved / "launch-starfield"), "--location", "0,0"],
                            cwd=self.base, capture_output=True, text=True, check=False)
    self.assert_success(result)
    self.assertEqual(json.loads(result.stdout), [
      "--scene", "6", "--star-catalog", str(moved / "hyg-v4.4.csv"), "--location", "0,0",
    ])

  def test_launcher_handles_a_directory_name_ending_in_a_newline(self) -> None:
    output = self.output.parent / "bundle with newline\n"
    self.assert_success(self.run_bundle(output=output))
    result = subprocess.run([str(output / "launch-starfield")], cwd=self.base,
                            capture_output=True, text=True, check=False)
    self.assert_success(result)
    self.assertEqual(json.loads(result.stdout), [
      "--scene", "6", "--star-catalog", str(output / "hyg-v4.4.csv"),
    ])

  def test_launcher_rejects_symlinks_for_itself_binary_and_catalog(self) -> None:
    self.assert_success(self.run_bundle(output=self.output))
    launcher = self.output / "launch-starfield"
    alias = self.base / "launcher alias"
    alias.symlink_to(launcher)
    result = subprocess.run([str(alias)], capture_output=True, text=True, check=False)
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("symlink", result.stderr)

    for name, target in (("chipviz-renderer", self.binary), ("hyg-v4.4.csv", self.catalog)):
      with self.subTest(name=name):
        bundled = self.output / name
        backup = self.output / (name + ".backup")
        bundled.rename(backup)
        bundled.symlink_to(target)
        try:
          result = subprocess.run([str(launcher)], capture_output=True, text=True, check=False)
          self.assertNotEqual(result.returncode, 0)
          self.assertIn("symlinks", result.stderr)
        finally:
          bundled.unlink()
          backup.rename(bundled)

  def test_existing_directory_file_and_symlinks_are_never_overwritten(self) -> None:
    directory = self.output.parent / "existing directory"
    directory.mkdir()
    sentinel = directory / "keep.txt"
    sentinel.write_text("keep", encoding="utf-8")
    existing_file = self.output.parent / "existing file"
    existing_file.write_text("keep file", encoding="utf-8")
    link = self.output.parent / "existing symlink"
    link.symlink_to(directory, target_is_directory=True)
    dangling = self.output.parent / "dangling symlink"
    dangling.symlink_to(self.output.parent / "missing target")
    for path in (directory, existing_file, link, dangling):
      with self.subTest(path=path):
        result = self.run_bundle(output=path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("already exists", result.stderr)
    self.assertEqual(sentinel.read_text(encoding="utf-8"), "keep")
    self.assertEqual(existing_file.read_text(encoding="utf-8"), "keep file")
    self.assertTrue(link.is_symlink())
    self.assertTrue(dangling.is_symlink())
    self.assertEqual(self.catalog.read_bytes(), CSV)

  def test_symlink_parents_and_missing_parents_fail_without_creating_output(self) -> None:
    real_parent = self.base / "real parent"
    real_parent.mkdir()
    linked_parent = self.base / "linked parent"
    linked_parent.symlink_to(real_parent, target_is_directory=True)
    result = self.run_bundle(output=linked_parent / "bundle")
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("no symlinks", result.stderr)
    self.assertFalse((real_parent / "bundle").exists())

    result = self.run_bundle(output=self.base / "missing parent" / "bundle")
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("parent must exist", result.stderr)
    self.assertFalse((self.base / "missing parent").exists())

    (self.checkout / "build").symlink_to(real_parent, target_is_directory=True)
    result = self.run_bundle()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("no symlinks", result.stderr)
    self.assertFalse((real_parent / "personal-starfield").exists())

  def test_existing_build_file_is_never_replaced(self) -> None:
    build = self.checkout / "build"
    build.write_text("keep build file", encoding="utf-8")
    result = self.run_bundle()
    self.assertNotEqual(result.returncode, 0)
    self.assertEqual(build.read_text(encoding="utf-8"), "keep build file")

  def test_release_tracked_other_checkout_and_parent_traversal_paths_are_rejected(self) -> None:
    (self.checkout / "assets" / "scenes").mkdir(parents=True)
    (self.checkout / "dist").mkdir()
    other = self.base / "other checkout"
    (other / ".git").mkdir(parents=True)
    paths = [
      self.checkout / "assets" / "scenes" / "bundle",
      self.checkout / "dist" / "bundle",
      self.checkout / "build" / "release" / "bundle",
      other / "bundle",
      self.output.parent / ".." / "bundle",
      pathlib.Path("/" + str(self.checkout / "assets" / "scenes" / "bundle")),
    ]
    for path in paths:
      with self.subTest(path=path):
        result = self.run_bundle(output=path)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(path.exists())
    self.assertFalse((self.checkout / "build").exists())
    self.assertEqual(list((other / ".git").iterdir()), [])

  def test_missing_nonfiles_symlink_inputs_and_nonexecutable_binary_fail(self) -> None:
    catalog_directory = self.inputs / "directory.csv"
    catalog_directory.mkdir()
    catalog_link = self.inputs / "linked.csv"
    catalog_link.symlink_to(self.catalog)
    binary_link = self.inputs / "linked executable"
    binary_link.symlink_to(self.binary)
    not_executable = self.inputs / "nonexecutable"
    not_executable.write_text("not executable", encoding="utf-8")
    not_executable.chmod(0o644)
    cases = [
      {"catalog": self.inputs / "missing.csv"},
      {"catalog": catalog_directory},
      {"catalog": catalog_link},
      {"binary": self.inputs / "missing binary"},
      {"binary": self.inputs},
      {"binary": binary_link},
      {"binary": not_executable},
    ]
    for case in cases:
      with self.subTest(case=case):
        result = self.run_bundle(output=self.output, **case)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("error:", result.stderr)
        self.assertFalse(self.output.exists())
    result = self.run_bundle("--upstream-notice", str(self.inputs / "missing notice"),
                             output=self.output)
    self.assertNotEqual(result.returncode, 0)
    self.assertFalse(self.output.exists())
    notice_link = self.inputs / "notice symlink"
    notice_link.symlink_to(self.catalog)
    result = self.run_bundle("--upstream-notice", str(notice_link), output=self.output)
    self.assertNotEqual(result.returncode, 0)
    self.assertFalse(self.output.exists())

  def test_invalid_gzip_and_empty_catalogs_leave_no_partial_bundle(self) -> None:
    large = CSV * 30000
    valid = gzip.compress(large, mtime=0)
    corrupt = valid[:-1]
    wrong_crc = bytearray(valid)
    wrong_crc[-8] ^= 1
    invalid = {
      "not-gzip.csv.gz": b"not gzip",
      "empty-file.csv.gz": b"",
      "truncated.csv.gz": corrupt,
      "wrong-crc.csv.gz": bytes(wrong_crc),
      "empty-content.csv.gz": gzip.compress(b"", mtime=0),
      "empty.csv": b"",
      "wrong-extension.csv": valid,
    }
    for name, contents in invalid.items():
      with self.subTest(name=name):
        path = self.inputs / name
        path.write_bytes(contents)
        result = self.run_bundle(catalog=path, output=self.output)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("error:", result.stderr)
        self.assertFalse(self.output.exists())
        self.assertEqual(path.read_bytes(), contents)

    result = self.run_bundle(catalog=self.inputs / "not-a-csv.gz", output=self.output)
    self.assertNotEqual(result.returncode, 0)
    self.assertIn(".csv or .csv.gz", result.stderr)

  def test_partial_cleanup_leaves_unexpected_user_files_untouched(self) -> None:
    foreign = self.output / "user-file.txt"

    def fail_copy(source: object, dest: BinaryIO) -> None:
      dest.write(b"partial")
      foreign.write_text("keep", encoding="utf-8")
      raise OSError("simulated write failure")

    errors = io.StringIO()
    with mock.patch.object(BUNDLE.shutil, "copyfileobj", side_effect=fail_copy):
      with contextlib.redirect_stderr(errors):
        with self.assertRaisesRegex(OSError, "simulated write failure"):
          BUNDLE.create_bundle(self.catalog, self.binary, self.output, [])
    self.assertEqual(foreign.read_text(encoding="utf-8"), "keep")
    self.assertFalse((self.output / "hyg-v4.4.csv").exists())
    self.assertIn("Partial output remains", errors.getvalue())

  def test_partial_cleanup_does_not_unlink_a_replaced_file(self) -> None:
    replacement = self.output / "hyg-v4.4.csv"

    def replace_and_fail(source: object, dest: BinaryIO) -> None:
      dest.write(b"partial")
      replacement.unlink()
      replacement.write_bytes(b"replacement user file")
      raise OSError("simulated write failure")

    errors = io.StringIO()
    with mock.patch.object(BUNDLE.shutil, "copyfileobj", side_effect=replace_and_fail):
      with contextlib.redirect_stderr(errors):
        with self.assertRaisesRegex(OSError, "simulated write failure"):
          BUNDLE.create_bundle(self.catalog, self.binary, self.output, [])
    self.assertEqual(replacement.read_bytes(), b"replacement user file")
    self.assertIn("Partial output remains", errors.getvalue())


if __name__ == "__main__":
  unittest.main()
