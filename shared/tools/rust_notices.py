#!/usr/bin/env python3
"""Generate or check third-party notices for the shipped Rust renderer binaries.

Walks the dependency graph that Cargo resolved from renderers/modern/Cargo.lock
(normal dependencies only, for the release platforms) and collects each crate's
licence expression, the LICENSE/COPYING/NOTICE files shipped anywhere in its
package (including vendored subdirectories) and the <copyright> blocks of any
Wayland protocol XML files it embeds. Identical texts are printed once, with
the crates that use them.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import xml.etree.ElementTree as ElementTree
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "renderers" / "modern" / "Cargo.toml"
OUTPUT = ROOT / "THIRD_PARTY_NOTICES" / "rust.md"
# Release binaries are built on x86_64 Linux. The Raspberry Pi 5 and Apple
# silicon targets are extra coverage for local builds of the same binaries.
PLATFORMS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "aarch64-apple-darwin")
NOTICE_FILE = re.compile(r"^(licen[cs]e|copying|notice|unlicense|copyright)([-._].*)?$", re.IGNORECASE)
NOTICE_DIR = re.compile(r"^licen[cs]es$", re.IGNORECASE)
# Source files whose names happen to match NOTICE_FILE (e.g. src/copying.rs).
SOURCE_SUFFIXES = {".rs", ".c", ".h", ".py", ".js", ".sh", ".toml", ".xml", ".json"}
SPDX_ID = re.compile(r"SPDX-License-Identifier:\s*([A-Za-z0-9.+-]+)")
COPYLEFT = re.compile(r"^(A?GPL|LGPL|MPL|EPL|CDDL)-")
LGPL_DECISION = "https://github.com/djensenius/chipviz/issues/110"
# Canonical SPDX texts (license-list-data v3.29.0) for crates whose package
# ships no licence file.
FALLBACK_DIR = ROOT / "THIRD_PARTY_NOTICES" / "licenses"
# Licences whose canonical text needs no per-project copyright line.
NO_HOLDER_NEEDED = {"Apache-2.0", "CC0-1.0"}
MIT_TEMPLATE_LINE = "Copyright (c) <year> <copyright holders>"
# Copyright lines for MIT crates without a licence file, from the upstream
# LICENSE at the commit recorded in the crate's .cargo_vcs_info.json.
MIT_COPYRIGHT = {
  ("block-sys", "0.2.1"): ("Copyright (c) Steven Sheldon", "https://github.com/madsmtm/objc2/blob/01c7406f13e3f4abf75ec5e3d074893990c5eab5/LICENSE.txt"),
  ("block2", "0.3.0"): ("Copyright (c) Steven Sheldon", "https://github.com/madsmtm/objc2/blob/3d9b6864af05592fc18b1ab27b3da9ed3896bc53/LICENSE.txt"),
  ("icrate", "0.0.4"): ("Copyright (c) Steven Sheldon", "https://github.com/madsmtm/objc2/blob/3d9b6864af05592fc18b1ab27b3da9ed3896bc53/LICENSE.txt"),
  ("objc-sys", "0.3.5"): ("Copyright (c) Steven Sheldon", "https://github.com/madsmtm/objc2/blob/4fc083f1c6d6784577e38b0ee8dbd344481e2fd2/LICENSE.txt"),
  ("objc2", "0.4.1"): ("Copyright (c) Steven Sheldon", "https://github.com/madsmtm/objc2/blob/3d9b6864af05592fc18b1ab27b3da9ed3896bc53/LICENSE.txt"),
  ("objc2-encode", "3.0.0"): ("Copyright (c) Steven Sheldon", "https://github.com/madsmtm/objc2/blob/3d9b6864af05592fc18b1ab27b3da9ed3896bc53/LICENSE.txt"),
  ("wayland-protocols-plasma", "0.2.0"): ("Copyright (c) 2015 Elinor Berger", "https://github.com/smithay/wayland-rs/blob/ea4c856cca609bf658cc7945ffa7e4b4aa268171/LICENSE.txt"),
  ("wayland-protocols-wlr", "0.2.0"): ("Copyright (c) 2015 Elinor Berger", "https://github.com/smithay/wayland-rs/blob/8050a7bd668154f99ebfed805705cd10a1435b5a/LICENSE.txt"),
}
# Why the remaining MIT crates have no verified copyright line.
OBJC2_MIT_NOTE = "upstream LICENSE.md at the crate's commit states MIT but names no copyright holder"
KNOWN_GAPS = {
  ("block", "0.1.6"): "upstream SSheldon/rust-block has no licence file at tag 0.1.6",
  ("dispatch", "0.2.0"): "upstream SSheldon/rust-dispatch has no licence file at the crate's commit",
  ("malloc_buf", "0.0.6"): "upstream SSheldon/malloc_buf has no licence file at tag 0.0.6",
  ("block2", "0.6.2"): OBJC2_MIT_NOTE,
  ("objc2", "0.6.5"): OBJC2_MIT_NOTE,
  ("objc2-encode", "4.1.0"): OBJC2_MIT_NOTE,
  ("objc2-foundation", "0.3.2"): OBJC2_MIT_NOTE,
}


class SourcesUnavailable(Exception):
  """Crate sources are not available (offline without a populated registry)."""


def cargo_metadata() -> dict:
  command = ["cargo", "metadata", "--format-version", "1", "--locked", "--manifest-path", str(MANIFEST)]
  for platform in PLATFORMS:
    command += ["--filter-platform", platform]
  try:
    result = subprocess.run(command, check=False, capture_output=True, text=True)
  except FileNotFoundError as error:
    raise SourcesUnavailable("cargo is not installed") from error
  if result.returncode != 0:
    raise SourcesUnavailable(result.stderr.strip().splitlines()[0] if result.stderr.strip() else "cargo metadata failed")
  return json.loads(result.stdout)


def shipped_packages(metadata: dict) -> list[dict]:
  packages = {package["id"]: package for package in metadata["packages"]}
  nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
  members = set(metadata["workspace_members"])
  seen: set[str] = set()
  stack = list(members)
  while stack:
    package_id = stack.pop()
    if package_id in seen:
      continue
    seen.add(package_id)
    for dep in nodes[package_id]["deps"]:
      if any(kind["kind"] is None for kind in dep["dep_kinds"]):
        stack.append(dep["pkg"])
  shipped = [packages[package_id] for package_id in seen - members]
  return sorted(shipped, key=lambda package: (package["name"], package["version"]))


def notice_files(package: dict) -> list[Path]:
  directory = Path(package["manifest_path"]).parent
  if not directory.is_dir():
    raise SourcesUnavailable(f"missing sources for {package['name']} {package['version']}")
  files: set[Path] = set()
  for entry in directory.rglob("*"):
    if not entry.is_file() or "target" in entry.relative_to(directory).parts[:-1]:
      continue
    in_notice_dir = any(NOTICE_DIR.match(part) for part in entry.relative_to(directory).parts[:-1])
    if in_notice_dir or (NOTICE_FILE.match(entry.name) and entry.suffix.lower() not in SOURCE_SUFFIXES):
      files.add(entry)
  if package.get("license_file"):
    licence_file = directory / package["license_file"]
    if licence_file.is_file():
      files.add(licence_file)
  return sorted(files, key=lambda path: display_path(path, directory))


def protocol_copyrights(directory: Path) -> list[tuple[str, str]]:
  """Return (relative path, text) for each <copyright> block in the crate's XML files."""
  blocks = []
  for path in sorted(directory.rglob("*.xml"), key=lambda path: display_path(path, directory)):
    raw = path.read_text(encoding="utf-8", errors="replace")
    if "<copyright" not in raw:
      continue
    try:
      elements = ElementTree.fromstring(raw).iter("copyright")
      texts = ["".join(element.itertext()) for element in elements]
    except ElementTree.ParseError:
      texts = re.findall(r"<copyright>(.*?)</copyright>", raw, re.DOTALL)
    for text in texts:
      text = normalise(dedent(text))
      if text:
        blocks.append((display_path(path, directory), text))
  return blocks


