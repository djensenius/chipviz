"""Host-only tests of the SMS frame-capture adapter and evidence checks."""

import ctypes
import unittest

import sms_capture_check as check
import sms_libretro_capture as capture


class SmsLibretroCaptureTests(unittest.TestCase):
  def test_decode_frame_formats_and_padding(self):
    # One red and one blue pixel, followed by two ignored padding bytes.
    rgb565 = bytes.fromhex("00 f8 1f 00 aa bb")
    self.assertEqual(capture.decode_frame(rgb565, 2, 1, 6, capture.PIXEL_RGB565),
                     bytes((255, 0, 0, 0, 0, 255)))
    rgb1555 = bytes.fromhex("00 7c 1f 00")
    self.assertEqual(capture.decode_frame(rgb1555, 2, 1, 4, capture.PIXEL_1555),
                     bytes((255, 0, 0, 0, 0, 255)))
    xrgb = bytes.fromhex("00 00 ff 00 ff 00 00 00")
    self.assertEqual(capture.decode_frame(xrgb, 2, 1, 8, capture.PIXEL_XRGB8888),
                     bytes((255, 0, 0, 0, 0, 255)))
    with self.assertRaises(ValueError):
      capture.decode_frame(rgb565[:-1], 2, 1, 6, capture.PIXEL_RGB565)

  def test_option_lookup_clears_stale_value(self):
    core = capture.SmsCore.__new__(capture.SmsCore)
    core.options_seen = {}
    core._options = {b"known": ctypes.c_char_p(b"100%")}
    variable = capture.Variable(b"known", None)
    self.assertTrue(core.environment(capture.ENV_GET_VARIABLE, ctypes.addressof(variable)))
    self.assertEqual(variable.value, b"100%")
    variable.key = b"unknown"
    self.assertFalse(core.environment(capture.ENV_GET_VARIABLE, ctypes.addressof(variable)))
    self.assertIsNone(variable.value)

  def test_input_mapping_and_release(self):
    core = capture.SmsCore.__new__(capture.SmsCore)
    core.pad, core.input_calls = 0x31, 0
    self.assertEqual([core.input(0, capture.DEVICE_JOYPAD, 0, i) for i in (4, 5, 0, 8)], [1, 0, 1, 1])
    self.assertEqual(core.input(1, capture.DEVICE_JOYPAD, 0, 4), 0)
    core.pad = 0
    self.assertEqual(core.input(0, capture.DEVICE_JOYPAD, 0, 4), 0)
    self.assertEqual([capture.schedule(i) for i in (10, 11, 12, 13, 180, 181, 680, 750)],
                     [1, 1, 1, 0, 16, 0, 12, 1])

  def test_snapshot_includes_input_and_procedural_phase(self):
    ram = bytearray(0x110)
    ram[0x20:0x30] = bytes(range(16))
    ram[0x30:0x32] = bytes((0x34, 0x12))
    ram[0x33] = 1
    snapshot = capture.target_snapshot(ram)
    self.assertEqual(snapshot["input"], list(range(16)))
    self.assertEqual(snapshot["procedural_counter"], 0x1234)
    self.assertEqual(snapshot["pad_active"], 1)
    with self.assertRaisesRegex(ValueError, "RAM unavailable"):
      capture.target_snapshot(ram[:0x100])

  def test_rgb565_quantization_and_png_integrity(self):
    ideal = bytes((0, 85, 170, 255, 170, 0))
    decoded = check.quantize(ideal, capture.PIXEL_RGB565)
    self.assertEqual(decoded, bytes((0, 85, 173, 255, 170, 0)))
    image = capture.png(2, 1, decoded)
    self.assertEqual(check.png_rgb(image), (2, 1, decoded))
    broken = bytearray(image)
    broken[-5] ^= 1
    with self.assertRaisesRegex(ValueError, "invalid PNG chunk"):
      check.png_rgb(broken)


if __name__ == "__main__":
  unittest.main()
