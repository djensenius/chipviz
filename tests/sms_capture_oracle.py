"""Compare debugger-observed SMS polls with the independent shared oracle.

Frame-only libretro captures are deliberately not accepted as poll captures.
"""

import argparse
import json
from pathlib import Path
import subprocess

import sms_libretro_capture as capture


def schedule():
  # Exactly one controller value per Z80 poll, including boot warm-up.
  return bytes(120) + bytes(capture.schedule(step) for step in range(820))


def oracle(pads, executable):
  result = subprocess.run([str(executable), "--pad-input-stream"], input=pads,
                          capture_output=True, check=True)
  if len(result.stdout) != len(pads) * 27:
    raise ValueError("shared oracle returned the wrong number of poll records")
  return result.stdout


def compare(pads, expected, document):
  if document.get("kind") != "sms-z80-poll-v1" or document.get("start") != "reset":
    raise ValueError("requires a reset-to-finish Z80 breakpoint capture, not frame snapshots")
  records = document.get("records", [])
  if len(expected) != len(pads) * 27 or len(records) != len(pads):
    raise ValueError("missing, extra or incomplete poll records")
  previous_visible = 0
  for step, (pad, record) in enumerate(zip(pads, records)):
    row = expected[step * 27:(step + 1) * 27]
    scene, palette = row[:2]
    inputs, trace = list(row[2:18]), row[18:]
    visible = trace[0]
    entry = int(visible and not previous_visible)
    point = "field" if visible else "native"
    if (record.get("step"), record.get("point"), record.get("buttons"),
        record.get("scene"), record.get("palette"), record.get("visible"),
        record.get("entry"), record.get("input")) != (
            step, point, pad, scene, palette, visible, entry, inputs):
      raise ValueError(f"step {step}: controller, input or scene state differs from shared oracle")
    if visible:
      if record.get("peff") != trace[1] + 4 * trace[2] or record.get("trace") != list(trace[1:]):
        raise ValueError(f"step {step}: palette or field trace differs from shared oracle")
    previous_visible = visible


def compare_gx(pads, expected, document):
  """Compare completed snapshots taken at the *next* controller-port read."""
  if document.get("kind") != "sms-gx-port-v1" or document.get("start") != "reset":
    raise ValueError("requires reset-to-finish GX port-hook capture")
  events, records = document.get("events", []), document.get("records", [])
  if len(expected) != len(pads) * 27 or len(records) != len(pads) or len(events) != len(pads) + 1:
    raise ValueError("missing, extra or incomplete polls")
  previous_frame = -1
  for index, event in enumerate(events):
    wanted = pads[index] if index < len(pads) else 0
    if (event.get("poll") != index or event.get("pc") != 0x02e4
        or event.get("port", 0) & 255 != 0xdc or event.get("pad") != wanted
        or not isinstance(event.get("frame"), int) or event["frame"] <= previous_frame):
      raise ValueError(f"poll {index}: port, PC, pad or frame ordering differs")
    previous_frame = event["frame"]
  previous_visible = 0
  procedural_counter = 0
  pad_active = False
  for step, record in enumerate(records):
    row = expected[step * 27:(step + 1) * 27]
    trace = row[18:]
    visible = trace[0]
    pad_active |= bool(pads[step] & 0x30)
    if not pad_active:
      procedural_counter = (procedural_counter + 1) & 0xffff
    next_trace = expected[(step + 1) * 27 + 18:(step + 2) * 27] if step + 1 < len(pads) else None
    if (record.get("step"), record.get("buttons"), record.get("scene"),
        record.get("palette"), record.get("input"), record.get("visible"),
        record.get("entry"), record.get("procedural_counter"), record.get("pad_active")) != (
            step, pads[step], row[0], row[1], list(row[2:18]), visible,
            int(visible and not previous_visible), procedural_counter, int(pad_active)):
      raise ValueError(f"step {step}: controller, input or scene state differs from shared oracle")
    if next_trace is not None and record.get("shown") != next_trace[0]:
      raise ValueError(f"step {step}: next-step scene differs from shared oracle")
    if visible:
      if record.get("trace") != list(trace[1:]):
        raise ValueError(f"step {step}: field trace differs from shared oracle")
      if next_trace is None:
        raise ValueError("cannot validate post-finish palette at the end of a visible capture")
      # On exit, require a long stable palette with the same effective value;
      # no unresolved palette request is allowed at this particular boundary.
      if not next_trace[0]:
        p = row[1]
        effective = (p & 4) + min(p & 3, 2)
        if step < 60 or any(expected[i * 27 + 1] != p for i in range(step - 60, step + 1)):
          raise ValueError(f"step {step}: exit palette stability not established")
        if effective != trace[1] + 4 * trace[2]:
          raise ValueError(f"step {step}: exit palette is not the settled request")
        wanted_peff = effective
      else:
        wanted_peff = next_trace[1] + 4 * next_trace[2]
      if record.get("peff") != wanted_peff:
        raise ValueError(f"step {step}: post-finish palette differs from shared oracle")
    previous_visible = visible


def check_coverage(expected):
  if len(expected) % 27:
    raise ValueError("incomplete oracle records")
  rows = [expected[i:i + 27] for i in range(0, len(expected), 27)]
  traces = [row[18:] for row in rows if row[18]]
  if ({row[1] for row in traces} != {0, 1, 2}
      or {row[2] for row in traces} != {0, 1}
      or {row[3] for row in traces} != {0, 1, 2, 3}):
    raise ValueError("oracle schedule does not cover every safe mode, family and layout")
  if not any(row[1] == 3 and row[18:21] == bytes((1, 2, 0)) for row in rows):
    raise ValueError("oracle schedule does not exercise unarmed relative palette 3")
  visible = [row[18] for row in rows]
  if 0 not in visible or 1 not in visible or not any(a and not b for a, b in zip(visible, visible[1:])):
    raise ValueError("oracle schedule does not enter and exit the field")


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--oracle", required=True, type=Path)
  parser.add_argument("--capture", required=True, type=Path)
  args = parser.parse_args()
  pads = schedule()
  expected = oracle(pads, args.oracle.resolve())
  check_coverage(expected)
  document = json.loads(args.capture.read_text())
  if document.get("kind") == "sms-gx-port-v1":
    compare_gx(pads, expected, document)
  else:
    compare(pads, expected, document)
  print(f"PASS: {len(pads)} reset-to-finish Z80 polls match shared ColourField oracle")
  print("NOTE: capture provenance, VDP timing and hardware safety require separate verification")


if __name__ == "__main__":
  main()
