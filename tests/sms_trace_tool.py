"""Export oracle-backed mailbox vectors and compare emulator captures.

The tool does not launch or implement an emulator. Its binary formats and
the required emulator-side call protocol are documented in cores/sms/README.md.
"""

import argparse
import importlib.util
import json
from pathlib import Path
import subprocess

import sms_detector_fixtures as fixtures


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("sms_assets", ROOT / "cores/sms/homebrew/field_assets.py")
assets = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(assets)
SCENARIOS = ("f18", "f19", "f20", "f21", "f22", "f24")
STEPS = 600


def inputs(name):
  return b"".join(bytes((scene, palette)) + payload
                  for scene, palette, payload in (getattr(fixtures, name)(step) for step in range(STEPS)))


def oracle(name, rows, executable):
  result = subprocess.run([str(executable), "--stream", name], input=rows, capture_output=True, check=True)
  if len(result.stdout) != STEPS * 9:
    raise ValueError("oracle did not return 600 trace records")
  return result.stdout


def pad_oracle(rows, executable):
  result = subprocess.run([str(executable), "--pad-stream"], input=rows, capture_output=True, check=True)
  if len(result.stdout) != STEPS * 9:
    raise ValueError("oracle did not return 600 pad trace records")
  return result.stdout


def compare_capture(name, expected, actual):
  if len(expected) != STEPS * 9 or len(actual) != STEPS * 16:
    raise ValueError("expected 600 oracle records and 600 mailbox records")
  for step in range(STEPS):
    want = expected[step * 9:step * 9 + 9]
    got = actual[step * 16:step * 16 + 16]
    if got[:9] != want:
      raise ValueError(f"{name} step {step}: trace {got[:9].hex()} != {want.hex()}")
    if name == "f18":
      # T, N, B, X, D, E, silent: only the new beat at step 300.
      event = bytes((0, 0, 1, 0, 0, 1, 0)) if step == 300 else bytes(7)
      if got[9:] != event:
        raise ValueError(f"f18 step {step}: events {got[9:].hex()} != {event.hex()}")