def dedent(text: str) -> str:
  lines = text.replace("\r\n", "\n").split("\n")
  indents = [len(line) - len(line.lstrip()) for line in lines if line.strip()]
  margin = min(indents, default=0)
  return "\n".join(line[margin:] for line in lines)


def is_top_level(path: Path, directory: Path) -> bool:
  """True for the crate's own licence files: in its root or its root LICENSES/ directory."""
  return path.parent == directory or (path.parent.parent == directory and bool(NOTICE_DIR.match(path.parent.name)))


def display_path(path: Path, directory: Path) -> str:
  try:
    return path.relative_to(directory).as_posix()
  except ValueError:
    return path.name


def licence_alternatives(expression: str) -> list[list[str]]:
  """Split an SPDX expression (or legacy "A/B") into OR alternatives of AND-ed ids."""
  alternatives = []
  for alternative in re.split(r"\s+OR\s+|/", expression):
    ids = [token for token in re.split(r"\s+AND\s+|[()\s]+", alternative) if token and token != "WITH"]
    if ids:
      alternatives.append(ids)
  return alternatives


def fallback_texts(package: dict, fallback_dir: Path) -> tuple[list[tuple[str, str]], str | None]:
  """Return (label, text) fallbacks for a crate without licence files, and a gap reason if incomplete."""
  key = (package["name"], package["version"])
  expression = package.get("license") or ""
  alternatives = licence_alternatives(expression)
  found: list[tuple[str, str]] = []
  complete_found: list[tuple[str, str]] = []
  satisfied = False
  for alternative in alternatives:
    complete = True
    alternative_found: list[tuple[str, str]] = []
    for spdx_id in alternative:
      path = fallback_dir / f"{spdx_id}.txt"
      if not path.is_file():
        complete = False
        continue
      text = normalise(path.read_text(encoding="utf-8"))
      label = f"fallback SPDX {spdx_id} text"
      if spdx_id == "MIT" and key in MIT_COPYRIGHT:
        line, source = MIT_COPYRIGHT[key]
        text = text.replace(MIT_TEMPLATE_LINE, line)
        label += f", copyright line from {source}"
      elif spdx_id not in NO_HOLDER_NEEDED:
        complete = False
      if (label, text) not in alternative_found:
        alternative_found.append((label, text))
    found += [item for item in alternative_found if item not in found]
    if complete:
      satisfied = True
      complete_found += [item for item in alternative_found if item not in complete_found]
  if not alternatives:
    return found, "no licence declared"
  if satisfied:
    # Only link texts of complete alternatives, never an unfilled template.
    return complete_found, None
  return found, KNOWN_GAPS.get(key, "no licence file in package and no verified copyright holder for its licence")


