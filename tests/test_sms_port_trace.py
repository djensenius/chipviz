"""Portable fake-core tests: no emulator or hardware timing evidence."""

import ctypes
import hashlib
import importlib.util
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import sms_port_trace as trace


HERE = Path(__file__).resolve().parent


class PortTraceTests(unittest.TestCase):
  @classmethod
  def setUpClass(cls):
    cls.temp = tempfile.TemporaryDirectory()
    cls.addClassCleanup(cls.temp.cleanup)
    cls.core_path = Path(cls.temp.name) / "fake.so"
    cls.shim_path = Path(cls.temp.name) / "shim.so"
    for source, output in (("sms_port_trace_fake.c", cls.core_path), ("sms_port_trace.c", cls.shim_path)):
      subprocess.run(["cc", "-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic",
                      "-shared", "-fPIC", str(HERE / source), "-o", str(output)], check=True)
    cls.profile = trace.Profile(hashlib.sha256(cls.core_path.read_bytes()).hexdigest())
    cls.core = ctypes.CDLL(str(cls.core_path))
    for name in ("fake_read", "fake_kind", "fake_port", "fake_value"):
      getattr(cls.core, name).argtypes = [ctypes.c_uint]
      getattr(cls.core, name).restype = ctypes.c_uint
    cls.core.fake_cycle.argtypes = [ctypes.c_uint32]
    cls.core.fake_write.argtypes = [ctypes.c_uint, ctypes.c_uint]
    cls.core.fake_count.restype = ctypes.c_uint

  def setUp(self):
    self.core.fake_reset()

  def open_trace(self, **kwargs):
    return trace.Trace(self.core_path, self.shim_path, profile=self.profile, **kwargs)

  def test_forward_once_and_order_and_restore_on_exception(self):
    with self.assertRaisesRegex(ValueError, "test exception"):
      with self.open_trace() as hook:
        hook.begin_frame(7)
        self.core.fake_cycle(100)
        self.assertEqual(self.core.fake_read(0x12dc), 0x1289)
        self.core.fake_cycle(370)
        self.core.fake_write(0x3fbf, 0x3f)
        hook.end_frame()
        events = hook.events()
        self.assertEqual([(e.frame, e.cycle, e.port, e.value, e.kind) for e in events],
                         [(7, 100, 0x12dc, 0x1289, 0), (7, 370, 0x3fbf, 0x3f, 1)])
        self.assertEqual(self.core.fake_count(), 2)
        self.assertEqual([(self.core.fake_kind(i), self.core.fake_port(i), self.core.fake_value(i))
                          for i in range(2)], [(0, 0x12dc, 0x1289), (1, 0x3fbf, 0x3f)])
        raise ValueError("test exception")
    self.assertEqual(self.core.fake_read(1), 0x54)
    self.core.fake_write(2, 3)
    self.assertEqual(self.core.fake_count(), 4)
    with self.open_trace():
      pass

  def test_overflow_fails_but_still_forwards(self):
    hook = self.open_trace(capacity=1)
    try:
      hook.begin_frame(0)
      self.core.fake_write(1, 2)
      self.assertEqual(self.core.fake_read(3), 0x56)
      with self.assertRaisesRegex(RuntimeError, "loss"):
        hook.end_frame()
      with self.assertRaisesRegex(RuntimeError, "loss"):
        hook.events()
      self.assertEqual(self.core.fake_count(), 2)
    finally:
      with self.assertRaisesRegex(RuntimeError, "loss"):
        hook.close()

  def test_outside_frame_is_loss_and_ratio_change_fails(self):
    hook = self.open_trace()
    try:
      self.core.fake_write(1, 2)
      with self.assertRaisesRegex(RuntimeError, "loss"):
        hook.events()
    finally:
      with self.assertRaisesRegex(RuntimeError, "loss"):
        hook.close()
    hook = self.open_trace()
    try:
      hook.begin_frame(0)
      ratio = ctypes.c_uint32.in_dll(self.core, "z80_cycle_ratio")
      ratio.value = 0
      self.core.fake_write(1, 2)
      with self.assertRaisesRegex(RuntimeError, "loss|ratio"):
        hook.end_frame()
    finally:
      with self.assertRaisesRegex(RuntimeError, "ratio"):
        hook.close()

  def test_exception_in_frame_restores_and_reports_incomplete(self):
    with self.assertRaisesRegex(ValueError, "original") as caught:
      with self.open_trace() as hook:
        hook.begin_frame(0)
        raise ValueError("original")
    self.assertTrue(any("unfinished frame" in note for note in caught.exception.__notes__))
    with self.open_trace():
      pass

  def test_unfinished_frame_fails_on_close(self):
    with self.assertRaisesRegex(RuntimeError, "unfinished frame"):
      with self.open_trace() as hook:
        hook.begin_frame(0)
    with self.open_trace():
      pass

  def test_boundaries_and_counter_reset(self):
    with self.open_trace() as hook:
      hook.begin_frame(0)
      self.core.fake_cycle(0xfffffff0)
      self.core.fake_write(1, 2)
      hook.end_frame()
      hook.begin_frame(1)
      self.core.fake_cycle(3)
      self.core.fake_write(1, 2)
      hook.end_frame()
      self.assertEqual([e.cycle for e in hook.events()], [0xfffffff0, 3])
      with self.assertRaisesRegex(RuntimeError, "boundary"):
        hook.begin_frame(1)
      with self.assertRaisesRegex(RuntimeError, "boundary"):
        hook.begin_frame(3)
      hook.begin_frame(2)
      self.core.fake_cycle(0xffffffff)
      self.core.fake_write(1, 2)
      self.core.fake_cycle(0)
      self.core.fake_write(1, 2)
      hook.end_frame()
      with self.assertRaisesRegex(RuntimeError, "counter decreased"):
        hook.events()

  def test_fail_closed_before_install(self):
    with self.assertRaisesRegex(RuntimeError, "hash"):
      trace.Trace(self.core_path, self.shim_path)
    with self.assertRaisesRegex(RuntimeError, "missing core symbol"):
      trace.Trace(self.core_path, self.shim_path,
                  profile=trace.Profile(self.profile.sha256, read_symbol="missing"))
    self.core.fake_replace_read()
    with self.assertRaisesRegex(RuntimeError, "identity"):
      self.open_trace()
    self.core.fake_reset()
    ctypes.c_uint32.in_dll(self.core, "z80_cycle_ratio").value = 0
    with self.assertRaisesRegex(RuntimeError, "ratio"):
      self.open_trace()
    self.core.fake_reset()
    with self.assertRaisesRegex(RuntimeError, "installation"):
      self.open_trace(capacity=4097)

  def test_singleton_and_pointer_replacement_fail_closed(self):
    with self.open_trace() as hook:
      with self.assertRaisesRegex(RuntimeError, "identity"):
        self.open_trace()
      other_path = Path(self.temp.name) / "other.so"
      shutil.copyfile(self.core_path, other_path)
      with self.assertRaisesRegex(RuntimeError, "installation"):
        trace.Trace(other_path, self.shim_path, profile=self.profile)
      self.core.fake_replace_read()
      with self.assertRaisesRegex(RuntimeError, "pointer"):
        hook.begin_frame(0)
      with self.assertRaisesRegex(RuntimeError, "pointer"):
        hook.events()
      with self.assertRaisesRegex(RuntimeError, "pointer"):
        hook.close()

  def test_changed_pointer_is_not_overwritten_and_other_restored(self):
    hook = self.open_trace()
    self.core.fake_replace_read()
    replacement = ctypes.c_void_p.in_dll(self.core, "z80_readport").value
    with self.assertRaisesRegex(RuntimeError, "pointer changed"):
      hook.close()
    self.assertEqual(ctypes.c_void_p.in_dll(self.core, "z80_readport").value, replacement)
    self.assertEqual(ctypes.c_void_p.in_dll(self.core, "z80_writeport").value,
                     ctypes.cast(self.core.z80_ms_port_w, ctypes.c_void_p).value)

  def test_seed_exact_sequence_and_spacing(self):
    values = (4, 0x80, 0xe0, 0x81, 0xff, 0x82, 0, 0xc0, 0x3f, 3, 0x0c, 0x30)
    with self.open_trace() as hook:
      hook.begin_frame(0)
      for i, value in enumerate(values):
        self.core.fake_cycle(100 + i * 270)
        self.core.fake_write(0xbf if i < 8 else 0xbe, value)
      hook.end_frame()
      events = hook.events()
      trace.check_seed(events)
      for bad in (events[:-1], events + events[-1:], events[:3] + events[4:]):
        with self.assertRaisesRegex(RuntimeError, "missing, duplicate"):
          trace.check_seed(bad)
      events[1].cycle += 1
      with self.assertRaisesRegex(RuntimeError, "spacing"):
        trace.check_seed(events)
      events[1].cycle -= 1
      events[1].frame += 1
      with self.assertRaisesRegex(RuntimeError, "spacing"):
        trace.check_seed(events)

  def test_seed_fixture_matches_generated_rom(self):
    spec = importlib.util.spec_from_file_location(
      "build_homebrew", HERE.parent / "shared/tools/build_homebrew.py")
    builder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(builder)
    rom = builder.build_sms_rom()
    events = []
    for offset in range(4, 52, 4):
      self.assertEqual((rom[offset], rom[offset + 2]), (0x3e, 0xd3))
      events.append(trace.Event(0, 100 + len(events) * 270, rom[offset + 3], rom[offset + 1], 1))
    self.assertEqual(rom[52], 0x76)
    trace.check_seed(events)


if __name__ == "__main__":
  unittest.main()
