#!/usr/bin/env python3
"""Measure local capture PNGs and check explicit, target-specific assertions.

This helper only inspects images that some other tool already captured. It
does not launch emulators or renderers. It prints measured values as JSON and
exits non-zero only when a requested assertion fails (1) or an input/usage is
invalid (2). No assertion is applied unless it is requested explicitly.
"""

from __future__ import annotations

import argparse
import collections
import fractions
import hashlib
import json
import math
import pathlib
import struct
import sys
import zlib
from dataclasses import dataclass


PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
SUPPORTED_COLOR_TYPES = {2: 3, 6: 4}  # RGB, RGBA -> bytes per pixel
MAX_PIXELS = 64 * 1024 * 1024
DEFAULT_LIT_THRESHOLD = 8
SEQUENCE_MODES = ("identical", "changes", "every-step-changes")


class CaptureError(ValueError):
  """Raised for unreadable, corrupt or unsupported inputs."""


@dataclass(frozen=True)
class Image:
  width: int
  height: int
  color_type: int
  rows: list[bytes]  # RGB only, 3 bytes per pixel; alpha is discarded


@dataclass(frozen=True)
class Region:
  x: int
  y: int
  width: int
  height: int


def _paeth(a: int, b: int, c: int) -> int:
  p = a + b - c
  pa = abs(p - a)
  pb = abs(p - b)
  pc = abs(p - c)
  if pa <= pb and pa <= pc:
    return a
  if pb <= pc:
    return b
  return c


def _unfilter(raw: bytes, width: int, height: int, bpp: int) -> list[bytes]:
  stride = width * bpp
  expected = height * (stride + 1)
  if len(raw) != expected:
    raise CaptureError(f"decompressed image data is {len(raw)} bytes, expected {expected}")
  rows: list[bytes] = []
  previous = bytearray(stride)
  for y in range(height):
    offset = y * (stride + 1)
    filter_type = raw[offset]
    line = bytearray(raw[offset + 1:offset + 1 + stride])
    if filter_type == 0:
      pass
    elif filter_type == 1:
      for i in range(bpp, stride):
        line[i] = (line[i] + line[i - bpp]) & 0xFF
    elif filter_type == 2:
      for i in range(stride):
        line[i] = (line[i] + previous[i]) & 0xFF
    elif filter_type == 3:
      for i in range(stride):
        left = line[i - bpp] if i >= bpp else 0
        line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
    elif filter_type == 4:
      for i in range(stride):
        if i >= bpp:
          line[i] = (line[i] + _paeth(line[i - bpp], previous[i], previous[i - bpp])) & 0xFF
        else:
          line[i] = (line[i] + previous[i]) & 0xFF
    else:
      raise CaptureError(f"invalid PNG filter type {filter_type} on row {y}")
    rows.append(bytes(line))
    previous = line
  return rows


def decode_png(data: bytes) -> Image:
  """Decode a non-interlaced 8-bit RGB/RGBA PNG to RGB rows."""
  if not data.startswith(PNG_SIGNATURE):
    raise CaptureError("not a PNG file (bad signature)")
  pos = len(PNG_SIGNATURE)
  header: tuple[int, ...] | None = None
  idat = bytearray()
  idat_open = False
  idat_done = False
  saw_end = False
  while pos < len(data):
    if pos + 8 > len(data):
      raise CaptureError("truncated PNG chunk header")
    length, kind = struct.unpack(">I4s", data[pos:pos + 8])
    end = pos + 8 + length + 4
    if end > len(data):
      raise CaptureError(f"truncated PNG chunk {kind!r}")
    body = data[pos + 8:pos + 8 + length]
    (crc,) = struct.unpack(">I", data[pos + 8 + length:end])
    if zlib.crc32(kind + body) & 0xFFFFFFFF != crc:
      raise CaptureError(f"CRC mismatch in PNG chunk {kind!r}")
    if header is None and kind != b"IHDR":
      raise CaptureError("first PNG chunk is not IHDR")
    if kind == b"IHDR":
      if header is not None or length != 13:
        raise CaptureError("invalid IHDR chunk")
      header = struct.unpack(">IIBBBBB", body)
    elif kind == b"IDAT":
      if idat_done:
        raise CaptureError("non-consecutive IDAT chunks")
      idat.extend(body)
      idat_open = True
    elif kind == b"IEND":
      saw_end = True
      break  # bytes after IEND are ignored
    elif kind == b"PLTE":
      if idat_open:
        raise CaptureError("PLTE chunk after IDAT")
    elif not kind[0] & 0x20:
      raise CaptureError(f"unknown critical PNG chunk {kind!r}")
    if kind != b"IDAT" and idat_open:
      idat_done = True
    pos = end
  if header is None:
    raise CaptureError("missing IHDR chunk")
  if not saw_end:
    raise CaptureError("missing IEND chunk")
  width, height, bit_depth, color_type, compression, filter_method, interlace = header
  if width == 0 or height == 0 or width * height > MAX_PIXELS:
    raise CaptureError(f"unsupported image dimensions {width}x{height}")
  if bit_depth != 8 or color_type not in SUPPORTED_COLOR_TYPES:
    raise CaptureError(
      f"unsupported PNG format bit_depth={bit_depth} color_type={color_type}; "
      "only 8-bit RGB (2) and RGBA (6) are supported"
    )
  if compression != 0 or filter_method != 0:
    raise CaptureError("unsupported PNG compression or filter method")
  if interlace != 0:
    raise CaptureError("interlaced PNGs are not supported")
  if not idat:
    raise CaptureError("missing IDAT data")
  bpp = SUPPORTED_COLOR_TYPES[color_type]
  expected = height * (width * bpp + 1)
  decompressor = zlib.decompressobj()
  try:
    raw = decompressor.decompress(bytes(idat), expected + 1)
  except zlib.error as error:
    raise CaptureError(f"corrupt PNG image data: {error}") from error
  if not decompressor.eof:
    raise CaptureError("PNG image data is truncated or longer than its dimensions")
  if decompressor.unused_data:
    raise CaptureError("unexpected bytes after the end of the PNG zlib stream")
  rows = _unfilter(raw, width, height, bpp)
  if bpp == 4:
    rgb_rows = []
    for row in rows:
      rgb = bytearray(width * 3)
      rgb[0::3] = row[0::4]
      rgb[1::3] = row[1::4]
      rgb[2::3] = row[2::4]
      rgb_rows.append(bytes(rgb))
    rows = rgb_rows
  return Image(width, height, color_type, rows)


