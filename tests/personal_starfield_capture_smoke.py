#!/usr/bin/env python3
"""Verify the bundle launcher with an already-built capture binary and fictional data.

This checks one fictional case; it is not an astronomy or safety certification.

Run only when GPU use is authorised:
  python3 tests/personal_starfield_capture_smoke.py --capture /path/to/chipviz-capture
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "shared" / "tools"))

import capture_assert


CATALOG = b"ra,dec,mag,ci,proper\n0,30,-2,0,Fictional\n"
EPOCH = "946728000"
FRAME = "frame000000-Starfield.png"

# At J2000 noon, Greenwich sidereal time is about 18h41m50.548s. For RA 0h,
# declination +30 degrees at the north pole and longitude 0, the altitude is
# 30 degrees and azimuth is (280.4606 - 180) = 100.4606 degrees, clockwise
# from north. In a 160x90 panorama with north at the horizontal seam and PNG
# rows from the top, x = 160*azimuth/360 = 44.65 and
# y = 90*(1 - altitude/90) = 60. The off-centre altitude also exposes a
# vertical flip. At the south pole the same star is 30 degrees below horizon.
#
# The bright fictional star is about 3.5 pixels wide. This 10x10 region allows
# subpixel/raster differences; tolerate two 8-bit RGB levels in the black
# baseline and require a 2x2 block four levels brighter, not exact colors.
STAR_X = range(40, 50)
STAR_Y = range(55, 65)
MAX_BASELINE_LEVEL = 2
MIN_STAR_DIFFERENCE = 4


def command(args: list[str]) -> subprocess.CompletedProcess:
  return subprocess.run(args, capture_output=True, text=True, timeout=120, check=False)


def require_success(result: subprocess.CompletedProcess, label: str) -> None:
  if result.returncode:
    raise RuntimeError(f"{label} exited {result.returncode}: {result.stderr.strip()}")


def capture(launcher: pathlib.Path, output: pathlib.Path, location: str) -> list[bytes]:
  result = command([
    str(launcher), "--profile", "m1", "--input", "procedural", "--frames", "0",
    "--size", "160x90", "--location", location, "--time", EPOCH, "--out", str(output),
  ])
  require_success(result, f"capture at {location}")

  manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
  expected = {
    "profile": "m1",
    "input": "procedural",
    "width": 160,
    "height": 90,
    "starfield_epoch_unix_seconds": int(EPOCH),
  }
  if (not isinstance(manifest, dict)
      or any(manifest.get(k) != v for k, v in expected.items())
      or manifest.get("starfield_epoch_advances_with_elapsed") is not False):
    raise RuntimeError(f"capture at {location} has unexpected manifest settings")
  captures = manifest.get("captures")
  expected_frame = {
    "file": FRAME,
    "frame_index": 0,
    "starfield_epoch_unix_seconds": int(EPOCH),
    "scene": 6,
    "displayed_scene": 6,
    "scene_name": "Starfield",
    "camera": "screen_space",
  }
  if (not isinstance(captures, list) or len(captures) != 1
      or not isinstance(captures[0], dict)
      or any(captures[0].get(k) != v for k, v in expected_frame.items())):
    raise RuntimeError(f"capture at {location} did not select the expected Starfield frame")

  image = capture_assert.decode_png((output / FRAME).read_bytes())
  if (image.width, image.height) != (160, 90):
    raise RuntimeError(f"capture at {location} has unexpected PNG dimensions")
  return image.rows


def verify(binary: pathlib.Path) -> None:
  binary = pathlib.Path(os.path.abspath(binary))
  with tempfile.TemporaryDirectory(prefix="chipviz-synthetic-") as directory:
    root = pathlib.Path(directory).resolve()
    checkout = root / "synthetic checkout with spaces"
    scripts = checkout / "scripts"
    scripts.mkdir(parents=True)
    (checkout / ".git").write_text("gitdir: synthetic\n", encoding="utf-8")
    script = scripts / "personal-starfield-bundle.py"
    shutil.copyfile(ROOT / "scripts" / script.name, script)

    catalog = root / "fictional catalog.csv"
    catalog.write_bytes(CATALOG)
    bundle = checkout / "build" / "synthetic bundle"
    bundle.mkdir(mode=0o700, parents=True)
    result = command([
      sys.executable, str(script), "--catalog", str(catalog),
      "--binary", str(binary), "--output", str(bundle),
    ])
    require_success(result, "synthetic bundle")

    launcher = bundle / "launch-starfield"
    missing_output = root / "missing location capture"
    missing = command([str(launcher), "--time", EPOCH, "--out", str(missing_output)])
    if (missing.returncode != 2
        or "--star-catalog and --location must be supplied together" not in missing.stderr
        or missing_output.exists()):
      raise RuntimeError("capture did not reject a missing explicit location before rendering")

    north = capture(launcher, root / "north capture", "90,0")
    south = capture(launcher, root / "south capture", "-90,0")
    if any(value > MAX_BASELINE_LEVEL for row in south for value in row):
      raise RuntimeError("south capture is not a dark below-horizon baseline")

    def star_pixel(x: int, y: int) -> bool:
      return any(north[y][3 * x + channel] - south[y][3 * x + channel] >= MIN_STAR_DIFFERENCE
                 for channel in range(3))

    if not any(all(star_pixel(x + dx, y + dy) for dy in (0, 1) for dx in (0, 1))
               for y in range(STAR_Y.start, STAR_Y.stop - 1)
               for x in range(STAR_X.start, STAR_X.stop - 1)):
      raise RuntimeError("fictional star missing from expected pixel region")


def main(argv: list[str] | None = None) -> int:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--capture", required=True, type=pathlib.Path,
                      help="already-built chipviz-capture executable")
  args = parser.parse_args(argv)
  try:
    verify(args.capture)
  except (capture_assert.CaptureError, json.JSONDecodeError, OSError,
          RuntimeError, subprocess.TimeoutExpired) as error:
    print(f"synthetic Starfield capture smoke failed: {error}", file=sys.stderr)
    return 1
  print("synthetic Starfield bundle capture smoke OK")
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
