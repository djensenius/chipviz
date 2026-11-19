import contextlib
import io
import json
import pathlib
import struct
import sys
import tempfile
import unittest
import zlib


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "shared" / "tools"))

import capture_assert


def _chunk(kind: bytes, body: bytes) -> bytes:
  return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)


def _filter_row(filter_type: int, line: bytes, previous: bytes, bpp: int) -> bytes:
  out = bytearray(len(line))
  for i, value in enumerate(line):
    left = line[i - bpp] if i >= bpp else 0
    up = previous[i]
    upper_left = previous[i - bpp] if i >= bpp else 0
    predictor = {
      0: 0,
      1: left,
      2: up,
      3: (left + up) >> 1,
      4: capture_assert._paeth(left, up, upper_left),
    }[filter_type]
    out[i] = (value - predictor) & 0xFF
  return bytes([filter_type]) + bytes(out)


def make_png(pixels, color_type=2, bit_depth=8, interlace=0, filters=(0,), raw_override=None) -> bytes:
  """Encode rows of (r, g, b[, a]) tuples, cycling PNG filter types per row."""
  height = len(pixels)
  width = len(pixels[0])
  bpp = 4 if color_type == 6 else 3
  raw = bytearray()
  previous = bytes(width * bpp)
  for y, row in enumerate(pixels):
    line = bytes(channel for pixel in row for channel in pixel[:bpp])
    raw.extend(_filter_row(filters[y % len(filters)], line, previous, bpp))
    previous = line
  if raw_override is not None:
    raw = raw_override
  header = struct.pack(">IIBBBBB", width, height, bit_depth, color_type, 0, 0, interlace)
  return (
    capture_assert.PNG_SIGNATURE
    + _chunk(b"IHDR", header)
    + _chunk(b"IDAT", zlib.compress(bytes(raw)))
    + _chunk(b"IEND", b"")
  )


def solid(width, height, color):
  return [[color] * width for _ in range(height)]


def gradient(width, height, offset=0):
  return [[((x * 37 + offset) % 256, (y * 53) % 256, (x * y + offset) % 256) for x in range(width)] for y in range(height)]


class CaptureAssertCase(unittest.TestCase):
  def setUp(self) -> None:
    self._tmp = tempfile.TemporaryDirectory()
    self.tmp = pathlib.Path(self._tmp.name)

  def tearDown(self) -> None:
    self._tmp.cleanup()

  def write(self, name: str, data: bytes) -> str:
    path = self.tmp / name
    path.write_bytes(data)
    return str(path)

  def invoke(self, *argv: str):
    stdout = io.StringIO()
    stderr = io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
      code = capture_assert.main(list(argv))
    report = json.loads(stdout.getvalue()) if stdout.getvalue() else None
    return code, report, stderr.getvalue()


class DecodeTests(CaptureAssertCase):
  def test_all_filter_types_round_trip_rgb_and_rgba(self) -> None:
    pixels = [[(x * 40 % 256, y * 70 % 256, (x + y) * 25 % 256, 200) for x in range(7)] for y in range(10)]
    expected = [bytes(c for p in row for c in p[:3]) for row in pixels]
    for color_type in (2, 6):
      image = capture_assert.decode_png(make_png(pixels, color_type=color_type, filters=(0, 1, 2, 3, 4)))
      self.assertEqual((image.width, image.height, image.color_type), (7, 10, color_type))
      self.assertEqual(image.rows, expected)

  def test_rejects_invalid_inputs_clearly(self) -> None:
    good = make_png(solid(4, 4, (1, 2, 3)))
    corrupt_crc = bytearray(good)
    corrupt_crc[30] ^= 0xFF  # inside IHDR body
    cases = {
      "bad signature": b"GIF89a" + good[6:],
      "CRC mismatch": bytes(corrupt_crc),
      "truncated": good[:-20],
      "only 8-bit RGB": make_png(solid(4, 4, (1, 2, 3)), bit_depth=16),
      "color_type=3": make_png(solid(4, 4, (1, 2, 3)), color_type=3),
      "color_type=0": make_png(solid(4, 4, (1, 2, 3)), color_type=0),
      "interlaced": make_png(solid(4, 4, (1, 2, 3)), interlace=1),
      "invalid PNG filter type": make_png(solid(2, 1, (1, 2, 3)), raw_override=b"\x07" + bytes(6)),
      "expected": make_png(solid(2, 2, (1, 2, 3)), raw_override=bytes(5)),
      "longer than": make_png(solid(2, 1, (1, 2, 3)), raw_override=bytes(40)),
      "missing IEND": good[:-12],
    }
    for message, data in cases.items():
      with self.subTest(message=message):
        with self.assertRaisesRegex(capture_assert.CaptureError, message):
          capture_assert.decode_png(data)

  def test_rejects_corrupt_zlib_stream(self) -> None:
    header = struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0)
    data = (capture_assert.PNG_SIGNATURE + _chunk(b"IHDR", header)
            + _chunk(b"IDAT", b"not zlib") + _chunk(b"IEND", b""))
    with self.assertRaisesRegex(capture_assert.CaptureError, "corrupt PNG image data"):
      capture_assert.decode_png(data)

  def test_cli_reports_invalid_input_with_exit_2(self) -> None:
    path = self.write("bad.png", b"\x89PNG\r\n\x1a\nxx")
    code, report, err = self.invoke(path, "--min-colors", "2")
    self.assertEqual(code, 2)
    self.assertIsNone(report)
    self.assertIn("bad.png", err)
    code, _, err = self.invoke(str(self.tmp / "missing.png"))
    self.assertEqual(code, 2)
    self.assertIn("missing.png", err)


