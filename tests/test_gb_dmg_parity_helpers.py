"""Portable checks for the opt-in DMG fixture capture protocol."""

import pathlib
import sys
import tempfile
import types
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import gb_dmg_parity_capture as capture
import gb_libretro_capture as glc


NAMES = (
  "wReady", "wFxSeq", "wAckSeq", "wLastSeq", "wStep", "wFrame",
  "wFxCtl", "wFxScene", "wFxPin", "wTraceShow", "wShownShow",
  "wShSCX", "wShSCY", "wShBGP", "wShLCDC", "wShown7", "wShowMode",
)
SYMBOLS = {name: index * 4 for index, name in enumerate(NAMES)}
CAPTURE_RUN = capture.CaptureCore.capture_run


class FakeCore:
  """Small WRAM/mailbox fake; callback failures never require SameBoy."""

  def __init__(self, *, acknowledge=True, corrupt_wrap=False, missing_callback=0,
               unequal_callback=0, repeat_frame=False):
    self.mem = bytearray(256)
    self.acknowledge = acknowledge
    self.corrupt_wrap = corrupt_wrap
    self.missing_callback = missing_callback
    self.unequal_callback = unequal_callback
    self.repeat_frame = repeat_frame
    self.callbacks = []
    self.frame = None
    self.runs = 0
    self.capture_calls = 0
    self.closed = False

  def read(self, address, size=1):
    return bytes(self.mem[address:address + size])

  def write(self, address, data):
    self.mem[address:address + len(data)] = data

  def get(self, name):
    return self.mem[SYMBOLS[name]]

  def set(self, name, value):
    self.mem[SYMBOLS[name]] = value

  def reset(self):
    pass

  def run_frame(self):
    self.runs += 1
    if not self.get("wReady"):
      self.set("wReady", 0xA5)
      self.set("wShBGP", 0xE4)
      self.set("wShLCDC", 0x91)
    elif self.acknowledge and self.get("wFxSeq") != self.get("wLastSeq"):
      step = int.from_bytes(self.read(SYMBOLS["wStep"], 2), "little") + 1
      self.write(SYMBOLS["wStep"], step.to_bytes(2, "little"))
      self.set("wAckSeq", self.get("wFxSeq"))
      self.set("wLastSeq", self.get("wFxSeq"))
      scx, scy, bgp = capture.registers(step)
      self.set("wFrame", 1 if self.corrupt_wrap and step == 256 else scy)
      self.set("wShSCX", scx)
      self.set("wShSCY", scy)
      self.set("wShBGP", bgp)
    step = int.from_bytes(self.read(SYMBOLS["wStep"], 2), "little")
    if self.capture_calls != self.missing_callback or not self.capture_calls:
      value = 1 if self.repeat_frame and step == 2 else step & 255
      if self.capture_calls == self.unequal_callback and self.capture_calls:
        value ^= 1
      self.frame = glc.Frame(160, 144, bytes([value]) * capture.FRAME_BYTES)
      self.callbacks.append(self.frame)

  def capture_run(self):
    self.capture_calls += 1
    return CAPTURE_RUN(self)

  def close(self):
    self.closed = True


class DmgParityHelpers(unittest.TestCase):
  def test_freshness_and_frame_shape(self):
    frame = glc.Frame(160, 144, bytes(capture.FRAME_BYTES))
    self.assertEqual(capture.fresh_frame([frame]), frame.rgb)
    for callbacks in ([], [None], [frame, frame], [glc.Frame(160, 144, b"")],
                      [glc.Frame(144, 160, frame.rgb)]):
      with self.subTest(callbacks=callbacks), self.assertRaises(ValueError):
        capture.fresh_frame(callbacks)

  def test_a_missing_new_callback_cannot_reuse_the_previous_frame(self):
    old = glc.Frame(160, 144, bytes(capture.FRAME_BYTES))
    fake = types.SimpleNamespace(frame=old, callbacks=[old], run_frame=lambda: None)
    with self.assertRaisesRegex(ValueError, "missing"):
      capture.CaptureCore.capture_run(fake)
    self.assertIsNone(fake.frame)

  def test_palette_boundaries_and_wrap(self):
    for step, expected in (
      (1, (128, 1, 0xE4)), (15, (0x87, 15, 0xE4)),
      (16, (8, 16, 0xEC)), (31, (0x8F, 31, 0xEC)),
      (32, (16, 32, 0xF4)), (47, (0x97, 47, 0xF4)),
      (48, (24, 48, 0xFC)), (63, (0x9F, 63, 0xFC)),
      (64, (32, 64, 0xE4)), (255, (255, 255, 0xFC)),
      (256, (0, 0, 0xE4)), (257, (128, 1, 0xE4)),
    ):
      with self.subTest(step=step):
        self.assertEqual(capture.registers(step), expected)

  def test_missing_symbol_is_rejected(self):
    with tempfile.TemporaryDirectory() as directory:
      path = pathlib.Path(directory) / "fixture.sym"
      path.write_text("00:c000 wFrame\n")
      with self.assertRaisesRegex(ValueError, "missing symbols"):
        capture.symbols(path)

  def run_fake(self, fake, steps):
    with mock.patch.object(capture, "CaptureCore", return_value=fake), \
         mock.patch.object(capture, "STEPS", steps):
      return capture.capture(None, None, SYMBOLS)

  def test_records_use_acknowledged_16_bit_steps_across_wrap(self):
    fake = FakeCore()
    data = self.run_fake(fake, (1, 256, 257))
    self.assertEqual(len(data), 3 * capture.RECORD_BYTES)
    for index, step in enumerate((1, 256, 257)):
      offset = index * capture.RECORD_BYTES
      self.assertEqual(data[offset:offset + 2], step.to_bytes(2, "little"))
      self.assertEqual(data[offset + 2:offset + capture.RECORD_BYTES],
                       bytes([step & 255]) * capture.FRAME_BYTES)
    self.assertTrue(fake.closed)

  def test_missing_acknowledgement_is_bounded(self):
    fake = FakeCore(acknowledge=False)
    with self.assertRaisesRegex(AssertionError, "step 1 not acknowledged"):
      self.run_fake(fake, (1,))
    self.assertEqual(fake.runs, 5)  # boot, then at most four attempts
    self.assertTrue(fake.closed)

  def test_wrong_native_counter_at_wrap_is_rejected(self):
    fake = FakeCore(corrupt_wrap=True)
    with self.assertRaisesRegex(AssertionError, "unexpected native state at step 256"):
      self.run_fake(fake, (256,))
    self.assertEqual(fake.get("wAckSeq"), 0)
    self.assertTrue(fake.closed)

  def test_missing_or_unequal_frozen_callbacks_are_rejected(self):
    for fake, error in (
      (FakeCore(missing_callback=2), "missing, duplicate, or null"),
      (FakeCore(unequal_callback=3), "frozen frames differ at step 1"),
    ):
      with self.subTest(error=error), self.assertRaisesRegex((ValueError, AssertionError), error):
        self.run_fake(fake, (1,))
      self.assertTrue(fake.closed)

  def test_repeated_frozen_frame_across_steps_is_rejected(self):
    fake = FakeCore(repeat_frame=True)
    with self.assertRaisesRegex(AssertionError, "consecutive selected states.*step 2"):
      self.run_fake(fake, (1, 2))
    self.assertTrue(fake.closed)


if __name__ == "__main__":
  unittest.main()
