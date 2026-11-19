import contextlib
import hashlib
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
    self.package = package
    self.normal = normal

  def add_shipped(self, name: str, licence: str, files: dict[str, str]) -> dict:
    package_id = self.package(name, licence, files)
    nodes = self.metadata["resolve"]["nodes"]
    nodes[0]["deps"].append({"pkg": package_id, "dep_kinds": self.normal})
    nodes.append({"id": package_id, "deps": []})
    return next(package for package in self.metadata["packages"] if package["id"] == package_id)

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
    self.assertNotIn(rust_notices.MIT_TEMPLATE_LINE, content)
    self.assertNotIn("bare 1.0.0 (fallback SPDX MIT text)", content)
    self.assertNotIn("build tool", content)
    self.assertNotIn("dev tool", content)

  def test_mit_only_crate_without_holder_is_a_known_gap(self) -> None:
    packages = {package["name"]: package for package in self.metadata["packages"]}
    packages["bare"]["license"] = "MIT"
    content = rust_notices.render(self.metadata, self.fallback)
    self.assertIn("- bare 1.0.0: no licence file in package", content)

  def test_mit_copyright_line_is_substituted(self) -> None:
    packages = {package["name"]: package for package in self.metadata["packages"]}
    packages["bare"]["license"] = "MIT"
    holders = {("bare", "1.0.0"): ("Copyright (c) 2020 Bare Authors", "https://example.invalid/bare/LICENSE")}
    with mock.patch.object(rust_notices, "MIT_COPYRIGHT", holders):
      content = rust_notices.render(self.metadata, self.fallback)
    self.assertIn("Copyright (c) 2020 Bare Authors\n\nPermission granted.", content)
    self.assertIn("copyright line from https://example.invalid/bare/LICENSE", content)
    self.assertNotIn(rust_notices.MIT_TEMPLATE_LINE, content)
    self.assertIn("## Known gaps\n\nNone.", content)

  def test_nested_licence_files_and_protocol_copyrights(self) -> None:
    vendored = self.add_shipped("vendored", "MIT", {
      "LICENSE": "Copyright vendored authors\n",
      "upstream/COPYING.LIB": "GNU LESSER GENERAL PUBLIC LICENSE\n",
      "upstream/LICENSES/LGPL-2.1-or-later.txt": "LGPL text\n",
      "src/copying.rs": "fn copy() {}\n",
      "upstream/src/b.xml": "<protocol name=\"b\">\n  <copyright>\n    Copyright 2021 B &amp; Co\n\n    SPDX-License-Identifier: LGPL-2.1-or-later\n  </copyright>\n</protocol>\n",
      "upstream/src/a.xml": "<protocol name=\"a\"><copyright><![CDATA[\n    Copyright 2020 A\n      indented\n  ]]></copyright></protocol>\n",
      "upstream/src/plain.xml": "<protocol name=\"plain\"/>\n",
    })
    files = [path.relative_to(pathlib.Path(vendored["manifest_path"]).parent).as_posix() for path in rust_notices.notice_files(vendored)]
    self.assertEqual(files, ["LICENSE", "upstream/COPYING.LIB", "upstream/LICENSES/LGPL-2.1-or-later.txt"])
    blocks = rust_notices.protocol_copyrights(pathlib.Path(vendored["manifest_path"]).parent)
    self.assertEqual(blocks, [
      ("upstream/src/a.xml", "Copyright 2020 A\n  indented"),
      ("upstream/src/b.xml", "Copyright 2021 B & Co\n\nSPDX-License-Identifier: LGPL-2.1-or-later"),
    ])
    content = rust_notices.render(self.metadata, self.fallback)
    self.assertIn("| vendored | 1.0.0 | MIT (also contains LGPL-2.1-or-later; 2 per-protocol notices) |", content)
    self.assertIn("vendored 1.0.0 (upstream/COPYING.LIB)", content)
    self.assertIn("== upstream/src/a.xml ==\nCopyright 2020 A", content)
    self.assertIn("- vendored 1.0.0: LGPL-2.1-or-later.", content)
    self.assertIn(rust_notices.LGPL_DECISION, content)
    self.assertNotIn("fn copy()", content)

  def test_licence_expression_parser(self) -> None:
    parse = rust_notices.licence_alternatives
    self.assertEqual(parse("(Apache-2.0 OR MIT) AND Unicode-3.0"), [["Apache-2.0", "Unicode-3.0"], ["MIT", "Unicode-3.0"]])
    self.assertEqual(parse("MIT OR Apache-2.0 WITH LLVM-exception"), [["MIT"], ["Apache-2.0", "LLVM-exception"]])
    self.assertEqual(parse("MIT AND Zlib OR Apache-2.0"), [["MIT", "Zlib"], ["Apache-2.0"]])
    self.assertEqual(parse("MIT/Apache-2.0"), [["MIT"], ["Apache-2.0"]])
    self.assertEqual(parse("(MIT OR Zlib) AND (Apache-2.0 OR MIT)"), [["MIT", "Apache-2.0"], ["MIT"], ["Zlib", "Apache-2.0"], ["Zlib", "MIT"]])
    self.assertEqual(parse(""), [])
    for broken in ("(MIT OR Apache-2.0", "MIT OR", "MIT Apache-2.0", "AND MIT", "MIT WITH AND", "MIT WITH (", "MIT WITH"):
      with self.assertRaises(ValueError, msg=broken):
        parse(broken)

  def test_grouped_and_term_without_text_is_a_gap(self) -> None:
    package = {"name": "ident", "version": "1.0.0", "license": "(Apache-2.0 OR MIT) AND Unicode-3.0"}
    texts, gap = rust_notices.fallback_texts(package, self.fallback)
    self.assertEqual([label for label, _ in texts], ["fallback SPDX Apache-2.0 text"])
    self.assertEqual(gap, "no licence file in package; no canonical text for Unicode-3.0")
    write(self.fallback / "Unicode-3.0.txt", "Unicode licence\n")
    texts, gap = rust_notices.fallback_texts(package, self.fallback)
    self.assertEqual([label for label, _ in texts], ["fallback SPDX Apache-2.0 text", "fallback SPDX Unicode-3.0 text"])
    self.assertIsNone(gap)

  def test_with_exception_needs_its_text_and_fewest_missing_wins(self) -> None:
    package = {"name": "llvm", "version": "1.0.0", "license": "MIT OR Apache-2.0 WITH LLVM-exception"}
    texts, gap = rust_notices.fallback_texts(package, self.fallback)
    self.assertEqual([label for label, _ in texts], ["fallback SPDX MIT text"])
    self.assertEqual(gap, "no licence file in package; no verified copyright holder for MIT")
    write(self.fallback / "LLVM-exception.txt", "LLVM exception\n")
    texts, gap = rust_notices.fallback_texts(package, self.fallback)
    self.assertEqual([label for label, _ in texts], ["fallback SPDX Apache-2.0 text", "fallback SPDX LLVM-exception text"])
    self.assertIsNone(gap)

  def test_declared_copyleft_is_listed_and_alternatives_are_optional(self) -> None:
    self.add_shipped("mpl", "MPL-2.0", {"LICENSE": "MPL text\n"})
    self.add_shipped("either", "MIT OR LGPL-2.1-or-later", {"LICENSE": "either text\n"})
    content = rust_notices.render(self.metadata, self.fallback)
    self.assertIn("## Copyleft components", content)
    self.assertIn("- mpl 1.0.0: MPL-2.0.", content)
    self.assertIn("- either 1.0.0: LGPL-2.1-or-later (optional: the crate may be used under an alternative licence).", content)

  def test_embedded_copyleft_is_required_even_if_also_declared_as_alternative(self) -> None:
    self.add_shipped("vendors-lgpl", "MIT OR LGPL-2.1-or-later", {
      "LICENSE": "vendors-lgpl text\n",
      "vendor/lib/COPYING": "SPDX-License-Identifier: LGPL-2.1-or-later\nLGPL-only component\n",
    })
    content = rust_notices.render(self.metadata, self.fallback)
    self.assertIn("- vendors-lgpl 1.0.0: LGPL-2.1-or-later.", content)
    self.assertNotIn("vendors-lgpl 1.0.0: LGPL-2.1-or-later (optional", content)

  def test_invalid_licence_expression_names_the_crate(self) -> None:
    self.add_shipped("broken", "MIT WITH AND", {"LICENSE": "broken text\n"})
    with self.assertRaisesRegex(ValueError, r"^broken 1\.0\.0: expected an exception id after WITH"):
      rust_notices.render(self.metadata, self.fallback)

  def test_attribution_only_notice_keeps_licence_fallback(self) -> None:
    self.add_shipped("noticed", "Apache-2.0", {"NOTICE": "Noticed Project\nCopyright 2024 Noticed Authors\n"})
    self.add_shipped("noticed-mit", "MIT", {"COPYRIGHT": "Copyright 2024 Someone\n"})
    content = rust_notices.render(self.metadata, self.fallback)
    self.assertIn("noticed 1.0.0 (NOTICE)", content)
    self.assertIn("noticed 1.0.0 (fallback SPDX Apache-2.0 text)", content)
    self.assertIn("noticed-mit 1.0.0 (COPYRIGHT)", content)
    self.assertIn("- noticed-mit 1.0.0: no licence file in package; no verified copyright holder for MIT.", content)

  def test_asset_notice_override_and_unverified_assets(self) -> None:
    write(self.fallback / "OFL-1.1.txt", "SIL OPEN FONT LICENSE\n")
    font = b"font bytes"
    package = self.add_shipped("fonty", "MIT", {"LICENSE": "fonty MIT\n"})
    directory = pathlib.Path(package["manifest_path"]).parent
    (directory / "src").mkdir(exist_ok=True)
    (directory / "src" / "Font.ttf").write_bytes(font)
    (directory / "src" / "Other.otf").write_bytes(b"other")
    (directory / "tests").mkdir(exist_ok=True)
    (directory / "tests" / "Test.ttf").write_bytes(b"test")
    override = {"versions": ("1.0.0",), "path": "src/Font.ttf", "sha256": hashlib.sha256(font).hexdigest(),
                "licence": "OFL-1.1", "copyright": "Copyright (c) 2020 Font Authors.", "source": "verified"}
    with mock.patch.object(rust_notices, "ASSET_NOTICES", {"fonty": [override]}):
      content = rust_notices.render(self.metadata, self.fallback)
      self.assertIn("| fonty | 1.0.0 | MIT (also contains OFL-1.1) |", content)
      self.assertIn("Copyright (c) 2020 Font Authors.\n\nSIL OPEN FONT LICENSE", content)
      self.assertIn("fonty 1.0.0 (embedded src/Font.ttf, OFL-1.1; verified)", content)
      self.assertIn("- fonty 1.0.0: embedded asset src/Other.otf has no verified notice.", content)
      self.assertNotIn("tests/Test.ttf", content)
      (directory / "src" / "Font.ttf").write_bytes(b"changed")
      content = rust_notices.render(self.metadata, self.fallback)
      self.assertIn("- fonty 1.0.0: embedded src/Font.ttf changed (SHA-256", content)
      self.assertNotIn("Font Authors", content)
    content = rust_notices.render(self.metadata, self.fallback)
    self.assertIn("- fonty 1.0.0: embedded asset src/Font.ttf has no verified notice.", content)

  def test_embedded_spdx_expressions_are_parsed(self) -> None:
    parse = rust_notices.embedded_licences
    self.assertEqual(parse("SPDX-License-Identifier: MIT AND LGPL-2.1-or-later\n", "f"), ({"MIT", "LGPL-2.1-or-later"}, set(), []))
    self.assertEqual(parse("/* SPDX-License-Identifier: (MIT OR Apache-2.0) AND GPL-2.0-only */", "f"),
                     ({"GPL-2.0-only"}, {"MIT", "Apache-2.0"}, []))
    self.assertEqual(parse("<!-- SPDX-License-Identifier: MIT OR LGPL-2.1-or-later -->", "f"), (set(), {"MIT", "LGPL-2.1-or-later"}, []))
    self.assertEqual(parse("# SPDX-License-Identifier: MIT ##", "f"), ({"MIT"}, set(), []))
    self.assertEqual(parse("SPDX-License-Identifier: MIT\nSPDX-License-Identifier: MIT OR Zlib\n", "f"), ({"MIT"}, {"Zlib"}, []))
    for broken in ("SPDX-License-Identifier: MIT AND (LGPL-2.1-or-later", "SPDX-License-Identifier:", "SPDX-License-Identifier: MIT WITH AND"):
      required, optional, gaps = parse(broken, "vendor/COPYING")
      self.assertEqual((required, optional), (set(), set()), broken)
      self.assertEqual(len(gaps), 1, broken)
      self.assertIn("unparseable SPDX-License-Identifier", gaps[0])
      self.assertIn("in vendor/COPYING", gaps[0])

  def test_embedded_and_expression_reaches_table_and_copyleft(self) -> None:
    self.add_shipped("mixed", "MIT", {
      "LICENSE": "mixed MIT\n",
      "vendor/COPYING": "SPDX-License-Identifier: MIT AND LGPL-2.1-or-later\n",
      "proto/a.xml": "<protocol><copyright>SPDX-License-Identifier: (BSD-3-Clause OR MPL-2.0)</copyright></protocol>",
      "proto/b.xml": "<protocol><copyright>SPDX-License-Identifier: MIT AND (</copyright></protocol>",
    })
    content = rust_notices.render(self.metadata, self.fallback)
    self.assertIn("| mixed | 1.0.0 | MIT (also contains BSD-3-Clause (optional), LGPL-2.1-or-later, MIT, MPL-2.0 (optional); 2 per-protocol notices) |", content)
    self.assertIn("- mixed 1.0.0: LGPL-2.1-or-later, MPL-2.0 (optional: the crate may be used under an alternative licence).", content)
    self.assertIn("- mixed 1.0.0: unparseable SPDX-License-Identifier 'MIT AND (' in proto/b.xml.", content)

  def test_fence_for_is_longer_than_any_backtick_run(self) -> None:
    self.assertEqual(rust_notices.fence_for("plain text"), "```")
    self.assertEqual(rust_notices.fence_for("```rust\ncode\n```"), "````")
    self.assertEqual(rust_notices.fence_for("`````"), "``````")

  def test_ci_false_values_are_not_ci(self) -> None:
    for value, expected in (("", False), ("0", False), ("false", False), ("FALSE", False), ("true", True), ("1", True)):
      with mock.patch.dict(os.environ, {"CI": value}):
        self.assertEqual(rust_notices.running_in_ci(), expected, value)

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
