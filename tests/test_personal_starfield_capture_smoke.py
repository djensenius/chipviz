import csv
import datetime
import importlib.util
import io
import math
import os
import pathlib
import shlex
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
SMOKE = ROOT / "tests" / "personal_starfield_capture_smoke.py"
SPEC = importlib.util.spec_from_file_location("personal_starfield_capture_smoke", SMOKE)
SMOKE_MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE_MODULE)

FAKE_CAPTURE = r"""
import json, os, pathlib, struct, sys, zlib

args = sys.argv[1:]
mode = os.environ.get("CHIPVIZ_FAKE_CAPTURE_MODE", "")
if args[:3] != ["--scene", "6", "--star-catalog"]:
    raise SystemExit("launcher did not pass the default scene and catalog")
if pathlib.Path(args[3]).read_bytes() != b"ra,dec,mag,ci,proper\n0,30,-2,0,Fictional\n":
    raise SystemExit("launcher did not pass the fictional catalog")

def value(option):
    return args[args.index(option) + 1]

if value("--time") != "946728000":
    raise SystemExit("wrong epoch")
if "--location" not in args:
    if mode == "accept-missing-location":
        raise SystemExit(0)
    print("--star-catalog and --location must be supplied together for scene 6", file=sys.stderr)
    raise SystemExit(2)
if (value("--profile"), value("--input"), value("--frames"), value("--size")) != (
        "m1", "procedural", "0", "160x90"):
    raise SystemExit("capture arguments changed")
location = value("--location")
if location not in ("90,0", "-90,0"):
    raise SystemExit("wrong location")

output = pathlib.Path(value("--out"))
output.mkdir()
epoch = 0 if mode == "wrong-epoch" else 946728000
frame = {
    "file": "frame000000-Starfield.png", "frame_index": 0,
    "starfield_epoch_unix_seconds": epoch, "scene": 6,
    "displayed_scene": 3 if mode == "wrong-scene" else 6,
    "scene_name": "Starfield", "camera": "screen_space",
}
manifest = {
    "profile": "m1", "input": "procedural", "width": 160, "height": 90,
    "starfield_epoch_unix_seconds": epoch,
    "starfield_epoch_advances_with_elapsed": mode == "advancing-epoch", "captures": [frame],
}
(output / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

baseline = 2 if mode in ("noise-tolerance", "below-threshold") else 0
rows = bytearray(b"".join(b"\0" + bytes([baseline]) * (160 * 3) for _ in range(90)))

def pixel(x, y, level):
    rows[y * (1 + 160 * 3) + 1 + x * 3] = level

def star(x, y, level):
    for dy in range(3):
        for dx in range(3):
            pixel(x + dx, y + dy, level)

if location == "90,0":
    if mode == "wrong-location":
        # A 90-degree longitude error shifts azimuth by 40 image pixels.
        star(83, 59, 40)
    elif mode == "wrong-y-orientation":
        star(43, 28, 40)
    elif mode == "unrelated-pixel":
        pixel(0, 0, 40)
    elif mode == "isolated-pixel":
        pixel(44, 60, 40)
    elif mode == "scattered-pixels":
        for x, y in ((40, 55), (49, 55), (40, 64), (49, 64)):
            pixel(x, y, 40)
    elif mode == "below-threshold":
        star(43, 59, baseline + 3)
    elif mode not in ("same-pixels", "missing-star"):
        star(43, 59, baseline + 4 if mode == "noise-tolerance" else 40)
elif mode == "south-star":
    star(43, 59, 40)
elif mode == "south-star-elsewhere":
    star(130, 12, 40)
elif mode == "south-over-tolerance":
    pixel(130, 12, 3)
header = struct.pack(">IIBBBBB", 160, 90, 8, 2, 0, 0, 0)
png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
       + chunk(b"IDAT", zlib.compress(bytes(rows))) + chunk(b"IEND", b""))
if mode == "invalid-png":
    png = b"not a PNG"
(output / frame["file"]).write_bytes(png)
"""


