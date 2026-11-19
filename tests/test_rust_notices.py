import contextlib
import io
import os
import pathlib
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "shared" / "tools"))

import rust_notices


def write(path: pathlib.Path, text: str) -> None:
  path.parent.mkdir(parents=True, exist_ok=True)
  path.write_text(text, encoding="utf-8")


class RustNoticesTests(unittest.TestCase):
  def setUp(self) -> None:
    self.tmp = tempfile.TemporaryDirectory()
    self.addCleanup(self.tmp.cleanup)
    root = pathlib.Path(self.tmp.name)
    self.fallback = root / "licenses"
    write(self.fallback / "MIT.txt", "MIT License\n\n" + rust_notices.MIT_TEMPLATE_LINE + "\n\nPermission granted.\n")
    write(self.fallback / "Apache-2.0.txt", "Apache License 2.0 text\n")
    packages = []

    def package(name: str, licence: str, files: dict[str, str], license_file: str | None = None) -> str:
      crate = root / "registry" / f"{name}-1.0.0"
      write(crate / "Cargo.toml", "[package]\n")
      for relative, text in files.items():
        write(crate / relative, text)
      package_id = f"registry#{name}@1.0.0"
      packages.append({
        "id": package_id,
        "name": name,
        "version": "1.0.0",
        "license": licence,
        "license_file": license_file,
        "repository": f"https://example.invalid/{name}",
        "manifest_path": str(crate / "Cargo.toml"),
      })
      return package_id

    root_id = package("app", "PolyForm-Noncommercial-1.0.0", {})
    linked = package("linked", "MIT", {"LICENSES/MIT.txt": "Copyright linked authors\n"})
    custom = package("custom", "", {"COPYRIGHT.custom": "ignored by name only\n", "terms.txt": "Custom terms\n"}, "terms.txt")
    bare = package("bare", "MIT OR Apache-2.0", {})
    build = package("build-only", "MIT", {"LICENSE": "build tool\n"})
    dev = package("dev-only", "MIT", {"LICENSE": "dev tool\n"})
    normal = [{"kind": None, "target": None}]
    nodes = [
      {"id": root_id, "deps": [
        {"pkg": linked, "dep_kinds": normal},
        {"pkg": build, "dep_kinds": [{"kind": "build", "target": None}]},
        {"pkg": dev, "dep_kinds": [{"kind": "dev", "target": None}]},
      ]},
      {"id": linked, "deps": [{"pkg": custom, "dep_kinds": normal}, {"pkg": bare, "dep_kinds": normal}]},
      {"id": custom, "deps": []},
      {"id": bare, "deps": []},
      {"id": build, "deps": [{"pkg": dev, "dep_kinds": normal}]},
      {"id": dev, "deps": []},
    ]
    self.metadata = {"packages": packages, "workspace_members": [root_id], "resolve": {"nodes": nodes}}

  def test_only_normal_edges_are_shipped(self) -> None:
    names = [package["name"] for package in rust_notices.shipped_packages(self.metadata)]
    self.assertEqual(names, ["bare", "custom", "linked"])

  def test_license_file_and_licenses_dir_are_collected(self) -> None:
    packages = {package["name"]: package for package in self.metadata["packages"]}
    linked = [path.name for path in rust_notices.notice_files(packages["linked"])]
    custom = [path.name for path in rust_notices.notice_files(packages["custom"])]
    self.assertEqual(linked, ["MIT.txt"])
    self.assertEqual(custom, ["COPYRIGHT.custom", "terms.txt"])

  def test_fallback_text_for_crate_without_licence_files(self) -> None:
    content = rust_notices.render(self.metadata, self.fallback)
    self.assertIn("bare 1.0.0 (fallback SPDX Apache-2.0 text)", content)
    self.assertIn("Apache License 2.0 text", content)
    self.assertIn("## Known gaps\n\nNone.", content)
    self.assertNotIn("build tool", content)
    self.assertNotIn("dev tool", content)

  def test_mit_only_crate_without_holder_is_a_known_gap(self) -> None:
    packages = {package["name"]: package for package in self.metadata["packages"]}
    packages["bare"]["license"] = "MIT"
    content = rust_notices.render(self.metadata, self.fallback)
    self.assertIn("- bare 1.0.0: no licence file in package", content)

  def run_main(self, argv: list[str], metadata: object, ci: bool) -> tuple[int, str]:
    env = {key: value for key, value in os.environ.items() if key != "CI"}
    if ci:
      env["CI"] = "true"
    if isinstance(metadata, Exception):
      fake = mock.Mock(side_effect=metadata)
    else:
      fake = mock.Mock(return_value=metadata)
    output = io.StringIO()
    with mock.patch.dict(os.environ, env, clear=True), \
        mock.patch.object(rust_notices, "cargo_metadata", fake), \
        mock.patch.object(rust_notices, "FALLBACK_DIR", self.fallback), \
        contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
      code = rust_notices.main(argv)
    return code, output.getvalue()

  def test_check_detects_stale_and_current_output(self) -> None:
    target = pathlib.Path(self.tmp.name) / "rust.md"
    write(target, "stale\n")
    code, text = self.run_main(["--check", "--output", str(target)], self.metadata, ci=False)
    self.assertEqual(code, 1)
    self.assertIn("is stale", text)
    write(target, rust_notices.render(self.metadata, self.fallback))
    code, text = self.run_main(["--check", "--output", str(target)], self.metadata, ci=False)
    self.assertEqual(code, 0)
    self.assertIn("chipviz Rust notices OK", text)

  def test_check_skips_without_sources_but_fails_in_ci(self) -> None:
    target = pathlib.Path(self.tmp.name) / "rust.md"
    missing = rust_notices.SourcesUnavailable("no registry")
    code, text = self.run_main(["--check", "--output", str(target)], missing, ci=False)
    self.assertEqual(code, 0)
    self.assertIn("check skipped", text)
    code, text = self.run_main(["--check", "--output", str(target)], missing, ci=True)
    self.assertEqual(code, 1)
    self.assertIn("required when CI is set", text)


if __name__ == "__main__":
  unittest.main()
