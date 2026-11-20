import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile


ROOT = pathlib.Path(__file__).resolve().parents[1]


class ReleasePackagingTests(unittest.TestCase):
  def setUp(self) -> None:
    self.temp = tempfile.TemporaryDirectory()
    self.addCleanup(self.temp.cleanup)
    self.root = pathlib.Path(self.temp.name) / "fixture"
    self.root.mkdir()
    for name in ("scripts/package-release.sh", "scripts/check-release-assets.py"):
      destination = self.root / name
      destination.parent.mkdir(parents=True, exist_ok=True)
      shutil.copy(ROOT / name, destination)
    for name in ("LICENSE", "COMMERCIAL.md", "THIRD_PARTY_NOTICES/roms.md"):
      self.write(name, "fixture")
    # Fake the heavy tools and producers, but execute the real packaging script,
    # shell file operations, zip creation, and release guard in this private root.
    producer = """import pathlib, sys
output = pathlib.Path(sys.argv[sys.argv.index("--output") + 1])
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text("fixture")
"""
    for name in ("host/bridge/chipviz_bridge.py", "host/bridge/chipviz_encode.py",
                 "host/bridge/n64_joybus.py", "host/bridge/usb_hid.py",
                 "shared/tools/cvz_to_c.py"):
      self.write(name, producer)
    self.write("shared/tools/build_homebrew.py", """
import pathlib, sys
output = pathlib.Path(sys.argv[sys.argv.index("--output") + 1])
excluded = [sys.argv[i + 1] for i, arg in enumerate(sys.argv[:-1]) if arg == "--exclude-target"]
for target, ext in (("c64", "prg"), ("snes", "sfc"), ("genesis", "md"), ("nes", "nes"), ("sms", "sms")):
  if target not in excluded:
    (output / f"chipviz-{target}.{ext}").write_text("fixture")
(output / "homebrew-status.json").write_text("fixture")
""")
    tools = self.root / "bin"
    tools.mkdir()
    (tools / "python3").symlink_to(sys.executable)
    fake = f"""#!{sys.executable}
import os, pathlib, sys
tool = pathlib.Path(sys.argv[0]).name
with open(os.environ["CALL_LOG"], "a", encoding="utf-8") as log:
  log.write(tool + " " + " ".join(sys.argv[1:]) + "\\n")
if tool == "make":
  target = sys.argv[sys.argv.index("-C") + 1].split("/")[1]
  extensions = {{"c64": "prg", "gba": "gba", "n64": "z64", "snes": "sfc",
                 "gb": "gb", "genesis": "md", "nes": "nes", "sms": "sms"}}
  path = pathlib.Path(f"cores/{{target}}/homebrew/chipviz-{{target}}.{{extensions[target]}}")
  path.parent.mkdir(parents=True, exist_ok=True)
  path.write_text("fixture")
if tool == "cargo":
  for name in ("chipviz-pi5", "chipviz-m1"):
    path = pathlib.Path("renderers/modern/target/release") / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("fixture")
  if os.environ.get("INJECT_SNES"):
    pathlib.Path("dist/assets/accidental.SMC").write_text("fixture")
"""
    for name in ("mise", "make", "cargo"):
      self.write(f"bin/{name}", fake)
      (tools / name).chmod(0o755)
    self.env = {key: value for key, value in os.environ.items()
                if key not in ("PVSNESLIB_HOME", "DEVKITARM", "N64_INST")}
    self.env.update({"PATH": f"{tools}:/usr/bin:/bin", "CALL_LOG": str(self.root / "calls")})

  def write(self, name: str, text: str) -> pathlib.Path:
    path = self.root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path

  def package(self) -> subprocess.CompletedProcess:
    return subprocess.run(
      ["bash", str(self.root / "scripts/package-release.sh")],
      cwd=self.root, env=self.env, capture_output=True, text=True, check=False,
    )

  def guard(self) -> subprocess.CompletedProcess:
    return subprocess.run(
      [sys.executable, str(self.root / "scripts/check-release-assets.py"),
       str(self.root / "dist/assets"), str(self.root / "dist/chipviz-assets.zip")],
      capture_output=True, text=True, check=False,
    )

  def test_no_sdk_and_sdk_with_stale_files(self) -> None:
    for sdk in (False, True):
      with self.subTest(sdk=sdk):
        if sdk:
          self.env["PVSNESLIB_HOME"] = str(self.root / "sdk")
          self.write("sdk/devkitsnes/snes_rules", "fixture")
        else:
          self.env.pop("PVSNESLIB_HOME", None)
        self.write("dist/assets/chipviz-snes.sfc", "stale")
        self.write("cores/snes/homebrew/chipviz-snes.sfc", "stale SDK ROM")
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        assets = self.root / "dist/assets"
        names = {path.name for path in assets.iterdir()}
        self.assertNotIn("chipviz-snes.sfc", names)
        self.assertTrue({"chipviz-c64.prg", "chipviz-genesis.md", "chipviz-nes.nes",
                         "chipviz-sms.sms", "chipviz-pi5", "LICENSE"} <= names)
        with zipfile.ZipFile(self.root / "dist/chipviz-assets.zip") as archive:
          self.assertEqual(set(archive.namelist()), names)
        self.assertNotIn("make -C cores/snes/homebrew", (self.root / "calls").read_text())
        self.assertEqual((self.root / "cores/snes/homebrew/chipviz-snes.sfc").read_text(), "stale SDK ROM")

  def test_packaging_rejects_late_insertion(self) -> None:
    self.env["INJECT_SNES"] = "1"
    result = self.package()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("accidental.SMC", result.stderr)

  def test_guard_rejects_individual_and_archive_insertions(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    self.write("dist/assets/other.SfC", "fixture")
    self.assertNotEqual(self.guard().returncode, 0)
    (self.root / "dist/assets/other.SfC").unlink()
    with zipfile.ZipFile(self.root / "dist/chipviz-assets.zip", "a") as archive:
      archive.writestr("nested/chipviz-snes.bin", "fixture")
    result = self.guard()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("nested/chipviz-snes.bin", result.stderr)
    (self.root / "dist/chipviz-assets.zip").unlink()
    self.assertNotEqual(self.guard().returncode, 0)

  def test_guard_rejects_unknown_asset_names(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    self.write("dist/assets/unexpected.bin", "fixture")
    result = self.guard()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("Unexpected release assets: unexpected.bin", result.stderr)
    (self.root / "dist/assets/unexpected.bin").unlink()
    with zipfile.ZipFile(self.root / "dist/chipviz-assets.zip", "a") as archive:
      archive.writestr("unexpected.bin", "fixture")
    self.assertNotEqual(self.guard().returncode, 0)
    self.write("dist/chipviz-assets.zip", "not a zip")
    self.assertNotEqual(self.guard().returncode, 0)

  def test_guard_rejects_duplicate_archive_entry(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    with zipfile.ZipFile(self.root / "dist/chipviz-assets.zip", "a") as archive:
      with self.assertWarnsRegex(UserWarning, "Duplicate name"):
        archive.writestr("chipviz-genesis.md", b"SNES seed")
    result = self.guard()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("Duplicate release archive entries", result.stderr)

  def test_guard_rejects_archive_content_mismatch(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    assets = self.root / "dist/assets"
    with zipfile.ZipFile(self.root / "dist/chipviz-assets.zip", "w") as archive:
      for path in sorted(assets.iterdir()):
        if path.name == "chipviz-genesis.md":
          archive.writestr(path.name, b"SNES seed")
        else:
          archive.write(path, path.name)
    result = self.guard()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("Release archive content differs from individual asset: chipviz-genesis.md", result.stderr)

  def test_guard_rejects_post_archive_individual_modification(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    self.write("dist/assets/chipviz-genesis.md", "modified after archive creation")
    result = self.guard()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("Release archive content differs from individual asset: chipviz-genesis.md", result.stderr)

  def test_guard_rejects_missing_or_added_allowed_entry(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    self.write("dist/assets/chipviz-gba.gba", "added after archive creation")
    result = self.guard()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("Release archive entries do not match individual assets", result.stderr)
    (self.root / "dist/assets/chipviz-gba.gba").unlink()
    (self.root / "dist/assets/chipviz-genesis.md").unlink()
    result = self.guard()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("Release archive entries do not match individual assets", result.stderr)

  def test_workflow_checks_both_uploads_after_rebuilding_archive(self) -> None:
    workflow = (ROOT / ".github/workflows/release-assets.yml").read_text(encoding="utf-8")
    self.assertNotIn("cores/snes/homebrew", workflow)
    self.assertNotIn("PVSNESLIB_HOME", workflow)
    self.assertLess(workflow.index("name: Rebuild combined archive"), workflow.index("name: Upload individual assets"))
    for name in ("individual assets", "combined zip"):
      step = workflow.split(f"- name: Upload {name}", 1)[1].split("\n      - name:", 1)[0]
      self.assertIn("python3 scripts/check-release-assets.py dist/assets dist/chipviz-assets.zip\n"
                    "          gh release upload ", step)


if __name__ == "__main__":
  unittest.main()
