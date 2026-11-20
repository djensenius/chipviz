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
    self.build = self.checkout / "build"
    self.build.mkdir(mode=0o700)
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
    self.output = self.build / "output with spaces" / "personal bundle"
    self.output.parent.mkdir(mode=0o700)
    self.output.mkdir(mode=0o700)

  def run_bundle(self, *args: str, catalog: pathlib.Path | None = None,
                 binary: pathlib.Path | None = None,
                 output: pathlib.Path | None = None,
                 umask: int | None = None) -> subprocess.CompletedProcess:
    command = [sys.executable, str(self.script),
               "--catalog", str(catalog or self.catalog),
               "--binary", str(binary or self.binary)]
    if output is not None:
      command += ["--output", str(output)]
    set_umask = None if umask is None else lambda: os.umask(umask)
    return subprocess.run(command + list(args), cwd=self.base, capture_output=True,
                          text=True, check=False, preexec_fn=set_umask)

  @contextlib.contextmanager
  def in_checkout(self):
    with mock.patch.object(BUNDLE, "ROOT", self.checkout):
      with mock.patch.object(BUNDLE, "BUILD", self.build):
        yield

  def assert_success(self, result: subprocess.CompletedProcess) -> None:
    self.assertEqual(result.returncode, 0, result.stderr)

  def test_csv_default_is_ignored_build_directory_and_inputs_are_unchanged(self) -> None:
    binary_bytes = self.binary.read_bytes()
    output = self.build / "personal-starfield"
    missing = self.run_bundle()
    self.assertNotEqual(missing.returncode, 0)
    self.assertIn("must exist", missing.stderr)
    self.assertFalse(output.exists())
    output.mkdir(mode=0o700)
    result = self.run_bundle()
    self.assert_success(result)
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
    self.assertIn("must be empty", repeat.stderr)
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
    output.mkdir(mode=0o700)
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
    directory.mkdir(mode=0o700)
    sentinel = directory / "keep.txt"
    sentinel.write_text("keep", encoding="utf-8")
    existing_file = self.output.parent / "existing file"
    existing_file.write_text("keep file", encoding="utf-8")
    link = self.output.parent / "existing symlink"
    link.symlink_to(directory, target_is_directory=True)
    dangling = self.output.parent / "dangling symlink"
    dangling.symlink_to(self.output.parent / "missing target")
    for path, message in ((directory, "must be empty"), (existing_file, "no symlinks"),
                          (link, "no symlinks"), (dangling, "no symlinks")):
      with self.subTest(path=path):
        result = self.run_bundle(output=path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)
    self.assertEqual(sentinel.read_text(encoding="utf-8"), "keep")
    self.assertEqual(existing_file.read_text(encoding="utf-8"), "keep file")
    self.assertTrue(link.is_symlink())
    self.assertTrue(dangling.is_symlink())
    self.assertEqual(self.catalog.read_bytes(), CSV)

  def test_symlink_parents_and_missing_parents_fail_without_creating_output(self) -> None:
    real_parent = self.build / "real parent"
    real_parent.mkdir(mode=0o700)
    real_output = real_parent / "bundle"
    real_output.mkdir(mode=0o700)
    linked_parent = self.build / "linked parent"
    linked_parent.symlink_to(real_parent, target_is_directory=True)
    result = self.run_bundle(output=linked_parent / "bundle")
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("no symlinks", result.stderr)
    self.assertEqual(list(real_output.iterdir()), [])

    result = self.run_bundle(output=self.build / "missing parent" / "bundle")
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("must exist", result.stderr)
    self.assertFalse((self.build / "missing parent").exists())

    (self.build / "personal-starfield").symlink_to(real_output, target_is_directory=True)
    result = self.run_bundle()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("no symlinks", result.stderr)
    self.assertEqual(list(real_output.iterdir()), [])

  def test_existing_parent_file_is_never_replaced(self) -> None:
    parent = self.build / "parent file"
    parent.write_text("keep parent file", encoding="utf-8")
    result = self.run_bundle(output=parent / "bundle")
    self.assertNotEqual(result.returncode, 0)
    self.assertEqual(parent.read_text(encoding="utf-8"), "keep parent file")

  def test_destination_must_be_private_and_is_not_chmodded(self) -> None:
    self.output.chmod(0o755)
    result = self.run_bundle(output=self.output)
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("private", result.stderr)
    self.assertEqual(stat.S_IMODE(self.output.stat().st_mode), 0o755)
    self.assertEqual(list(self.output.iterdir()), [])

  def test_release_tracked_other_checkout_and_parent_traversal_paths_are_rejected(self) -> None:
    (self.checkout / "assets" / "scenes").mkdir(parents=True)
    tracked = self.checkout / "assets" / "scenes" / "bundle"
    tracked.mkdir(mode=0o700)
    dist = self.checkout / "dist" / "bundle"
    dist.mkdir(mode=0o700, parents=True)
    release = self.build / "release" / "bundle"
    release.mkdir(mode=0o700, parents=True)
    outside = self.base / "outside bundle"
    outside.mkdir(mode=0o700)
    other = self.build / "other checkout"
    (other / ".git").mkdir(parents=True)
    nested = other / "bundle"
    nested.mkdir(mode=0o700)
    paths = [
      (tracked, "below this checkout"),
      (dist, "release path"),
      (release, "release path"),
      (outside, "below this checkout"),
      (nested, "another Git checkout"),
      (self.output.parent / ".." / "output with spaces" / "personal bundle", "must not contain"),
      (pathlib.Path("/" + str(tracked)), "single leading slash"),
    ]
    for path, message in paths:
      with self.subTest(path=path):
        result = self.run_bundle(output=path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)
        self.assertEqual(list(path.iterdir()), [])
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
        self.assertEqual(list(self.output.iterdir()), [])
    result = self.run_bundle("--upstream-notice", str(self.inputs / "missing notice"),
                             output=self.output)
    self.assertNotEqual(result.returncode, 0)
    self.assertEqual(list(self.output.iterdir()), [])
    notice_link = self.inputs / "notice symlink"
    notice_link.symlink_to(self.catalog)
    result = self.run_bundle("--upstream-notice", str(notice_link), output=self.output)
    self.assertNotEqual(result.returncode, 0)
    self.assertEqual(list(self.output.iterdir()), [])

  def test_invalid_gzip_and_empty_catalogs_preserve_any_partial_bundle(self) -> None:
    large = CSV * 30000
    valid = gzip.compress(large, mtime=0)
    corrupt = valid[:-1]
    wrong_crc = bytearray(valid)
    wrong_crc[-8] ^= 1
    invalid = {
      "not-gzip.csv.gz": (b"not gzip", False),
      "empty-file.csv.gz": (b"", False),
      "truncated.csv.gz": (corrupt, True),
      "wrong-crc.csv.gz": (bytes(wrong_crc), True),
      "corrupt-second-member.csv.gz": (gzip.compress(CSV, mtime=0) + corrupt, True),
      "empty-content.csv.gz": (gzip.compress(b"", mtime=0), True),
      "empty.csv": (b"", False),
      "wrong-extension.csv": (valid, False),
    }
    for index, (name, (contents, partial)) in enumerate(invalid.items()):
      with self.subTest(name=name):
        path = self.inputs / name
        path.write_bytes(contents)
        output = self.output.parent / f"invalid-{index}"
        output.mkdir(mode=0o700)
        result = self.run_bundle(catalog=path, output=output)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("error:", result.stderr)
        if partial:
          self.assertTrue((output / "hyg-v4.4.csv").is_file())
          self.assertIn("No files were removed", result.stderr)
        else:
          self.assertEqual(list(output.iterdir()), [])
        self.assertFalse((output / "launch-starfield").exists())
        self.assertEqual(path.read_bytes(), contents)

    result = self.run_bundle(catalog=self.inputs / "not-a-csv.gz", output=self.output)
    self.assertNotEqual(result.returncode, 0)
    self.assertIn(".csv or .csv.gz", result.stderr)

  def test_failure_leaves_partial_and_unexpected_user_files_untouched(self) -> None:
    foreign = self.output / "user-file.txt"

    def fail_copy(source: object, dest: BinaryIO, limit: int) -> None:
      dest.write(b"partial")
      foreign.write_text("keep", encoding="utf-8")
      raise OSError("simulated write failure")

    errors = io.StringIO()
    with self.in_checkout():
      with mock.patch.object(BUNDLE, "copy_catalog", side_effect=fail_copy):
        with contextlib.redirect_stderr(errors):
          with self.assertRaisesRegex(OSError, "simulated write failure"):
            BUNDLE.create_bundle(self.catalog, self.binary, self.output, [])
    self.assertEqual(foreign.read_text(encoding="utf-8"), "keep")
    self.assertEqual((self.output / "hyg-v4.4.csv").read_bytes(), b"partial")
    self.assertIn("No files were removed", errors.getvalue())

  def test_failure_does_not_unlink_a_replaced_file(self) -> None:
    replacement = self.output / "hyg-v4.4.csv"
    moved = self.output / "moved-partial.csv"

    def replace_and_fail(source: object, dest: BinaryIO, limit: int) -> None:
      dest.write(b"partial")
      dest.flush()
      replacement.rename(moved)
      replacement.write_bytes(b"replacement user file")
      raise OSError("simulated write failure")

    errors = io.StringIO()
    with self.in_checkout():
      with mock.patch.object(BUNDLE, "copy_catalog", side_effect=replace_and_fail):
        with contextlib.redirect_stderr(errors):
          with self.assertRaisesRegex(OSError, "simulated write failure"):
            BUNDLE.create_bundle(self.catalog, self.binary, self.output, [])
    self.assertEqual(replacement.read_bytes(), b"replacement user file")
    self.assertEqual(moved.read_bytes(), b"partial")
    self.assertIn("No files were removed", errors.getvalue())

  def test_catalog_limit_for_plain_gzip_and_concatenated_members(self) -> None:
    self.assertEqual(BUNDLE.MAX_CATALOG_BYTES, 67_108_864)
    self.assertLess(33_929_696, BUNDLE.MAX_CATALOG_BYTES)
    limit = 12
    exact = b"abcdefghijkl"
    over = exact + b"m"
    cases = [
      ("plain-exact.csv", exact, exact, True),
      ("plain-over.csv", over, exact, False),
      ("gzip-exact.csv.gz", gzip.compress(exact, mtime=0), exact, True),
      ("gzip-over.csv.gz", gzip.compress(over, mtime=0), exact, False),
      ("concat-exact.csv.gz", gzip.compress(exact[:6], mtime=0)
       + gzip.compress(exact[6:], mtime=0), exact, True),
      ("concat-over.csv.gz", gzip.compress(exact[:6], mtime=0)
       + gzip.compress(over[6:], mtime=0), exact, False),
    ]
    with self.in_checkout():
      with mock.patch.object(BUNDLE, "MAX_CATALOG_BYTES", limit):
        for index, (name, contents, expected, succeeds) in enumerate(cases):
          with self.subTest(name=name):
            catalog = self.inputs / name
            catalog.write_bytes(contents)
            output = self.output.parent / f"bounded-{index}"
            output.mkdir(mode=0o700)
            errors = io.StringIO()
            with contextlib.redirect_stderr(errors):
              if succeeds:
                self.assertEqual(BUNDLE.create_bundle(catalog, self.binary, output, []), output)
              else:
                with self.assertRaisesRegex(BUNDLE.BundleError, "exceeds 12 bytes; use a smaller"):
                  BUNDLE.create_bundle(catalog, self.binary, output, [])
            self.assertEqual((output / "hyg-v4.4.csv").read_bytes(), expected)
            self.assertEqual((output / "launch-starfield").exists(), succeeds)
            if not succeeds:
              self.assertIn("No files were removed", errors.getvalue())

  def test_catalog_copy_reads_only_the_first_byte_over_the_limit(self) -> None:
    plain = io.BytesIO(b"abcdefghij")
    output = io.BytesIO()
    with self.assertRaisesRegex(BUNDLE.BundleError, "exceeds 4 bytes"):
      BUNDLE.copy_catalog(plain, output, 4)
    self.assertEqual(plain.tell(), 5)
    self.assertEqual(output.getvalue(), b"abcd")

    compressed = io.BytesIO(gzip.compress(b"abc", mtime=0) + gzip.compress(b"defghij", mtime=0))
    with gzip.GzipFile(fileobj=compressed, mode="rb") as source:
      output = io.BytesIO()
      with self.assertRaisesRegex(BUNDLE.BundleError, "exceeds 4 bytes"):
        BUNDLE.copy_catalog(source, output, 4)
      self.assertEqual(source.tell(), 5)
      self.assertEqual(output.getvalue(), b"abcd")

  def test_launcher_mode_stripped_by_umask_fails_without_cleanup(self) -> None:
    result = self.run_bundle(output=self.output, umask=0o111)
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("launcher mode was restricted by umask", result.stderr)
    self.assertIn("No files were removed", result.stderr)
    self.assertEqual({path.name for path in self.output.iterdir()}, {
      "hyg-v4.4.csv", "chipviz-renderer", "HYG-NOTICE.txt", "launch-starfield",
    })
    launcher = self.output / "launch-starfield"
    self.assertEqual(stat.S_IMODE(launcher.stat().st_mode), 0o600)
    self.assertEqual(launcher.read_bytes(), b"")
    self.assertEqual((self.output / "hyg-v4.4.csv").read_bytes(), CSV)

  def test_launcher_is_created_after_all_other_bundle_files(self) -> None:
    upstream = self.inputs / "upstream notice.txt"
    upstream.write_bytes(b"synthetic attribution\n")
    with self.in_checkout():
      with mock.patch.object(BUNDLE, "create_file", wraps=BUNDLE.create_file) as create:
        BUNDLE.create_bundle(self.catalog, self.binary, self.output, [upstream])
    self.assertEqual([call.args[1] for call in create.call_args_list], [
      "hyg-v4.4.csv", "chipviz-renderer", "HYG-UPSTREAM-NOTICE-1.txt",
      "HYG-NOTICE.txt", "launch-starfield",
    ])

  def test_nonempty_replacement_before_destination_open_is_untouched(self) -> None:
    moved = self.output.parent / "original-before-open"
    victim = self.output.parent / "victim-before-open"
    victim.mkdir(mode=0o700)
    (victim / "keep.txt").write_bytes(b"existing user file")
    victim_mode = stat.S_IMODE(victim.stat().st_mode)
    real_open = BUNDLE.os.open
    injected = False

    def replace_before_open(path: object, flags: int, *args: object, **kwargs: object) -> int:
      nonlocal injected
      if path == self.output.name and flags & os.O_DIRECTORY and not injected:
        injected = True
        self.output.rename(moved)
        victim.rename(self.output)
      return real_open(path, flags, *args, **kwargs)

    with self.in_checkout():
      with mock.patch.object(BUNDLE.os, "open", side_effect=replace_before_open):
        with self.assertRaisesRegex(BUNDLE.BundleError, "must be empty"):
          BUNDLE.create_bundle(self.catalog, self.binary, self.output, [])
    self.assertTrue(injected)
    self.assertEqual((self.output / "keep.txt").read_bytes(), b"existing user file")
    self.assertEqual({path.name for path in self.output.iterdir()}, {"keep.txt"})
    self.assertEqual(stat.S_IMODE(self.output.stat().st_mode), victim_mode)
    self.assertEqual(list(moved.iterdir()), [])

  def test_open_directory_remains_pinned_when_replaced_or_renamed(self) -> None:
    real_create = BUNDLE.create_file
    for kind in ("directory", "symlink"):
      with self.subTest(kind=kind):
        output = self.output.parent / f"renamed-{kind}"
        output.mkdir(mode=0o700)
        moved = self.output.parent / f"opened-{kind}"
        victim = self.output.parent / f"victim-{kind}"
        victim.mkdir(mode=0o700)
        (victim / "keep.txt").write_bytes(b"existing user file")
        victim_mode = stat.S_IMODE(victim.stat().st_mode)
        injected = False

        def replace_before_write(fd: int, name: str, mode: int) -> BinaryIO:
          nonlocal injected
          if not injected:
            injected = True
            output.rename(moved)
            if kind == "directory":
              victim.rename(output)
            else:
              output.symlink_to(victim, target_is_directory=True)
          return real_create(fd, name, mode)

        with self.in_checkout():
          with mock.patch.object(BUNDLE, "create_file", side_effect=replace_before_write):
            self.assertEqual(BUNDLE.create_bundle(self.catalog, self.binary, output, []), output)
        self.assertTrue(injected)
        self.assertEqual((output / "keep.txt").read_bytes(), b"existing user file")
        self.assertEqual({path.name for path in output.iterdir()}, {"keep.txt"})
        self.assertEqual(stat.S_IMODE(output.stat().st_mode), victim_mode)
        self.assertEqual((moved / "hyg-v4.4.csv").read_bytes(), CSV)
        self.assertTrue((moved / "launch-starfield").is_file())
        self.assertEqual(output.is_symlink(), kind == "symlink")

  def test_late_file_and_symlink_collisions_never_overwrite_user_files(self) -> None:
    real_create = BUNDLE.create_file
    names = (BUNDLE.CATALOG_NAME, BUNDLE.BINARY_NAME, BUNDLE.NOTICE_NAME, BUNDLE.LAUNCHER_NAME)
    for index, collision_name in enumerate(names):
      for kind in ("file", "symlink"):
        with self.subTest(name=collision_name, kind=kind):
          output = self.output.parent / f"collision-{index}-{kind}"
          output.mkdir(mode=0o700)
          victim = self.inputs / f"collision-victim-{index}-{kind}"
          victim.write_bytes(b"existing user file")
          collision = output / collision_name
          injected = False

          def create_with_collision(fd: int, name: str, mode: int) -> BinaryIO:
            nonlocal injected
            if name == collision_name and not injected:
              injected = True
              if kind == "file":
                collision.write_bytes(b"existing user file")
              else:
                collision.symlink_to(victim)
            return real_create(fd, name, mode)

          errors = io.StringIO()
          with self.in_checkout():
            with mock.patch.object(BUNDLE, "create_file", side_effect=create_with_collision):
              with contextlib.redirect_stderr(errors):
                with self.assertRaisesRegex(BUNDLE.BundleError, "refusing to overwrite"):
                  BUNDLE.create_bundle(self.catalog, self.binary, output, [])
          self.assertTrue(injected)
          self.assertEqual(collision.read_bytes(), b"existing user file")
          self.assertEqual(victim.read_bytes(), b"existing user file")
          self.assertEqual(collision.is_symlink(), kind == "symlink")
          self.assertIn("No files were removed", errors.getvalue())

  def test_utility_never_chmods_or_deletes_on_success_or_failure(self) -> None:
    failed = self.output.parent / "write-failure"
    failed.mkdir(mode=0o700)
    errors = io.StringIO()
    with contextlib.ExitStack() as stack:
      stack.enter_context(self.in_checkout())
      for operation in ("chmod", "fchmod", "unlink", "remove", "rmdir"):
        stack.enter_context(mock.patch.object(
          BUNDLE.os, operation, side_effect=AssertionError(f"unexpected {operation}"),
        ))
      BUNDLE.create_bundle(self.catalog, self.binary, self.output, [])
      with mock.patch.object(BUNDLE, "copy_catalog", side_effect=OSError("simulated failure")):
        with contextlib.redirect_stderr(errors):
          with self.assertRaisesRegex(OSError, "simulated failure"):
            BUNDLE.create_bundle(self.catalog, self.binary, failed, [])
    self.assertTrue((self.output / "launch-starfield").is_file())
    self.assertTrue((failed / "hyg-v4.4.csv").is_file())
    self.assertIn("No files were removed", errors.getvalue())


if __name__ == "__main__":
  unittest.main()
