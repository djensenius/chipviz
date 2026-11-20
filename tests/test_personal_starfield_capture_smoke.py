import os
import pathlib
import shlex
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
SMOKE = ROOT / "tests" / "personal_starfield_capture_smoke.py"
FAKE_CAPTURE = r"""
import json, os, pathlib, struct, sys, zlib

args = sys.argv[1:]
mode = os.environ.get("CHIPVIZ_FAKE_CAPTURE_MODE", "")
if args[:3] != ["--scene", "6", "--star-catalog"]:
    raise SystemExit("launcher did not pass the default scene and catalog")
if pathlib.Path(args[3]).read_bytes() != b"ra,dec,mag,ci,proper\n0,45,-2,0,Fictional\n":
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

rows = bytearray(b"".join(b"\0" + bytes(160 * 3) for _ in range(90)))
if location == "90,0" and mode != "same-pixels":
    rows[1] = 40
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

  def test_smoke_rejects_incorrect_output_or_implicit_location(self) -> None:
    cases = (
      ("accept-missing-location", "did not reject a missing explicit location"),
      ("wrong-epoch", "unexpected manifest settings"),
      ("advancing-epoch", "unexpected manifest settings"),
      ("wrong-scene", "did not select the expected Starfield frame"),
      ("same-pixels", "no brighter pixels above the horizon"),
      ("invalid-png", "not a PNG file"),
    )
    for mode, message in cases:
      with self.subTest(mode=mode):
        result = self.run_smoke(mode)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)


if __name__ == "__main__":
  unittest.main()
