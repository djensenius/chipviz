#!/usr/bin/env python3
"""Check Markdown relative links and trailing whitespace."""

from __future__ import annotations

import pathlib
import re
import sys
from urllib.parse import unquote, urlparse


ROOT = pathlib.Path(__file__).resolve().parents[2]
MARKDOWN_LINK = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")


def iter_markdown_files() -> list[pathlib.Path]:
  return sorted(
    path
    for path in ROOT.rglob("*.md")
    if ".git" not in path.parts and "target" not in path.parts
  )


def is_external_link(target: str) -> bool:
  parsed = urlparse(target)
  return bool(parsed.scheme or parsed.netloc or target.startswith("#") or target.startswith("mailto:"))


def check_file(path: pathlib.Path) -> list[str]:
  errors: list[str] = []
  text = path.read_text(encoding="utf-8")
  for line_number, line in enumerate(text.splitlines(), start=1):
    if line.rstrip() != line:
      errors.append(f"{path.relative_to(ROOT)}:{line_number}: trailing whitespace")
    for match in MARKDOWN_LINK.finditer(line):
      raw_target = match.group(1).split("#", 1)[0]
      if not raw_target or is_external_link(raw_target):
        continue
      target = (path.parent / unquote(raw_target)).resolve()
      try:
        target.relative_to(ROOT)
      except ValueError:
        errors.append(f"{path.relative_to(ROOT)}:{line_number}: link escapes repo: {match.group(1)}")
        continue
      if not target.exists():
        errors.append(f"{path.relative_to(ROOT)}:{line_number}: missing link target: {match.group(1)}")
  return errors


def main() -> int:
  errors: list[str] = []
  for path in iter_markdown_files():
    errors.extend(check_file(path))
  if errors:
    for error in errors:
      print(error, file=sys.stderr)
    return 1
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
