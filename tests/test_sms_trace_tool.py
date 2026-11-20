"""Host-only validation of fixture/oracle handoff and capture comparators."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import sms_trace_tool as tool
import sms_capture_oracle as capture_oracle


ROOT = Path(__file__).resolve().parents[1]


class SmsTraceToolTests(unittest.TestCase):
  @classmethod
  def setUpClass(cls):
    cls.temp = tempfile.TemporaryDirectory(prefix="sms-oracle-test-")
    cls.oracle = Path(cls.temp.name) / "oracle"
    subprocess.run(["rustc", "--edition=2024", str(ROOT / "tests/sms_shared_oracle.rs"),
                    "-o", str(cls.oracle)], check=True)
    cls.pad_reference = Path(cls.temp.name) / "pad-reference"
    cls.procedural_reference = Path(cls.temp.name) / "procedural-reference"
    for source, output in (("sms_pad_reference.c", cls.pad_reference),
                           ("sms_procedural_oracle.c", cls.procedural_reference)):
      subprocess.run(["cc", "-std=c99", "-Wall", "-Wextra", "-Werror",
                      "-I", str(ROOT / "shared/include"), str(ROOT / "tests" / source),
                      str(ROOT / "shared/src/control_frame.c"), "-o", str(output)], check=True)

  @classmethod
  def tearDownClass(cls):
    cls.temp.cleanup()

  def test_all_fixture_rows_match_independent_reference_setup(self):
    for name in tool.SCENARIOS:
      trace = tool.oracle(name, tool.inputs(name), self.oracle)
      self.assertEqual(len(trace), 600 * 9)

  def test_f18_held_palette_is_rejected_by_reference(self):
    rows = bytearray(tool.inputs("f18"))
    rows[1::18] = bytes(600)
    result = subprocess.run([str(self.oracle), "--stream", "f18"], input=rows, capture_output=True)
    self.assertNotEqual(result.returncode, 0)
    self.assertIn(b"f18 input step 0", result.stderr)

  def test_f18_trace_and_event_comparator_rejects_bad_capture(self):
    expected = tool.oracle("f18", tool.inputs("f18"), self.oracle)
    self.assertTrue(all(expected[i * 9 + 1] == 2 for i in range(600)))
    self.assertEqual([expected[step * 9 + 5] for step in (0, 9, 299, 300, 599)], [0, 0, 0, 1, 1])
    actual = bytearray()
    for step in range(600):
      actual.extend(expected[step * 9:step * 9 + 9])
      actual.extend(bytes((0, 0, 1, 0, 0, 1, 0)) if step == 300 else bytes(7))
    tool.compare_capture("f18", expected, actual)
    actual[1] = 0
    with self.assertRaisesRegex(ValueError, "step 0: trace"):
      tool.compare_capture("f18", expected, actual)
    actual[1] = 2
    actual[16 + 10] = 1
    with self.assertRaisesRegex(ValueError, "step 1: events"):
      tool.compare_capture("f18", expected, actual)

  def test_pixel_comparator_checks_every_pixel(self):
    for layout in range(4):
      trace = bytes((1, 2, 1, layout, 1, 1, 3, 2, 1 if layout >= 2 else 0))
      image = tool.expected_rgb(trace)
      tool.compare_pixels(trace, image)
      broken = bytearray(image)
      broken[-1] ^= 1
      with self.assertRaisesRegex(ValueError, r"pixel \(255,191\)"):
        tool.compare_pixels(trace, broken)
    self.assertEqual(tool.expected_rgb(bytes(9)), bytes((255, 255, 255)) * 256 * 192)

  def test_render_vectors_cover_all_layouts_and_families(self):
    data = tool.render_vectors()
    self.assertEqual(len(data), 600 * 9)
    rows = [data[i:i + 9] for i in range(0, len(data), 9)]
    field = [row for row in rows if row[0]]
    self.assertEqual(len(field), 320)
    self.assertEqual({(row[2], row[3], row[4]) for row in field},
                     {(family, layout, combo) for family in range(2)
                      for layout in range(4) for combo in range(4)})
    for row in rows:
      tool.expected_vdp(row)

  def test_vblank_comparator_checks_timing_and_full_write_stream(self):
    trace = bytes((1, 2, 0, 0, 0, 0, 3, 3, 0))
    expected = trace * 600
    frames = []
    for step in range(600):
      start = step * 1000
      reg, colours = tool.expected_vdp(trace)
      writes = [{"cycle": start + 10 + i, "kind": "cram", "address": i + 1, "value": c}
                for i, c in enumerate(colours)]
      writes.append({"cycle": start + 13, "kind": "reg", "address": 2, "value": reg})
      frames.append({"step": step, "vdp_frame": step + 100,
                     "start": start, "active": start + 100, "writes": writes})
    with self.assertRaisesRegex(tool.VBlankInconclusive, "provenance"):
      tool.compare_vblank(expected, frames, frame_period=1000)
    frames[300]["writes"][0]["cycle"] = frames[300]["active"]
    with self.assertRaisesRegex(ValueError, "step 300: write outside VBlank"):
      tool.compare_vblank(expected, frames, frame_period=1000)
    frames[300]["writes"][0]["cycle"] = frames[300]["start"] + 10
    frames[300]["writes"].append({"cycle": frames[300]["start"] + 14,
                                  "kind": "reg", "address": 1, "value": 0x80})
    with self.assertRaisesRegex(ValueError, "step 300: VDP write sequence differs"):
      tool.compare_vblank(expected, frames, frame_period=1000)

  def test_vblank_rejects_missing_frame_and_missing_independent_evidence(self):
    trace = bytes((1, 2, 0, 0, 0, 0, 3, 3, 0))
    expected = trace * 600
    frames = []
    for step in range(600):
      start = step * 1000
      reg, colours = tool.expected_vdp(trace)
      writes = [{"cycle": start + 10 + i, "kind": "cram", "address": i + 1, "value": c}
                for i, c in enumerate(colours)]
      writes.append({"cycle": start + 13, "kind": "reg", "address": 2, "value": reg})
      frames.append({"step": step, "vdp_frame": step + 100,
                     "start": start, "active": start + 100, "writes": writes})
    with self.assertRaisesRegex(tool.VBlankInconclusive, "period"):
      tool.compare_vblank(expected, frames)
    del frames[0]["vdp_frame"]
    with self.assertRaisesRegex(tool.VBlankInconclusive, "frame index"):
      tool.compare_vblank(expected, frames, frame_period=1000)
    frames[0]["vdp_frame"] = 100
    frames[300]["vdp_frame"] += 1
    with self.assertRaisesRegex(ValueError, "step 300: missing or duplicate VDP frame"):
      tool.compare_vblank(expected, frames, frame_period=1000)
    frames[300]["vdp_frame"] -= 1
    for frame in frames[300:]:
      frame["start"] += 1000
      frame["active"] += 1000
      for write in frame["writes"]:
        write["cycle"] += 1000
    with self.assertRaisesRegex(ValueError, "step 300: VBlank period"):
      tool.compare_vblank(expected, frames, frame_period=1000)

  def test_vblank_cli_cannot_claim_success_from_synthetic_metadata(self):
    trace = bytes((1, 2, 0, 0, 0, 0, 3, 3, 0))
    expected = Path(self.temp.name) / "vblank.expected"
    capture = Path(self.temp.name) / "vblank.json"
    expected.write_bytes(trace * 600)
    reg, colours = tool.expected_vdp(trace)
    frames = []
    for step in range(600):
      start = step * 1000
      writes = [{"cycle": start + 10 + i, "kind": "cram", "address": i + 1, "value": c}
                for i, c in enumerate(colours)]
      writes.append({"cycle": start + 13, "kind": "reg", "address": 2, "value": reg})
      frames.append({"step": step, "vdp_frame": step, "start": start,
                     "active": start + 100, "writes": writes})
    capture.write_text(json.dumps(frames))
    result = subprocess.run(
        [sys.executable, str(ROOT / "tests/sms_trace_tool.py"), "compare-vblank",
         "--expected", str(expected), "--capture", str(capture), "--frame-period", "1000"],
        capture_output=True, text=True)
    self.assertEqual(result.returncode, 2)
    self.assertIn("INCONCLUSIVE", result.stderr)
    self.assertNotIn("PASS", result.stdout + result.stderr)

  def test_pad_schedule_and_trace_comparator(self):
    schedule = tool.pad_schedule()
    expected = tool.pad_oracle(schedule, self.oracle)
    self.assertEqual([expected[s * 9] for s in (39, 40, 229, 230)], [0, 1, 1, 0])
    records = []
    for step in range(600):
      trace = expected[step * 9:step * 9 + 9]
      scene = 7 if 10 <= step < 200 else 0
      palette = 0 if step < 60 else 1 if step < 70 else 2 if step < 80 else 3 if step < 160 else 2
      records.append({"step": step, "buttons": schedule[step], "scene_ring": scene,
                      "palette_ring": palette, "visible": trace[0],
                      "peff": trace[1] + 4 * trace[2], "trace": list(trace[1:])})
    tool.compare_pad(expected, records)
    records[120]["palette_ring"] = 4
    with self.assertRaisesRegex(ValueError, "step 120: pad/scene"):
      tool.compare_pad(expected, records)
    records[120]["palette_ring"] = 3
    records[120]["trace"][3] ^= 1
    with self.assertRaisesRegex(ValueError, "step 120: pad field trace"):
      tool.compare_pad(expected, records)

  def test_detailed_pad_stream_matches_shared_trace_and_inputs(self):
    pads = tool.pad_schedule()
    detailed = capture_oracle.oracle(pads, self.oracle)
    trace = tool.pad_oracle(pads, self.oracle)
    self.assertEqual(b"".join(detailed[i * 27 + 18:i * 27 + 27] for i in range(600)), trace)
    self.assertEqual(detailed[:18], bytes((0, 0, 80, 255, 64, 0,
                                          0, 29, 58, 87, 116, 145, 174, 203,
                                          1, 1, 48, 80)))
    row = detailed[100 * 27:101 * 27]
    self.assertEqual(row[2:18], bytes((64, 64, 64, 64, 0, 0, 0, 0, 0, 0, 0, 0,
                                       1, 0, 0, 0)))
    self.assertEqual(len(capture_oracle.oracle(capture_oracle.schedule(), self.oracle)), 940 * 27)

  def assert_pad_inputs_match_c(self, pads):
    actual = capture_oracle.oracle(pads, self.oracle)
    reference = subprocess.run([str(self.pad_reference)], input=pads, capture_output=True, check=True).stdout
    self.assertEqual(len(reference), len(pads) * 16)
    for step in range(len(pads)):
      self.assertEqual(actual[step * 27 + 2:step * 27 + 18],
                       reference[step * 16:step * 16 + 16], f"poll {step}")
    return actual

  def test_procedural_inputs_match_c_and_sms_model_through_16bit_wrap(self):
    table = Path(self.temp.name) / "procedural.bin"
    table.write_bytes(tool.assets.procedural_inputs())
    model = subprocess.run([str(self.procedural_reference), str(table)],
                           capture_output=True, text=True, check=True)
    self.assertIn("65536 frames passed", model.stdout)
    actual = self.assert_pad_inputs_match_c(bytes(65553))
    self.assertEqual(actual[4096 * 27 + 16], 64)
    self.assertEqual(actual[65521 * 27 + 16], 63)
    self.assertEqual(actual[65536 * 27 + 16], 48)

  def test_long_trace_digital_transition_latches_across_release(self):
    for switch in (4096, 65535, 65536, 65537):
      pads = bytearray(switch + 16)
      pads[switch] = 0x10
      pads[switch + 2] = 0x20
      actual = self.assert_pad_inputs_match_c(bytes(pads))
      for offset, flags in ((0, 1), (1, 0), (2, 2), (3, 0), (15, 0)):
        step = switch + offset
        self.assertEqual(actual[step * 27 + 2:step * 27 + 18],
                         bytes((64, 64, 64, 64)) + bytes(8) + bytes((flags, 0, 0, 0)))


if __name__ == "__main__":
  unittest.main()