class SingleImageTests(CaptureAssertCase):
  def test_uniform_image_fails_nonuniform_checks_and_reports_measurements(self) -> None:
    path = self.write("black.png", make_png(solid(16, 8, (0, 0, 0))))
    code, report, _ = self.invoke(path, "--min-colors", "2", "--min-luma-stddev", "1", "--min-lit-fraction", "0.01")
    self.assertEqual(code, 1)
    frame = report["frames"][0]
    self.assertEqual((frame["distinct_colors"], frame["luma_stddev"], frame["lit_fraction"]), (1, 0.0, 0.0))
    self.assertEqual([check["passed"] for check in report["assertions"]], [False, False, False])
    self.assertFalse(report["passed"])

  def test_uniform_white_is_measured_as_fully_lit_and_can_be_rejected(self) -> None:
    path = self.write("white.png", make_png(solid(8, 8, (255, 255, 255))))
    code, report, _ = self.invoke(path, "--min-lit-fraction", "0.5")
    self.assertEqual(code, 0)
    code, report, _ = self.invoke(path, "--max-lit-fraction", "0.99", "--min-colors", "2")
    self.assertEqual(code, 1)
    self.assertEqual(report["frames"][0]["lit_fraction"], 1.0)

  def test_spatial_variation_passes_requested_checks(self) -> None:
    path = self.write("grad.png", make_png(gradient(32, 16), filters=(4, 1)))
    code, report, _ = self.invoke(
      path, "--expect-size", "32x16", "--min-colors", "16", "--min-luma-stddev", "10",
      "--min-lit-fraction", "0.5", "--max-colors", "512",
    )
    self.assertEqual(code, 0, report)
    self.assertTrue(all(check["passed"] for check in report["assertions"]))
    self.assertEqual(len(report["assertions"]), 5)

  def test_no_requested_assertions_only_measures(self) -> None:
    path = self.write("black.png", make_png(solid(4, 4, (0, 0, 0))))
    code, report, _ = self.invoke(path, "--meta", "rom_sha256=abc", "--meta", "frame_label=f1200")
    self.assertEqual(code, 0)
    self.assertEqual(report["assertions"], [])
    self.assertEqual(report["provenance"], {"rom_sha256": "abc", "frame_label": "f1200"})

  def test_region_restricts_measurement(self) -> None:
    pixels = solid(20, 10, (0, 0, 0))
    for y in range(2, 4):
      for x in range(15, 18):
        pixels[y][x] = (200, 100, 50)
    path = self.write("sparse.png", make_png(pixels))
    code, report, _ = self.invoke(path, "--region", "0,0,10,10", "--min-colors", "2")
    self.assertEqual(code, 1)
    self.assertEqual(report["frames"][0]["region"], [0, 0, 10, 10])
    code, report, _ = self.invoke(path, "--region", "15,2,3,2", "--min-lit-fraction", "1.0", "--max-colors", "1")
    self.assertEqual(code, 0)
    self.assertEqual(report["frames"][0]["pixels"], 6)

  def test_region_outside_image_and_wrong_size(self) -> None:
    path = self.write("small.png", make_png(solid(4, 4, (9, 9, 9))))
    code, _, err = self.invoke(path, "--region", "2,2,4,4")
    self.assertEqual(code, 2)
    self.assertIn("exceeds image", err)
    code, report, _ = self.invoke(path, "--expect-size", "256x224")
    self.assertEqual(code, 1)
    self.assertEqual(report["assertions"][0]["measured"], "4x4")

  def test_alpha_is_ignored_for_measurement(self) -> None:
    pixels = [[(10, 20, 30, x * 60) for x in range(4)] for _ in range(2)]
    path = self.write("rgba.png", make_png(pixels, color_type=6))
    code, report, _ = self.invoke(path, "--max-colors", "1")
    self.assertEqual(code, 0)
    self.assertEqual(report["frames"][0]["color_type"], 6)