def parse_region(text: str) -> Region:
  try:
    x, y, width, height = (int(part) for part in text.split(","))
  except ValueError as error:
    raise argparse.ArgumentTypeError("region must be X,Y,W,H integers") from error
  if x < 0 or y < 0 or width <= 0 or height <= 0:
    raise argparse.ArgumentTypeError("region needs non-negative origin and positive size")
  return Region(x, y, width, height)


def parse_size(text: str) -> tuple[int, int]:
  try:
    width, height = (int(part) for part in text.lower().split("x"))
  except ValueError as error:
    raise argparse.ArgumentTypeError("size must be WxH") from error
  if width <= 0 or height <= 0:
    raise argparse.ArgumentTypeError("size must have positive width and height")
  return width, height


def parse_meta(text: str) -> tuple[str, str]:
  key, sep, value = text.partition("=")
  if not sep or not key:
    raise argparse.ArgumentTypeError("metadata must be KEY=VALUE")
  return key, value


def region_rows(image: Image, region: Region | None) -> tuple[Region, list[bytes]]:
  if region is None:
    region = Region(0, 0, image.width, image.height)
  if region.x + region.width > image.width or region.y + region.height > image.height:
    raise CaptureError(
      f"region {region.x},{region.y},{region.width},{region.height} "
      f"exceeds image {image.width}x{image.height}"
    )
  start = region.x * 3
  stop = start + region.width * 3
  return region, [row[start:stop] for row in image.rows[region.y:region.y + region.height]]


def measure(rows: list[bytes], lit_threshold: int) -> dict[str, object]:
  """Return exact counts and unrounded statistics; round only for display."""
  histogram: collections.Counter[tuple[int, int, int]] = collections.Counter()
  for row in rows:
    histogram.update(zip(row[0::3], row[1::3], row[2::3]))
  total = sum(histogram.values())
  luma_sum = 0
  luma_sq = 0
  lit = 0
  for (r, g, b), count in histogram.items():
    luma = (r * 299 + g * 587 + b * 114) // 1000
    luma_sum += luma * count
    luma_sq += luma * luma * count
    if luma > lit_threshold:
      lit += count
  mean = luma_sum / total
  variance = (luma_sq * total - luma_sum * luma_sum) / (total * total)
  return {
    "pixels": total,
    "distinct_colors": len(histogram),
    "luma_mean": mean,
    "luma_stddev": math.sqrt(variance),
    "lit_pixels": lit,
    "lit_fraction": fractions.Fraction(lit, total),
  }


