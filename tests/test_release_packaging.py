import os
import pathlib
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile


ROOT = pathlib.Path(__file__).resolve().parents[1]
RENDERER_NAMES = ("chipviz-pi5", "chipviz-m1")


def elf64_header(object_type: int = 2, osabi: int = 0) -> bytes:
  # A header-only fixture, not a runnable renderer.
  return struct.pack(
    "<16sHHIQQQIHHHHHH", b"\x7fELF\x02\x01\x01" + bytes((osabi,)) + bytes(8),
    object_type, 62, 1, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0,
  )


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
  if os.environ.get("FAKE_MAC_RENDERERS"):
    data = bytes.fromhex("cffaedfe") + bytes(28)
  else:
    data = bytes.fromhex("{elf64_header().hex()}")
  for name in ("chipviz-pi5", "chipviz-m1"):
    path = pathlib.Path("renderers/modern/target/release") / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
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

  def guard(self, require_renderers: bool = False) -> subprocess.CompletedProcess:
    command = [sys.executable, str(self.root / "scripts/check-release-assets.py")]
    if require_renderers:
      command.append("--require-x86-64-renderers")
    command.extend([str(self.root / "dist/assets"), str(self.root / "dist/chipviz-assets.zip")])
    return subprocess.run(
      command, capture_output=True, text=True, check=False,
    )

  def assert_guard_error(self, message: str) -> None:
    for require_renderers in (False, True):
      with self.subTest(require_renderers=require_renderers):
        result = self.guard(require_renderers)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(message, result.stderr)

  def set_renderer_assets(self, headers: dict[str, bytes]) -> None:
    assets = self.root / "dist/assets"
    for name in RENDERER_NAMES:
      path = assets / name
      if name in headers:
        path.write_bytes(headers[name])
      else:
        path.unlink(missing_ok=True)
    with zipfile.ZipFile(self.root / "dist/chipviz-assets.zip", "w") as archive:
      for path in sorted(assets.iterdir()):
        archive.write(path, path.name)

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
    self.assert_guard_error("SNES release assets are withheld (#109): other.SfC")
    (self.root / "dist/assets/other.SfC").unlink()
    with zipfile.ZipFile(self.root / "dist/chipviz-assets.zip", "a") as archive:
      archive.writestr("nested/chipviz-snes.bin", "fixture")
    self.assert_guard_error("SNES release assets are withheld (#109): nested/chipviz-snes.bin")
    (self.root / "dist/chipviz-assets.zip").unlink()
    self.assert_guard_error("Cannot inspect release assets:")

  def test_guard_rejects_unknown_asset_names(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    self.write("dist/assets/unexpected.bin", "fixture")
    self.assert_guard_error("Unexpected release assets: unexpected.bin")
    (self.root / "dist/assets/unexpected.bin").unlink()
    with zipfile.ZipFile(self.root / "dist/chipviz-assets.zip", "a") as archive:
      archive.writestr("unexpected.bin", "fixture")
    self.assert_guard_error("Unexpected release assets: unexpected.bin")
    self.write("dist/chipviz-assets.zip", "not a zip")
    self.assert_guard_error("Cannot inspect release assets:")

  def test_guard_rejects_duplicate_archive_entry(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    with zipfile.ZipFile(self.root / "dist/chipviz-assets.zip", "a") as archive:
      with self.assertWarnsRegex(UserWarning, "Duplicate name"):
        archive.writestr("chipviz-genesis.md", b"SNES seed")
    self.assert_guard_error("Duplicate release archive entries")

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
    self.assert_guard_error("Release archive content differs from individual asset: chipviz-genesis.md")

  def test_guard_rejects_post_archive_individual_modification(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    self.write("dist/assets/chipviz-genesis.md", "modified after archive creation")
    self.assert_guard_error("Release archive content differs from individual asset: chipviz-genesis.md")

  def test_guard_rejects_missing_or_added_allowed_entry(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    self.write("dist/assets/chipviz-gba.gba", "added after archive creation")
    self.assert_guard_error("Release archive entries do not match individual assets")
    (self.root / "dist/assets/chipviz-gba.gba").unlink()
    (self.root / "dist/assets/chipviz-genesis.md").unlink()
    self.assert_guard_error("Release archive entries do not match individual assets")

  def test_default_packaging_accepts_mac_like_renderers(self) -> None:
    self.env["FAKE_MAC_RENDERERS"] = "1"
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    mac_like_header = bytes.fromhex("cffaedfe") + bytes(28)
    with zipfile.ZipFile(self.root / "dist/chipviz-assets.zip") as archive:
      for name in RENDERER_NAMES:
        self.assertEqual((self.root / "dist/assets" / name).read_bytes(), mac_like_header)
        self.assertEqual(archive.read(name), mac_like_header)
    result = self.guard()
    self.assertEqual(result.returncode, 0, result.stderr)
    result = self.guard(require_renderers=True)
    self.assertEqual(result.returncode, 1, result.stderr)
    self.assertIn("Invalid ELF64 little-endian x86-64 executable/shared-object header:", result.stderr)

  def test_guard_accepts_x86_64_executable_and_pie_headers(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    for pi_type, m1_type in ((2, 2), (2, 3), (3, 2), (3, 3)):
      with self.subTest(pi_type=pi_type, m1_type=m1_type):
        self.set_renderer_assets({
          "chipviz-pi5": elf64_header(pi_type),
          "chipviz-m1": elf64_header(m1_type, osabi=3),
        })
        result = self.guard(require_renderers=True)
        self.assertEqual(result.returncode, 0, result.stderr)

  def test_guard_requires_both_renderers_when_opted_in(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    for present in ((), ("chipviz-pi5",), ("chipviz-m1",)):
      with self.subTest(present=present):
        self.set_renderer_assets({name: elf64_header() for name in present})
        result = self.guard()
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.guard(require_renderers=True)
        self.assertEqual(result.returncode, 1, result.stderr)
        missing = sorted(set(RENDERER_NAMES) - set(present))
        self.assertIn(f"Missing renderer assets: {', '.join(missing)}", result.stderr)

  def test_guard_rejects_invalid_renderer_headers(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    valid = elf64_header()

    def replace(offset: int, value: bytes) -> bytes:
      return valid[:offset] + value + valid[offset + len(value):]

    big_endian = struct.pack(
      ">16sHHIQQQIHHHHHH", b"\x7fELF\x02\x02\x01" + bytes(9),
      2, 62, 1, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0,
    )
    invalid = {
      "bad magic": replace(0, b"\x00"),
      "32-bit class": replace(4, b"\x01"),
      "invalid class": replace(4, b"\x00"),
      "big-endian": big_endian,
      "invalid endianness": replace(5, b"\x00"),
      "ident version zero": replace(6, b"\x00"),
      "ident version future": replace(6, b"\x02"),
      "no object type": replace(16, struct.pack("<H", 0)),
      "relocatable object": replace(16, struct.pack("<H", 1)),
      "core dump": replace(16, struct.pack("<H", 4)),
      "unknown object type": replace(16, struct.pack("<H", 0xffff)),
      "x86 machine": replace(18, struct.pack("<H", 3)),
      "arm64 machine": replace(18, struct.pack("<H", 183)),
      "header version zero": replace(20, struct.pack("<I", 0)),
      "header version future": replace(20, struct.pack("<I", 2)),
      "zero header size": replace(52, struct.pack("<H", 0)),
      "short header size": replace(52, struct.pack("<H", 63)),
      "oversized header": replace(52, struct.pack("<H", 65)),
    }
    for length in (0, 3, 6, 15, 17, 19, 23, 24, 51, 52, 53, 54, 63):
      invalid[f"truncated to {length} bytes"] = valid[:length]
    for name in RENDERER_NAMES:
      for case, bad_header in invalid.items():
        with self.subTest(renderer=name, case=case):
          headers = {renderer: valid for renderer in RENDERER_NAMES}
          headers[name] = bad_header
          self.set_renderer_assets(headers)
          result = self.guard(require_renderers=True)
          self.assertEqual(result.returncode, 1, result.stderr)
          self.assertIn(
            f"Invalid ELF64 little-endian x86-64 executable/shared-object header: {name}",
            result.stderr,
          )

  def test_renderer_headers_do_not_replace_archive_content_equality(self) -> None:
    result = self.package()
    self.assertEqual(result.returncode, 0, result.stderr)
    headers = {name: elf64_header() for name in RENDERER_NAMES}
    for name in RENDERER_NAMES:
      with self.subTest(renderer=name):
        self.set_renderer_assets(headers)
        result = self.guard(require_renderers=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        (self.root / "dist/assets" / name).write_bytes(headers[name] + b"changed")
        self.assert_guard_error(f"Release archive content differs from individual asset: {name}")

  def test_workflow_checks_both_uploads_after_rebuilding_archive(self) -> None:
    workflow = (ROOT / ".github/workflows/release-assets.yml").read_text(encoding="utf-8")
    self.assertNotIn("cores/snes/homebrew", workflow)
    self.assertNotIn("PVSNESLIB_HOME", workflow)
    self.assertLess(workflow.index("name: Rebuild combined archive"), workflow.index("name: Upload individual assets"))
    for name in ("individual assets", "combined zip"):
      step = workflow.split(f"- name: Upload {name}", 1)[1].split("\n      - name:", 1)[0]
      self.assertIn("python3 scripts/check-release-assets.py --require-x86-64-renderers "
                    "dist/assets dist/chipviz-assets.zip\n"
                    "          gh release upload ", step)


if __name__ == "__main__":
  unittest.main()
