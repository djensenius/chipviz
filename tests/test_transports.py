import json
import pathlib
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "host" / "bridge"))
sys.path.insert(0, str(ROOT / "shared" / "tools"))

import chipviz_bridge
import cvz_to_c
import live_bridge
import n64_joybus
import usb_hid


class TransportTests(unittest.TestCase):
  def setUp(self) -> None:
    self.frame = chipviz_bridge.make_procedural_frame(0)
    self.wire = chipviz_bridge.pack_frame(self.frame)

  def test_n64_joybus_packet_is_16_bytes(self) -> None:
    decoded = n64_joybus.decode_control_frame(self.wire)
    packet = n64_joybus.pack_joybus_frame(decoded)

    self.assertEqual(len(packet), n64_joybus.JOYBUS_SIZE)
    self.assertEqual(packet[2], self.frame.spectrum[0])
    self.assertEqual(packet[3], self.frame.spectrum[1])
    self.assertEqual(packet[14], self.frame.energy)
    self.assertEqual(packet[15], self.frame.notes[0].velocity)

  def test_n64_joybus_stream_conversion(self) -> None:
    stream = self.wire * 3
    converted = n64_joybus.convert_stream(stream)

    self.assertEqual(len(converted), 3 * n64_joybus.JOYBUS_SIZE)

  def test_usb_hid_mapping_outputs_buttons_and_keys(self) -> None:
    state = usb_hid.map_frame(self.wire)

    self.assertTrue(state.buttons & 0x01)
    self.assertIn("Space", state.keys)
    self.assertIn("Q", state.keys)
    self.assertEqual(state.left_x, self.frame.energy - 128)
    self.assertEqual(state.left_y, -127)

  def test_live_bridge_normalizes_input_stream(self) -> None:
    frames = live_bridge.frame_records(self.wire * 2)

    self.assertEqual(frames, [self.wire, self.wire])

  def test_cvz_to_c_renders_frame_array(self) -> None:
    header = cvz_to_c.render_header(self.wire, "chipviz_test_frames")

    self.assertIn("chipviz_test_frames_count = 1u", header)
    self.assertIn("0xC7", header)

  def test_cli_outputs_are_json_or_files(self) -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
      root = pathlib.Path(temp_dir)
      cvz = root / "input.cvz"
      cvz.write_bytes(self.wire)
      hid = root / "hid.json"
      joybus = root / "joybus.bin"
      header = root / "frames.h"

      self.assertEqual(usb_hid.main_from_args(["--input", str(cvz), "--output", str(hid)]), 0)
      self.assertEqual(n64_joybus.main_from_args(["--input", str(cvz), "--output", str(joybus)]), 0)
      self.assertEqual(cvz_to_c.main_from_args(["--input", str(cvz), "--symbol", "frames", "--output", str(header)]), 0)

      self.assertEqual(len(joybus.read_bytes()), n64_joybus.JOYBUS_SIZE)
      self.assertEqual(json.loads(hid.read_text(encoding="utf-8"))[0]["keys"][0], "1")
      self.assertIn("static const uint8_t frames[]", header.read_text(encoding="utf-8"))

  def test_cvz_to_c_rejects_empty_symbol(self) -> None:
    with self.assertRaisesRegex(ValueError, "symbol"):
      cvz_to_c.sanitize_symbol("")

  def test_cvz_to_c_header_guard_uses_symbol(self) -> None:
    header = cvz_to_c.render_header(self.wire, "custom_frames")

    self.assertIn("CHIPVIZ_GENERATED_CUSTOM_FRAMES_H", header)


if __name__ == "__main__":
  unittest.main()