def expected_vdp(trace):
  if len(trace) != 9:
    raise ValueError("trace record must be 9 bytes")
  shown, mode, family, layout, combo, rotation, *levels = trace
  if not shown:
    if trace != bytes(9):
      raise ValueError("native trace must be all zero")
    return 0xf7, [0x3f, 0, 0]
  count = 3 if layout >= 2 else 2
  if shown != 1 or mode > 2 or family > 1 or layout > 3 or combo > 3 or rotation >= count:
    raise ValueError("invalid field trace")
  if any(level > 3 for level in levels) or (count == 2 and levels[2] != 0):
    raise ValueError("invalid field levels")
  colours = assets.COMBOS[family * 2 + count - 2][combo]
  cram = []
  for i in range(count):
    colour = assets.COLOURS[colours[(i + rotation) % count]]
    cram.append(sum(((((colour >> s) & 3) * levels[i] + 1) // 3) << s for s in (0, 2, 4)))
  return 0xf9 + 2 * layout, cram + [0] * (3 - count)


def expected_rgb(trace):
  _, cram = expected_vdp(trace)
  if not trace[0]:
    return bytes((255, 255, 255)) * (256 * 192)
  layout = trace[3]
  rgb = [bytes(((colour >> s) & 3) * 85 for s in (0, 2, 4)) for colour in cram]
  return b"".join(rgb[assets.region(layout, x // 8, y // 8)] for y in range(192) for x in range(256))


def compare_pixels(trace, actual):
  expected = expected_rgb(trace)
  if len(actual) != len(expected):
    raise ValueError("capture must be 256x192 raw RGB888")
  for offset in range(0, len(expected), 3):
    if actual[offset:offset + 3] != expected[offset:offset + 3]:
      pixel = offset // 3
      raise ValueError(f"pixel ({pixel % 256},{pixel // 256}): "
                       f"{actual[offset:offset + 3].hex()} != {expected[offset:offset + 3].hex()}")


def compare_vblank(expected, frames):
  if len(expected) != STEPS * 9 or len(frames) != STEPS:
    raise ValueError("expected 600 traces and 600 complete VDP frame records")
  previous_active = -1
  for step, frame in enumerate(frames):
    if frame["step"] != step or not previous_active < frame["start"] < frame["active"]:
      raise ValueError(f"step {step}: invalid frame bounds/order")
    previous_active = frame["active"]
    register, cram = expected_vdp(expected[step * 9:step * 9 + 9])
    want = [("cram", i + 1, value) for i, value in enumerate(cram)] + [("reg", 2, register)]
    writes = frame["writes"]
    if [(w["kind"], w["address"], w["value"]) for w in writes] != want:
      raise ValueError(f"step {step}: VDP write sequence differs")
    cycles = [w["cycle"] for w in writes]
    if cycles != sorted(set(cycles)) or any(not frame["start"] <= cycle < frame["active"] for cycle in cycles):
      raise ValueError(f"step {step}: write outside VBlank or nonmonotonic cycles")


def pad_schedule():
  """Active-high $DC bits, one poll per row; release and opposites included."""
  out = bytearray(STEPS)
  for step in (10, 11, 12):
    out[step] |= 1  # up, held across three polls
  for step in (60, 70, 80):
    out[step] |= 8  # right
  out[120] |= 12  # opposing palette directions cancel
  out[160] |= 4  # left
  out[200] |= 2  # down
  for step in range(100, 105):
    out[step] |= 16  # held beat, only first poll is an edge
  out[130] |= 16
  out[140] |= 32  # bar
  return bytes(out)


def render_vectors():
  """Presenter-only synthetic traces, not state-machine expectations."""
  out = bytearray()
  for family in range(2):
    for layout in range(4):
      count = 3 if layout >= 2 else 2
      for combo in range(4):
        for rotation in range(count):
          for level in range(4):
            levels = (level, (level + 1) % 4, (level + 2) % 4 if count == 3 else 0)
            out.extend((1, 1, family, layout, combo, rotation, *levels))
  out.extend(bytes((STEPS - len(out) // 9) * 9))
  return bytes(out)


def compare_pad(expected, records):
  if len(records) != STEPS or len(expected) != STEPS * 9:
    raise ValueError("expected 600 pad snapshots and oracle traces")
  schedule = pad_schedule()
  for step, record in enumerate(records):
    scene = 7 if 10 <= step < 200 else 0
    palette = 0 if step < 60 else 1 if step < 70 else 2 if step < 80 else 3 if step < 160 else 2
    trace = expected[step * 9:step * 9 + 9]
    visible = trace[0]
    if (record["step"], record["buttons"], record["scene_ring"], record["palette_ring"],
        record["visible"]) != (step, schedule[step], scene, palette, visible):
      raise ValueError(f"step {step}: pad/scene snapshot differs")
    if visible and record["peff"] != trace[1] + 4 * trace[2]:
      raise ValueError(f"step {step}: effective palette differs")
    if visible and record["trace"] != list(trace[1:]):
      raise ValueError(f"step {step}: pad field trace differs")


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  commands = parser.add_subparsers(dest="command", required=True)
  export = commands.add_parser("export")
  export.add_argument("--oracle", required=True, type=Path)
  export.add_argument("--output-dir", required=True, type=Path)
  capture = commands.add_parser("compare-trace")
  capture.add_argument("scenario", choices=SCENARIOS)
  capture.add_argument("--expected", required=True, type=Path)
  capture.add_argument("--capture", required=True, type=Path)
  pixels = commands.add_parser("compare-pixels")
  pixels.add_argument("--expected", required=True, type=Path)
  pixels.add_argument("--step", required=True, type=int)
  pixels.add_argument("--capture", required=True, type=Path)
  vblank = commands.add_parser("compare-vblank")
  vblank.add_argument("--expected", required=True, type=Path)
  vblank.add_argument("--capture", required=True, type=Path)
  pad = commands.add_parser("export-pad")
  pad.add_argument("--oracle", required=True, type=Path)
  pad.add_argument("--output", required=True, type=Path)
  pad.add_argument("--expected", required=True, type=Path)
  pad_check = commands.add_parser("compare-pad")
  pad_check.add_argument("--expected", required=True, type=Path)
  pad_check.add_argument("--capture", required=True, type=Path)
  render = commands.add_parser("export-render")
  render.add_argument("--output", required=True, type=Path)
  args = parser.parse_args()
  if args.command == "export":
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for name in SCENARIOS:
      rows = inputs(name)
      traces = oracle(name, rows, args.oracle.resolve())
      (args.output_dir / f"{name}.input.bin").write_bytes(rows)
      (args.output_dir / f"{name}.expected.bin").write_bytes(traces)
      print(f"{name}: 600 input and oracle trace records")
  elif args.command == "compare-trace":
    compare_capture(args.scenario, args.expected.read_bytes(), args.capture.read_bytes())
    print(f"PASS: {args.scenario} target mailbox trace and specified events")
  elif args.command == "compare-pixels":
    expected = args.expected.read_bytes()
    if len(expected) != STEPS * 9 or args.step not in range(STEPS):
      parser.error("expected must contain 600 records and step must be 0..599")
    compare_pixels(expected[args.step * 9:args.step * 9 + 9], args.capture.read_bytes())
    print(f"PASS: 256x192 active pixels at step {args.step}")
  elif args.command == "compare-vblank":
    compare_vblank(args.expected.read_bytes(), json.loads(args.capture.read_text()))
    print("PASS: 600 complete VBlank write sequences")
  elif args.command == "export-pad":
    rows = pad_schedule()
    traces = pad_oracle(rows, args.oracle.resolve())
    args.output.write_bytes(rows)
    args.expected.write_bytes(traces)
    print("600 active-high pad polls and oracle trace records")
  elif args.command == "compare-pad":
    compare_pad(args.expected.read_bytes(), json.loads(args.capture.read_text()))
    print("PASS: 600 pad, scene and effective-palette snapshots")
  else:
    args.output.write_bytes(render_vectors())
    print("600 presenter-only traces (320 field, 280 native)")


if __name__ == "__main__":
  main()