class PersonalStarfieldCaptureSmokeTests(unittest.TestCase):
  def setUp(self) -> None:
    self.temp = tempfile.TemporaryDirectory()
    self.addCleanup(self.temp.cleanup)
    self.binary = pathlib.Path(self.temp.name) / "fake capture binary"
    self.binary.write_text(
      "#!/bin/sh\n"
      f"exec {shlex.quote(sys.executable)} -c {shlex.quote(FAKE_CAPTURE)} \"$@\"\n",
      encoding="utf-8",
    )
    self.binary.chmod(0o755)

  def run_smoke(self, mode: str = "") -> subprocess.CompletedProcess:
    env = os.environ.copy()
    env["CHIPVIZ_FAKE_CAPTURE_MODE"] = mode
    return subprocess.run(
      [sys.executable, str(SMOKE), "--capture", str(self.binary)],
      capture_output=True, text=True, check=False, env=env,
    )

  def test_launcher_and_fictional_capture_contract(self) -> None:
    result = self.run_smoke()
    self.assertEqual(result.returncode, 0, result.stderr)
    self.assertIn("synthetic Starfield bundle capture smoke OK", result.stdout)

  def test_expected_region_from_independent_local_sky_basis(self) -> None:
    self.assertEqual(
      datetime.datetime.fromtimestamp(int(SMOKE_MODULE.EPOCH), datetime.timezone.utc),
      datetime.datetime(2000, 1, 1, 12, tzinfo=datetime.timezone.utc),
    )
    stars = list(csv.DictReader(io.StringIO(SMOKE_MODULE.CATALOG.decode("ascii"))))
    self.assertEqual(len(stars), 1)
    ra = math.radians(float(stars[0]["ra"]) * 15)
    dec = math.radians(float(stars[0]["dec"]))
    star = (math.cos(dec) * math.cos(ra), math.cos(dec) * math.sin(ra), math.sin(dec))

    # J2000 Greenwich mean sidereal time; dot the equatorial vector into
    # independently constructed east/north/up unit vectors at each pole.
    theta = math.radians(15 * (18 + 41 / 60 + 50.54841 / 3600))
    east = (-math.sin(theta), math.cos(theta), 0.0)

    def dot(left: tuple[float, ...], right: tuple[float, ...]) -> float:
      return sum(a * b for a, b in zip(left, right))

    def observe(latitude: float) -> tuple[float, float]:
      lat = math.radians(latitude)
      north = (-math.sin(lat) * math.cos(theta), -math.sin(lat) * math.sin(theta), math.cos(lat))
      up = (math.cos(lat) * math.cos(theta), math.cos(lat) * math.sin(theta), math.sin(lat))
      azimuth = math.degrees(math.atan2(dot(star, east), dot(star, north))) % 360
      altitude = math.degrees(math.asin(dot(star, up)))
      return azimuth, altitude

    azimuth, altitude = observe(90)
    self.assertAlmostEqual(azimuth, 100.460618, places=5)
    self.assertAlmostEqual(altitude, 30.0)
    self.assertAlmostEqual(observe(-90)[1], -30.0)
    x = 160 * azimuth / 360
    y = 90 * (1 - altitude / 90)
    self.assertAlmostEqual(x, 44.649164, places=5)
    self.assertAlmostEqual(y, 60.0)
    self.assertIn(math.floor(x), SMOKE_MODULE.STAR_X)
    self.assertIn(math.floor(y), SMOKE_MODULE.STAR_Y)
    self.assertNotIn(math.floor(90 - y), SMOKE_MODULE.STAR_Y)

  def test_smoke_tolerates_small_baseline_and_raster_quantization(self) -> None:
    result = self.run_smoke("noise-tolerance")
    self.assertEqual(result.returncode, 0, result.stderr)

  def test_smoke_rejects_incorrect_output_or_implicit_location(self) -> None:
    cases = (
      ("accept-missing-location", "did not reject a missing explicit location"),
      ("wrong-epoch", "unexpected manifest settings"),
      ("advancing-epoch", "unexpected manifest settings"),
      ("wrong-scene", "did not select the expected Starfield frame"),
      ("same-pixels", "missing from expected pixel region"),
      ("missing-star", "missing from expected pixel region"),
      ("wrong-location", "missing from expected pixel region"),
      ("wrong-y-orientation", "missing from expected pixel region"),
      ("unrelated-pixel", "missing from expected pixel region"),
      ("isolated-pixel", "missing from expected pixel region"),
      ("scattered-pixels", "missing from expected pixel region"),
      ("below-threshold", "missing from expected pixel region"),
      ("south-star", "not a dark below-horizon baseline"),
      ("south-star-elsewhere", "not a dark below-horizon baseline"),
      ("south-over-tolerance", "not a dark below-horizon baseline"),
      ("invalid-png", "not a PNG file"),
    )
    for mode, message in cases:
      with self.subTest(mode=mode):
        result = self.run_smoke(mode)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)


if __name__ == "__main__":
  unittest.main()