class SequenceTests(CaptureAssertCase):
  def test_identical_sequence(self) -> None:
    paths = [self.write(f"same{i}.png", make_png(gradient(12, 6), filters=(i % 5,))) for i in range(3)]
    code, report, _ = self.invoke(*paths, "--sequence", "identical")
    self.assertEqual(code, 0)
    self.assertEqual([step["changed_pixels"] for step in report["steps"]], [0, 0])
    code, report, _ = self.invoke(*paths, "--sequence", "changes")
    self.assertEqual(code, 1)
    self.assertEqual(report["assertions"][0]["measured"], 0)

  def test_changing_sequence(self) -> None:
    paths = [self.write(f"move{i}.png", make_png(gradient(12, 6, offset=i * 3))) for i in range(3)]
    code, report, _ = self.invoke(*paths, "--sequence", "every-step-changes")
    self.assertEqual(code, 0)
    self.assertTrue(all(step["changed"] for step in report["steps"]))
    code, _, _ = self.invoke(*paths, "--sequence", "identical")
    self.assertEqual(code, 1)

  def test_held_frames_satisfy_changes_but_not_every_step(self) -> None:
    first = self.write("a.png", make_png(gradient(8, 8)))
    held = self.write("b.png", make_png(gradient(8, 8)))
    moved = self.write("c.png", make_png(gradient(8, 8, offset=5)))
    code, _, _ = self.invoke(first, held, moved, "--sequence", "changes")
    self.assertEqual(code, 0)
    code, report, _ = self.invoke(first, held, moved, "--sequence", "every-step-changes")
    self.assertEqual(code, 1)
    self.assertEqual([step["changed"] for step in report["steps"]], [False, True])

  def test_tolerance_and_change_fraction(self) -> None:
    base = solid(10, 10, (100, 100, 100))
    noisy = solid(10, 10, (100, 100, 100))
    noisy[0][0] = (102, 100, 100)
    paths = [self.write("base.png", make_png(base)), self.write("noisy.png", make_png(noisy))]
    code, report, _ = self.invoke(*paths, "--sequence", "identical")
    self.assertEqual(code, 1)
    self.assertEqual(report["steps"][0]["max_channel_delta"], 2)
    code, _, _ = self.invoke(*paths, "--sequence", "identical", "--tolerance", "2")
    self.assertEqual(code, 0)
    code, _, _ = self.invoke(*paths, "--sequence", "identical", "--change-fraction", "0.01")
    self.assertEqual(code, 0)

  def test_region_limits_sequence_comparison(self) -> None:
    base = solid(10, 4, (0, 0, 0))
    changed = solid(10, 4, (0, 0, 0))
    changed[3][9] = (255, 255, 255)
    paths = [self.write("r0.png", make_png(base)), self.write("r1.png", make_png(changed))]
    code, _, _ = self.invoke(*paths, "--sequence", "identical", "--region", "0,0,5,4")
    self.assertEqual(code, 0)
    code, _, _ = self.invoke(*paths, "--sequence", "changes", "--region", "5,0,5,4")
    self.assertEqual(code, 0)

  def test_invalid_sequences(self) -> None:
    one = self.write("one.png", make_png(solid(4, 4, (0, 0, 0))))
    other = self.write("other.png", make_png(solid(5, 4, (0, 0, 0))))
    code, _, err = self.invoke(one, "--sequence", "changes")
    self.assertEqual(code, 2)
    self.assertIn("at least two", err)
    code, _, err = self.invoke(one, other, "--sequence", "changes")
    self.assertEqual(code, 2)
    self.assertIn("different dimensions", err)
    code, _, err = self.invoke(one, one, "--sequence", "identical", "--tolerance", "300")
    self.assertEqual(code, 2)


if __name__ == "__main__":
  unittest.main()