def compare(first: list[bytes], second: list[bytes], tolerance: int) -> dict[str, object]:
  """Return exact changed-pixel counts; changed_fraction is an exact Fraction."""
  changed = 0
  max_delta = 0
  total = 0
  for row_a, row_b in zip(first, second):
    total += len(row_a) // 3
    if row_a == row_b:
      continue
    for i in range(0, len(row_a), 3):
      delta = max(abs(row_a[i] - row_b[i]), abs(row_a[i + 1] - row_b[i + 1]), abs(row_a[i + 2] - row_b[i + 2]))
      if delta > max_delta:
        max_delta = delta
      if delta > tolerance:
        changed += 1
  return {
    "compared_pixels": total,
    "changed_pixels": changed,
    "changed_fraction": fractions.Fraction(changed, total),
    "max_channel_delta": max_delta,
  }


def _display(stats: dict[str, object]) -> dict[str, object]:
  shown = dict(stats)
  for key, digits in (("luma_mean", 4), ("luma_stddev", 4), ("lit_fraction", 6), ("changed_fraction", 6)):
    if key in shown:
      shown[key] = round(float(shown[key]), digits)
  return shown


def parse_fraction(option: str, text: str) -> fractions.Fraction:
  """Parse decimal or ratio text ("0.3", "1/3") exactly, without a float round trip."""
  try:
    return fractions.Fraction(text.strip())
  except (ValueError, ZeroDivisionError) as error:
    raise CaptureError(f"{option} must be a finite decimal or ratio such as 0.3 or 1/3") from error


def validate(args: argparse.Namespace) -> None:
  if args.sequence and len(args.images) < 2:
    raise CaptureError("--sequence needs at least two images")
  if not 0 <= args.tolerance <= 255:
    raise CaptureError("--tolerance must be between 0 and 255")
  if not 0 <= args.lit_threshold <= 255:
    raise CaptureError("--lit-threshold must be between 0 and 255")
  exact: dict[str, fractions.Fraction | None] = {}
  for option in ("change_fraction", "min_lit_fraction", "max_lit_fraction"):
    text = getattr(args, option)
    flag = f"--{option.replace('_', '-')}"
    exact[option] = None if text is None else parse_fraction(flag, text)
  if not 0 <= exact["change_fraction"] < 1:
    raise CaptureError("--change-fraction must be in [0, 1)")
  for option in ("min_lit_fraction", "max_lit_fraction"):
    if exact[option] is not None and not 0 <= exact[option] <= 1:
      raise CaptureError(f"--{option.replace('_', '-')} must be in [0, 1]")
  if exact["min_lit_fraction"] is not None and exact["max_lit_fraction"] is not None \
      and exact["min_lit_fraction"] > exact["max_lit_fraction"]:
    raise CaptureError("--min-lit-fraction must not exceed --max-lit-fraction")
  for option in ("min_colors", "max_colors"):
    value = getattr(args, option)
    if value is not None and value < 0:
      raise CaptureError(f"--{option.replace('_', '-')} must be non-negative")
  if args.min_colors is not None and args.max_colors is not None and args.min_colors > args.max_colors:
    raise CaptureError("--min-colors must not exceed --max-colors")
  if args.min_luma_stddev is not None and not (math.isfinite(args.min_luma_stddev) and args.min_luma_stddev >= 0):
    raise CaptureError("--min-luma-stddev must be finite and non-negative")
  args.exact = exact


def _fraction_text(stats: dict[str, object]) -> str:
  return f"{stats['lit_pixels']}/{stats['pixels']} ({float(stats['lit_fraction']):.6g})"


def _check(results: list[dict[str, object]], name: str, target: str, expected: str, measured: object, passed: bool) -> None:
  results.append({"assertion": name, "target": target, "expected": expected, "measured": measured, "passed": passed})


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
  parser.add_argument("images", nargs="+", type=pathlib.Path, help="PNG captures, in sequence order")
  parser.add_argument("--region", type=parse_region, help="X,Y,W,H region measured in every image")
  parser.add_argument("--lit-threshold", type=int, default=DEFAULT_LIT_THRESHOLD,
                      help="a pixel is lit when integer Rec.601 luma exceeds this (default 8)")
  parser.add_argument("--expect-size", type=parse_size, help="assert every image is exactly WxH")
  parser.add_argument("--min-colors", type=int, help="assert at least N distinct RGB colours in the region")
  parser.add_argument("--max-colors", type=int, help="assert at most N distinct RGB colours in the region")
  parser.add_argument("--min-luma-stddev", type=float, help="assert luma standard deviation >= F")
  parser.add_argument("--min-lit-fraction", help="assert lit pixel fraction >= F (0..1; decimal or ratio, exact)")
  parser.add_argument("--max-lit-fraction", help="assert lit pixel fraction <= F (0..1; decimal or ratio, exact)")
  parser.add_argument("--sequence", choices=SEQUENCE_MODES,
                      help="assert consecutive images are identical, change at least once, or change every step")
  parser.add_argument("--tolerance", type=int, default=0,
                      help="per-channel delta treated as unchanged in sequence comparisons (default 0)")
  parser.add_argument("--change-fraction", default="0",
                      help="a step counts as changed when changed_fraction exceeds this "
                           "(0 <= F < 1; decimal or ratio, exact; default 0)")
  parser.add_argument("--meta", type=parse_meta, action="append", default=[],
                      help="KEY=VALUE provenance echoed into the report (repeatable)")
  return parser