def normalise(text: str) -> str:
  lines = [line.rstrip() for line in text.replace("\r\n", "\n").replace("\r", "\n").split("\n")]
  return "\n".join(lines).strip("\n")


def fence_for(text: str) -> str:
  longest = max((len(run) for run in re.findall(r"`{3,}", text)), default=2)
  return "`" * max(3, longest + 1)


def render(metadata: dict, fallback_dir: Path | None = None) -> str:
  fallback_dir = FALLBACK_DIR if fallback_dir is None else fallback_dir
  packages = shipped_packages(metadata)
  texts: dict[str, dict] = {}
  rows: list[tuple[str, str, str, list[str], list[str]]] = []
  gaps: list[tuple[str, str]] = []
  copyleft: list[tuple[str, list[str]]] = []

  def add(text: str, user: str) -> str:
    key = hashlib.sha256(" ".join(text.split()).encode("utf-8")).hexdigest()
    entry = texts.setdefault(key, {"text": text, "users": []})
    entry["users"].append(user)
    return key

  for package in packages:
    label = f"{package['name']} {package['version']}"
    directory = Path(package["manifest_path"]).parent
    refs: list[str] = []
    top_level = False
    embedded: set[str] = set()
    for path in notice_files(package):
      text = normalise(path.read_text(encoding="utf-8", errors="replace"))
      if text:
        refs.append(add(text, f"{label} ({display_path(path, directory)})"))
        if is_top_level(path, directory) or path == directory / (package.get("license_file") or ""):
          top_level = True
        else:
          embedded.update(SPDX_ID.findall(text))
          if any(NOTICE_DIR.match(part) for part in path.relative_to(directory).parts[:-1]):
            embedded.add(path.stem)
    protocols = protocol_copyrights(directory)
    if protocols:
      combined = "\n\n".join(f"== {path} ==\n{text}" for path, text in protocols)
      refs.append(add(combined, f"{label} (<copyright> blocks of {len(protocols)} protocol XML files)"))
      for _, text in protocols:
        embedded.update(SPDX_ID.findall(text))
    fallback: list[str] = []
    if not top_level:
      found, gap = fallback_texts(package, fallback_dir)
      fallback = [add(text, f"{label} ({fallback_label})") for fallback_label, text in found]
      if gap:
        gaps.append((label, gap))
    licence = package.get("license") or "not declared"
    declared = {token for alternative in licence_alternatives(licence) for token in alternative}
    extra = sorted(embedded - declared)
    notes = []
    if extra:
      notes.append("also contains " + ", ".join(extra))
    if protocols:
      notes.append(f"{len(protocols)} per-protocol notice{'' if len(protocols) == 1 else 's'}")
    if notes:
      licence += " (" + "; ".join(notes) + ")"
    copyleft_ids = [spdx_id for spdx_id in extra if COPYLEFT.match(spdx_id)]
    if copyleft_ids:
      copyleft.append((label, copyleft_ids))
    rows.append((label, licence, package.get("repository") or "", refs, fallback))

  ordered = sorted(texts.items(), key=lambda item: item[1]["users"][0])
  numbers = {key: index for index, (key, _) in enumerate(ordered, start=1)}

  lines = [
    "# Third-party notices: Rust renderer binaries",
    "",
    "<!-- Generated by `just third-party-notices` (shared/tools/rust_notices.py). Do not edit. -->",
    "",
    "The `chipviz-m1`, `chipviz-pi5` and `chipviz-capture` binaries statically",
    "include the Rust crates below, resolved from `renderers/modern/Cargo.lock`",
    f"for {', '.join(f'`{platform}`' for platform in PLATFORMS)}.",
    "Each crate is distributed under its own licence. Where a crate offers a",
    "choice of licences, every licence file shipped in its package is reproduced,",
    "including files in vendored subdirectories and the `<copyright>` blocks of",
    "embedded Wayland protocol XML files. Licences found only in those vendored",
    "files are listed as \"also contains\" in the table.",
    "Notice numbers marked \"(fallback)\" are canonical SPDX licence texts used",
    "because the published crate ships no licence file. For MIT crates, the",
    "copyright line comes from the upstream repository at the crate's commit.",
    "",
    "## Crates",
    "",
    "| Crate | Version | Licence | Repository | Notice texts |",
    "| --- | --- | --- | --- | --- |",
  ]
  for label, licence, repository, refs, fallback in rows:
    name, version = label.split(" ", 1)
    links = [f"[{numbers[key]}](#notice-{numbers[key]}) (fallback)" for key in sorted(set(fallback), key=numbers.get)]
    links += [f"[{numbers[key]}](#notice-{numbers[key]})" for key in sorted(set(refs) - set(fallback), key=numbers.get)]
    notices = ", ".join(links) or "none"
    lines.append(f"| {name} | {version} | {licence.replace('|', '/')} | {repository} | {notices} |")
  if copyleft:
    lines += ["", "## Copyleft components", ""]
    lines += [f"- {label}: {', '.join(ids)}." for label, ids in copyleft]
    lines += [
      "",
      "How the release binaries meet these licences' terms is tracked in",
      f"[issue #110]({LGPL_DECISION}).",
    ]
  lines += ["", "## Known gaps", ""]
  if gaps:
    lines += [
      "These crates ship no licence file, and no copyright holder could be",
      "verified upstream for the licence they declare. The canonical licence",
      "text is listed for each, with the copyright line left as the SPDX template.",
      "",
    ]
    lines += [f"- {label}: {reason}." for label, reason in gaps]
  else:
    lines.append("None.")
  lines += ["", "## Licence and notice texts", ""]
  for key, entry in ordered:
    number = numbers[key]
    fence = fence_for(entry["text"])
    lines += [
      f"### Notice {number}",
      "",
      f'<a id="notice-{number}"></a>',
      "",
      "Used by: " + "; ".join(entry["users"]) + ".",
      "",
      fence + "text",
      entry["text"],
      fence,
      "",
    ]
  return "\n".join(lines).rstrip("\n") + "\n"


def running_in_ci() -> bool:
  return os.environ.get("CI", "").strip().lower() not in ("", "0", "false")


def main(argv: list[str] | None = None) -> int:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--check", action="store_true", help="fail if the committed notices are stale")
  parser.add_argument("--output", type=Path, default=OUTPUT)
  args = parser.parse_args(argv)
  try:
    content = render(cargo_metadata())
  except SourcesUnavailable as error:
    if args.check and not running_in_ci():
      print(f"chipviz Rust notices check skipped: {error}")
      return 0
    reason = " (crate sources are required when CI is set)" if args.check else ""
    print(f"cannot generate Rust notices{reason}: {error}", file=sys.stderr)
    return 1
  if args.check:
    current = args.output.read_text(encoding="utf-8") if args.output.exists() else ""
    if current != content:
      print(f"{args.output} is stale; run `just third-party-notices`", file=sys.stderr)
      return 1
    print("chipviz Rust notices OK")
    return 0
  args.output.parent.mkdir(parents=True, exist_ok=True)
  args.output.write_text(content, encoding="utf-8")
  print(f"wrote {args.output}")
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
