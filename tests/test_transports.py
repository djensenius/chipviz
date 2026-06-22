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

  def test_live_bridge_audio_band_normalization(self) -> None:
    bands = live_bridge.normalize_bands([0.0, 1.0, 2.0, 4.0, 0.0, 8.0, 2.0, 1.0])

    self.assertEqual(len(bands), 8)
    self.assertEqual(bands[0], 0)
    self.assertEqual(bands[5], 255)

  def test_live_bridge_band_edges_rejects_invalid_band_count(self) -> None:
    for bands in (0, -1):
      with self.subTest(bands=bands):
        with self.assertRaisesRegex(ValueError, "bands must be at least 1"):
          live_bridge.band_edges(48000, bands)

  def test_live_bridge_frame_from_bands_packs_notes(self) -> None:
    wire = live_bridge.frame_from_bands(9, (255, 128, 64, 32, 16, 8, 4, 2), midi_velocity=90)
    decoded = n64_joybus.decode_control_frame(wire)

    self.assertEqual(decoded.frame, 9)
    self.assertEqual(decoded.spectrum[0], 255)
    self.assertEqual(decoded.note_velocities, (90,))

  def test_live_bridge_frame_from_bands_requires_exactly_eight_bands(self) -> None:
    with self.assertRaisesRegex(ValueError, "requires exactly 8 bands"):
      live_bridge.frame_from_bands(0, (1, 2, 3, 4))
    with self.assertRaisesRegex(ValueError, "requires exactly 8 bands"):
      live_bridge.frame_from_bands(0, (1, 2, 3, 4, 5, 6, 7, 8, 9))

  def test_live_bridge_frame_from_bands_clamps_midi_velocity(self) -> None:
    high_wire = live_bridge.frame_from_bands(0, (1, 2, 3, 4, 5, 6, 7, 8), midi_velocity=300)
    low_wire = live_bridge.frame_from_bands(1, (1, 2, 3, 4, 5, 6, 7, 8), midi_velocity=-5)
    high_decoded = n64_joybus.decode_control_frame(high_wire)
    low_decoded = n64_joybus.decode_control_frame(low_wire)

    self.assertEqual(len(high_wire), chipviz_bridge.WIRE_SIZE)
    self.assertEqual(len(low_wire), chipviz_bridge.WIRE_SIZE)
    self.assertEqual(high_decoded.note_velocities, (255,))
    self.assertEqual(low_decoded.note_velocities, ())

  def test_live_bridge_rejects_missing_optional_dependency(self) -> None:
    with self.assertRaisesRegex(RuntimeError, "definitely_missing_chipviz_module"):
      live_bridge.require_module("definitely_missing_chipviz_module", "nothing")

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
