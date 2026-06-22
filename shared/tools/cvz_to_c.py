#!/usr/bin/env python3
"""Convert raw .cvz control-frame streams into C header arrays."""

from __future__ import annotations

import argparse
from pathlib import Path


WIRE_SIZE = 33


def sanitize_symbol(symbol: str) -> str:
  if not symbol or not symbol.replace("_", "").isalnum() or symbol[0].isdigit():
    raise ValueError("symbol must be a C identifier-like name")
  return symbol


def render_header(data: bytes, symbol: str) -> str:
  if len(data) % WIRE_SIZE != 0:
    raise ValueError("input is not aligned to 33-byte control frames")
  guard = f"CHIPVIZ_GENERATED_{symbol.upper()}_H"
  lines = [
    f"#ifndef {guard}",
    f"#define {guard}",
    "",
    "#include <stddef.h>",
    "#include <stdint.h>",
    "",
    f"static const size_t {symbol}_count = {len(data) // WIRE_SIZE}u;",
    f"static const uint8_t {symbol}[] = {{",
  ]
  for offset in range(0, len(data), 12):
    chunk = ", ".join(f"0x{byte:02X}" for byte in data[offset:offset + 12])
    lines.append(f"  {chunk},")
  lines.extend(["};", "", "#endif", ""])
  return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--input", type=Path, required=True)
  parser.add_argument("--symbol", required=True)
  parser.add_argument("--output", type=Path, required=True)
  return parser


def main_from_args(argv: list[str] | None = None) -> int:
  args = build_parser().parse_args(argv)
  symbol = sanitize_symbol(args.symbol)
  args.output.parent.mkdir(parents=True, exist_ok=True)
  args.output.write_text(render_header(args.input.read_bytes(), symbol), encoding="utf-8")
  return 0


def main() -> int:
  return main_from_args()


if __name__ == "__main__":
  raise SystemExit(main())