def run(args: argparse.Namespace) -> dict[str, object]:
  validate(args)
  checks: list[dict[str, object]] = []
  frames: list[dict[str, object]] = []
  regions: list[list[bytes]] = []
  for path in args.images:
    try:
      data = path.read_bytes()
    except OSError as error:
      raise CaptureError(f"{path}: {error.strerror or error}") from error
    try:
      image = decode_png(data)
      region, rows = region_rows(image, args.region)
    except CaptureError as error:
      raise CaptureError(f"{path}: {error}") from error
    stats = measure(rows, args.lit_threshold)
    name = str(path)
    frames.append({
      "path": name,
      "sha256": hashlib.sha256(data).hexdigest(),
      "width": image.width,
      "height": image.height,
      "color_type": image.color_type,
      "region": [region.x, region.y, region.width, region.height],
      **_display(stats),
    })
    regions.append(rows)
    if args.expect_size is not None:
      expected = f"{args.expect_size[0]}x{args.expect_size[1]}"
      _check(checks, "expect-size", name, expected, f"{image.width}x{image.height}",
             (image.width, image.height) == args.expect_size)
    if args.min_colors is not None:
      _check(checks, "min-colors", name, f">= {args.min_colors}", stats["distinct_colors"],
             stats["distinct_colors"] >= args.min_colors)
    if args.max_colors is not None:
      _check(checks, "max-colors", name, f"<= {args.max_colors}", stats["distinct_colors"],
             stats["distinct_colors"] <= args.max_colors)
    if args.min_luma_stddev is not None:
      _check(checks, "min-luma-stddev", name, f">= {args.min_luma_stddev}", round(stats["luma_stddev"], 4),
             stats["luma_stddev"] >= args.min_luma_stddev)
    if args.min_lit_fraction is not None:
      _check(checks, "min-lit-fraction", name, f">= {args.min_lit_fraction}", _fraction_text(stats),
             stats["lit_fraction"] >= args.exact["min_lit_fraction"])
    if args.max_lit_fraction is not None:
      _check(checks, "max-lit-fraction", name, f"<= {args.max_lit_fraction}", _fraction_text(stats),
             stats["lit_fraction"] <= args.exact["max_lit_fraction"])

  steps: list[dict[str, object]] = []
  comparable: bool | None = None
  if len(regions) > 1:
    shape = (len(regions[0]), len(regions[0][0]))
    comparable = all((len(rows), len(rows[0])) == shape for rows in regions)
    if comparable:
      for index in range(1, len(regions)):
        step = compare(regions[index - 1], regions[index], args.tolerance)
        changed = step["changed_fraction"] > args.exact["change_fraction"]
        steps.append({"from": frames[index - 1]["path"], "to": frames[index]["path"], **_display(step), "changed": changed})
    elif args.sequence:
      raise CaptureError("sequence images/regions have different dimensions")
  if args.sequence:
    changed_steps = sum(1 for step in steps if step["changed"])
    expected = {
      "identical": "0 changed steps",
      "changes": ">= 1 changed step",
      "every-step-changes": f"{len(steps)} changed steps",
    }[args.sequence]
    passed = {
      "identical": changed_steps == 0,
      "changes": changed_steps >= 1,
      "every-step-changes": changed_steps == len(steps),
    }[args.sequence]
    _check(checks, f"sequence-{args.sequence}", "sequence", expected, changed_steps, passed)

  return {
    "tool": "capture_assert",
    "provenance": dict(args.meta),
    "parameters": {
      "region": None if args.region is None else [args.region.x, args.region.y, args.region.width, args.region.height],
      "lit_threshold": args.lit_threshold,
      "tolerance": args.tolerance,
      "change_fraction": str(args.exact["change_fraction"]),
    },
    "frames": frames,
    "steps_comparable": comparable,
    "steps": steps,
    "assertions": checks,
    "passed": all(check["passed"] for check in checks),
  }


def main(argv: list[str] | None = None) -> int:
  args = build_parser().parse_args(argv)
  try:
    report = run(args)
  except CaptureError as error:
    print(f"capture_assert: error: {error}", file=sys.stderr)
    return 2
  print(json.dumps(report, indent=2))
  return 0 if report["passed"] else 1


if __name__ == "__main__":
  raise SystemExit(main())
