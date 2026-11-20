"""Host-only checks of independent SMS poll/oracle comparison."""

import copy
import unittest

import sms_capture_oracle as checker


class SmsCaptureOracleTests(unittest.TestCase):
  def setUp(self):
    self.pads = bytes((0, 1, 0))
    inputs = bytes(range(16))
    native = bytes(9)
    field = bytes((1, 2, 1, 3, 2, 1, 3, 2, 1))
    self.expected = (bytes((0, 0)) + inputs + native
                     + bytes((7, 0)) + inputs + field
                     + bytes((7, 0)) + inputs + native)
    self.capture = {"kind": "sms-z80-poll-v1", "start": "reset", "records": [
        {"step": 0, "point": "native", "buttons": 0, "scene": 0, "palette": 0,
         "visible": 0, "entry": 0, "input": list(inputs)},
        {"step": 1, "point": "field", "buttons": 1, "scene": 7, "palette": 0,
         "visible": 1, "entry": 1, "input": list(inputs), "peff": 6,
         "trace": list(field[1:])},
        {"step": 2, "point": "native", "buttons": 0, "scene": 7, "palette": 0,
         "visible": 0, "entry": 0, "input": list(inputs)},
    ]}

  def test_good_trace_and_bad_target_states(self):
    checker.compare(self.pads, self.expected, self.capture)
    for key, value in (("scene", 0), ("palette", 3), ("visible", 0), ("entry", 0),
                       ("buttons", 0), ("input", [0] * 16), ("peff", 7),
                       ("trace", [0] * 8)):
      wrong = copy.deepcopy(self.capture)
      wrong["records"][1][key] = value
      with self.subTest(key=key), self.assertRaisesRegex(ValueError, "step 1"):
        checker.compare(self.pads, self.expected, wrong)

  def test_incomplete_or_frame_only_capture_is_rejected(self):
    wrong = copy.deepcopy(self.capture)
    wrong["kind"] = "libretro-frames"
    with self.assertRaisesRegex(ValueError, "breakpoint capture"):
      checker.compare(self.pads, self.expected, wrong)
    wrong = copy.deepcopy(self.capture)
    wrong["records"].pop(0)
    with self.assertRaisesRegex(ValueError, "missing, extra"):
      checker.compare(self.pads, self.expected, wrong)
    wrong = copy.deepcopy(self.capture)
    wrong["records"][1]["step"] = 2
    with self.assertRaisesRegex(ValueError, "step 1"):
      checker.compare(self.pads, self.expected, wrong)

  def test_schedule_includes_warmup_and_release(self):
    pads = checker.schedule()
    self.assertEqual(len(pads), 940)
    self.assertEqual(pads[:120], bytes(120))
    self.assertEqual(list(pads[130:134]), [1, 1, 1, 0])

  def test_incomplete_coverage_is_rejected(self):
    with self.assertRaisesRegex(ValueError, "every safe mode"):
      checker.check_coverage(self.expected)
    rows = []
    for mode, family, layout, palette in ((0, 0, 0, 0), (1, 0, 1, 1),
                                          (2, 0, 2, 3), (2, 1, 3, 4)):
      rows.append(bytes((7, palette)) + bytes(16)
                  + bytes((1, mode, family, layout, 0, 0, 3, 3, 0)))
    rows.append(bytes(27))
    checker.check_coverage(b"".join(rows))
    rows[2] = bytes((7, 2)) + rows[2][2:]
    with self.assertRaisesRegex(ValueError, "unarmed relative palette"):
      checker.check_coverage(b"".join(rows))

  def test_gx_completed_snapshots_and_mismatches(self):
    pads = bytes(67)
    rows = []
    records = []
    events = [{"poll": i, "port": 0xdc, "pc": 0x02e4, "pad": 0, "frame": i + 10}
              for i in range(68)]
    for i in range(67):
      visible = int(1 <= i <= 65)
      mode = int(i == 31)
      field = bytes((visible, mode, 0, 0, 0, 0, 3, 3, 0)) if visible else bytes(9)
      rows.append(bytes((7 if i else 0, 0)) + bytes(16) + field)
      next_visible = int(i < 65)
      records.append({"step": i, "buttons": 0, "scene": 7 if i else 0,
                      "palette": 0, "input": [0] * 16, "visible": visible,
                      "entry": int(i == 1), "shown": next_visible,
                      "procedural_counter": i + 1, "pad_active": 0,
                      "peff": int(i == 30), "trace": list(field[1:])})
    expected = b"".join(rows)
    document = {"kind": "sms-gx-port-v1", "start": "reset", "events": events, "records": records}
    checker.compare_gx(pads, expected, document)
    for collection, index, key, bad in (
        ("events", 20, "pc", 0x222), ("events", 20, "pad", 1),
        ("events", 20, "frame", 29), ("records", 30, "input", [1] * 16),
        ("records", 30, "procedural_counter", 30),
        ("records", 30, "peff", 0), ("records", 30, "trace", [0] * 8),
        ("records", 30, "shown", 0), ("records", 1, "entry", 0)):
      wrong = copy.deepcopy(document)
      wrong[collection][index][key] = bad
      with self.subTest(key=key), self.assertRaises(ValueError):
        checker.compare_gx(pads, expected, wrong)
    wrong = copy.deepcopy(document)
    wrong["events"].pop()
    with self.assertRaisesRegex(ValueError, "missing, extra"):
      checker.compare_gx(pads, expected, wrong)


if __name__ == "__main__":
  unittest.main()
