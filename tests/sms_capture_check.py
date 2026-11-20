"""Check real frame-level SMS captures; does not establish VBlank timing."""

import argparse
from functools import lru_cache
import hashlib
import json
from pathlib import Path
import struct
import zlib

import sms_libretro_capture as capture
import sms_trace_tool as trace_tool


def quantize(rgb, pixel_format):
  if pixel_format == capture.PIXEL_XRGB8888:
    return rgb
  if pixel_format not in (capture.PIXEL_1555, capture.PIXEL_RGB565):
    raise ValueError("unsupported pixel format")
  result = bytearray(rgb)
  for index, value in enumerate(rgb):
    bits = 6 if pixel_format == capture.PIXEL_RGB565 and index % 3 == 1 else 5
    component = value >> (8 - bits)
    result[index] = (component << (8 - bits)) | (component >> (2 * bits - 8))
  return bytes(result)


def png_rgb(data):
  if data[:8] != b"\x89PNG\r\n\x1a\n":
    raise ValueError("not a PNG")
  pos, header, compressed, done = 8, None, bytearray(), False
  while pos < len(data):
    if pos + 12 > len(data):
      raise ValueError("truncated PNG chunk")
    length = int.from_bytes(data[pos:pos + 4], "big")
    kind = data[pos + 4:pos + 8]
    body = data[pos + 8:pos + 8 + length]
    crc = data[pos + 8 + length:pos + 12 + length]
    if len(body) != length or len(crc) != 4 or int.from_bytes(crc, "big") != zlib.crc32(kind + body) & 0xffffffff:
      raise ValueError("invalid PNG chunk")
    pos += 12 + length
    if kind == b"IHDR":
      header = struct.unpack(">IIBBBBB", body)
    elif kind == b"IDAT":
      compressed.extend(body)
    elif kind == b"IEND":
      done = True
      break
  if not done or pos != len(data) or header is None:
    raise ValueError("incomplete PNG")
  width, height, depth, colour, compression, filtering, interlace = header
  if (depth, colour, compression, filtering, interlace) != (8, 2, 0, 0, 0):
    raise ValueError("unsupported PNG")
  rows = zlib.decompress(compressed)
  stride = width * 3 + 1
  if len(rows) != stride * height or any(rows[y * stride] != 0 for y in range(height)):
    raise ValueError("invalid PNG rows")
  return width, height, b"".join(rows[y * stride + 1:(y + 1) * stride] for y in range(height))


def expected_palette(step):
  return 0 if step < 60 else 1 if step < 110 else 2 if step < 550 else 3 if step < 600 else 4


def check(candidate_dir, seed_dir, repeat_dir):
  result = json.loads((candidate_dir / "capture.json").read_text())
  repeat = json.loads((repeat_dir / "capture.json").read_text())
  seed = json.loads((seed_dir / "capture.json").read_text())
  if result != repeat:
    raise ValueError("fresh-run captures differ")
  if result["region"] not in (0, 1) or len(result["records"]) != 820:
    raise ValueError("wrong region or frame count")
  if result["pixel_format"] != capture.PIXEL_RGB565 or result["system_ram_size"] != 8192:
    raise ValueError("unexpected pixel format or RAM size")
  if seed["region"] != result["region"] or seed["core_sha256"] != result["core_sha256"]:
    raise ValueError("seed used a different core or region")
  records = result["records"]
  @lru_cache(maxsize=None)
  def expected_image(key):
    return quantize(trace_tool.expected_rgb(bytes(key)), result["pixel_format"])
  for step, record in enumerate(records):
    pad = capture.schedule(step)
    scene = 7 if 10 <= step < 750 else 0
    visible = int(40 <= step < 780)
    if (record["step"], record["pad"], record["buttons"], record["scene"], record["palette"],
        record["visible"]) != (step, pad, pad, scene, expected_palette(step), visible):
      raise ValueError(f"step {step}: control/scene mismatch")
    if (record["polls"], record["video_calls"], record["width"], record["height"]) != (1, 1, 256, 192):
      raise ValueError(f"step {step}: unexpected frontend callbacks or frame geometry")
    if visible and record["trace"][0] not in (0, 1, 2):
      raise ValueError(f"step {step}: flash mode")
    if step:
      prior = records[step - 1]
      key = tuple([prior["visible"]] + (prior["trace"] if prior["visible"] else [0] * 8))
      if record["rgb_sha256"] != hashlib.sha256(expected_image(key)).hexdigest():
        raise ValueError(f"step {step}: frame differs from previous observed trace")
  for start, end, layout in ((41, 270, 0), (271, 390, 1), (391, 510, 2), (511, 780, 3)):
    if records[start]["trace"][2] != layout or records[end - 1]["trace"][2] != layout:
      raise ValueError(f"layout {layout} missing at expected boundary")
  if {r["trace"][0] for r in records if r["visible"]} != {0, 1, 2}:
    raise ValueError("not all non-flash modes observed")
  if {r["trace"][1] for r in records if r["visible"]} != {0, 1}:
    raise ValueError("not both palette families observed")
  white = hashlib.sha256(bytes([255]) * (256 * 192 * 3)).hexdigest()
  if any(seed["records"][i]["rgb_sha256"] != white for i in range(len(seed["records"]))):
    raise ValueError("seed is not the expected static white frame")
  if records[0]["rgb_sha256"] != white or records[-1]["rgb_sha256"] != white:
    raise ValueError("candidate native differs from white seed")
  images = sorted(candidate_dir.glob("step-*.rgb"))
  if len(images) != 16:
    raise ValueError("expected 16 captured RGB/PNG image pairs")
  for image in images:
    step = int(image.stem.split("-")[1])
    rgb = image.read_bytes()
    if hashlib.sha256(rgb).hexdigest() != records[step]["rgb_sha256"]:
      raise ValueError(f"step {step}: RGB file hash mismatch")
    width, height, decoded = png_rgb(image.with_suffix(".png").read_bytes())
    if (width, height, decoded) != (256, 192, rgb):
      raise ValueError(f"step {step}: PNG does not match captured RGB")
  print(f"PASS: {result['fps']:.6f} Hz; 820 controls, 819 trace/frame hashes, 16 RGB/PNG pairs, seed, replay")
  print("PASS: all four layouts, both families, held/spectral/cuts; no flash mode")
  print(f"NOTE: exposed video RAM {result['video_ram_size']} bytes; VDP write timing not observed")


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--candidate", required=True, type=Path)
  parser.add_argument("--seed", required=True, type=Path)
  parser.add_argument("--repeat", required=True, type=Path)
  args = parser.parse_args()
  check(args.candidate, args.seed, args.repeat)


if __name__ == "__main__":
  main()
