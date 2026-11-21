"""Portable fake-core tests: no emulator or hardware timing evidence."""

import ctypes
import gc
import hashlib
import importlib.util
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock
import weakref

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
    cls.digest = hashlib.sha256(cls.core_path.read_bytes()).hexdigest()
    cls.layout = trace.Layout(size=68, cycle_offset=64)
    cls.layouts = mock.patch.dict(trace._VETTED_LAYOUTS, {cls.digest: cls.layout})
    cls.layouts.start()
    cls.addClassCleanup(cls.layouts.stop)
    cls.core = ctypes.CDLL(str(cls.core_path))
    for name in ("fake_read", "fake_kind", "fake_port", "fake_value"):
      getattr(cls.core, name).argtypes = [ctypes.c_uint]
      getattr(cls.core, name).restype = ctypes.c_uint
    cls.core.fake_cycle.argtypes = [ctypes.c_uint32]
    cls.core.fake_write.argtypes = [ctypes.c_uint, ctypes.c_uint]
    cls.core.fake_count.restype = ctypes.c_uint64
    cls.core.z80_ms_port_r.argtypes = [ctypes.c_uint]
    cls.core.z80_ms_port_r.restype = ctypes.c_ubyte
    cls.core.z80_ms_port_w.argtypes = [ctypes.c_uint, ctypes.c_ubyte]
    cls.core.z80_ms_port_w.restype = None

  def setUp(self):
    self.core.fake_reset()

  def open_trace(self, **kwargs):
    return trace.Trace(self.core_path, self.shim_path, **kwargs)

  def assert_original_slots(self):
    self.assertEqual(ctypes.c_void_p.in_dll(self.core, "z80_readport").value,
                     ctypes.cast(self.core.z80_ms_port_r, ctypes.c_void_p).value)
    self.assertEqual(ctypes.c_void_p.in_dll(self.core, "z80_writeport").value,
                     ctypes.cast(self.core.z80_ms_port_w, ctypes.c_void_p).value)

  def test_exception_after_native_install_restores(self):
    bind = trace.Trace._bind
    def interrupted_bind(hook):
      bind(hook)
      native_install = hook.shim.sms_trace_install
      def interrupted(*args):
        native_install(*args)
        raise KeyboardInterrupt("after install")
      hook.shim.sms_trace_install = interrupted
    with mock.patch.object(trace.Trace, "_bind", interrupted_bind):
      with self.assertRaisesRegex(KeyboardInterrupt, "after install"):
        self.open_trace()
    self.assert_original_slots()

  def test_exception_before_native_uninstall_restores(self):
    hook = self.open_trace()
    def interrupted(_hook):
      raise KeyboardInterrupt("before uninstall")
    with mock.patch.object(trace.Trace, "_uninstall", interrupted, create=True):
      with self.assertRaisesRegex(KeyboardInterrupt, "before uninstall"):
        hook.close()
    self.assert_original_slots()

  def test_install_exception_preserves_replaced_slot_and_original_error(self):
    native_install = trace.Trace._install
    def interrupted(hook, *args):
      self.assertEqual(native_install(hook, *args), 0)
      self.core.fake_replace_read()
      raise KeyboardInterrupt("install interruption")
    with mock.patch.object(trace.Trace, "_install", interrupted):
      with self.assertRaisesRegex(KeyboardInterrupt, "install interruption") as caught:
        self.open_trace()
    self.assertTrue(any("pointer changed" in note for note in caught.exception.__notes__))
    self.assertNotEqual(ctypes.c_void_p.in_dll(self.core, "z80_readport").value,
                        ctypes.cast(self.core.z80_ms_port_r, ctypes.c_void_p).value)
    self.assertEqual(ctypes.c_void_p.in_dll(self.core, "z80_writeport").value,
                     ctypes.cast(self.core.z80_ms_port_w, ctypes.c_void_p).value)

  def test_exception_after_native_uninstall_restores(self):
    hook = self.open_trace()
    native_uninstall = getattr(trace.Trace, "_uninstall", None)
    def interrupted(owner):
      native_uninstall(owner)
      raise KeyboardInterrupt("after uninstall")
    with mock.patch.object(trace.Trace, "_uninstall", interrupted, create=True):
      with self.assertRaisesRegex(KeyboardInterrupt, "after uninstall"):
        hook.close()
    self.assert_original_slots()

  def test_interrupted_cleanup_preserves_replaced_slot_and_original_error(self):
    hook = self.open_trace()
    self.core.fake_replace_read()
    replacement = ctypes.c_void_p.in_dll(self.core, "z80_readport").value
    def interrupted(_hook):
      raise KeyboardInterrupt("original interruption")
    with mock.patch.object(trace.Trace, "_uninstall", interrupted):
      with self.assertRaisesRegex(KeyboardInterrupt, "original interruption") as caught:
        hook.close()
    self.assertTrue(any("pointer changed" in note for note in caught.exception.__notes__))
    self.assertEqual(ctypes.c_void_p.in_dll(self.core, "z80_readport").value, replacement)
    self.assertEqual(ctypes.c_void_p.in_dll(self.core, "z80_writeport").value,
                     ctypes.cast(self.core.z80_ms_port_w, ctypes.c_void_p).value)

  def test_context_preserves_original_exception_if_uninstall_interrupts(self):
    def interrupted(_hook):
      raise KeyboardInterrupt("cleanup interruption")
    with mock.patch.object(trace.Trace, "_uninstall", interrupted):
      with self.assertRaisesRegex(ValueError, "original exception") as caught:
        with self.open_trace():
          raise ValueError("original exception")
    self.assertTrue(any("cleanup interruption" in note for note in caught.exception.__notes__))
    self.assert_original_slots()

  def test_cleanup_failure_keeps_owner_for_retry(self):
    hook = self.open_trace()
    def interrupted(_hook):
      raise KeyboardInterrupt("original interruption")
    with mock.patch.object(trace.Trace, "_uninstall", interrupted):
      with mock.patch.object(hook, "_native_uninstall", side_effect=RuntimeError("cleanup failed")):
        with self.assertRaisesRegex(KeyboardInterrupt, "original interruption") as caught:
          hook.close()
    self.assertTrue(hook.active)
    self.assertTrue(any("cleanup failed" in note for note in caught.exception.__notes__))
    hook.close()
    self.assert_original_slots()

  def test_failed_construction_retains_libraries_for_retry(self):
    bind = trace.Trace._bind
    install = trace.Trace._install
    def interrupted_bind(hook):
      bind(hook)
      def cleanup_fails(*_args):
        raise RuntimeError("second interruption")
      hook._native_uninstall = cleanup_fails
    def interrupted_install(hook, *args):
      self.assertEqual(install(hook, *args), 0)
      raise KeyboardInterrupt("original interruption")
    with mock.patch.object(trace.Trace, "_bind", interrupted_bind):
      with mock.patch.object(trace.Trace, "_install", interrupted_install):
        with self.assertRaisesRegex(KeyboardInterrupt, "original interruption") as caught:
          self.open_trace()
    owner = caught.exception.trace_owner
    self.assertTrue(any("second interruption" in note for note in caught.exception.__notes__))
    self.assertIn(owner, trace.retained_traces())
    owner_ref, core_ref, shim_ref = weakref.ref(owner), weakref.ref(owner.core), weakref.ref(owner.shim)
    del owner, caught
    gc.collect()
    owner = owner_ref()
    self.assertIsNotNone(owner)
    self.assertIs(core_ref(), owner.core)
    self.assertIs(shim_ref(), owner.shim)
    with self.assertRaisesRegex(RuntimeError, "second interruption"):
      owner.retry_cleanup()
    self.assertIn(owner, trace.retained_traces())
    owner._native_uninstall = owner.shim.sms_trace_uninstall
    owner.retry_cleanup()
    self.assertNotIn(owner, trace.retained_traces())
    self.assert_original_slots()

  def test_failed_second_construction_retry_preserves_other_owner(self):
    with self.open_trace() as existing:
      other_path = Path(self.temp.name) / "second-owner.so"
      shutil.copyfile(self.core_path, other_path)
      bind = trace.Trace._bind
      install = trace.Trace._install
      native_query = []
      def interrupted_bind(hook):
        bind(hook)
        native_query.append(hook.shim.sms_trace_owned)
        def cleanup_fails(*_args):
          raise RuntimeError("query interruption")
        hook.shim.sms_trace_owned = cleanup_fails
      def interrupted_install(hook, *args):
        self.assertNotEqual(install(hook, *args), 0)
        raise KeyboardInterrupt("second installation")
      with mock.patch.object(trace.Trace, "_bind", interrupted_bind):
        with mock.patch.object(trace.Trace, "_install", interrupted_install):
          with self.assertRaisesRegex(KeyboardInterrupt, "second installation") as caught:
            trace.Trace(other_path, self.shim_path)
      failed = caught.exception.trace_owner
      self.assertIn(failed, trace.retained_traces())
      failed.shim.sms_trace_owned = native_query[0]
      failed.retry_cleanup()
      self.assertNotIn(failed, trace.retained_traces())
      existing.begin_frame(0)
      self.core.fake_write(1, 2)
      existing.end_frame()
      self.assertEqual(len(existing.events()), 1)

  def test_failed_construction_registry_when_exception_rejects_attribute(self):
    class LockedError(Exception):
      def __setattr__(self, name, value):
        if name == "trace_owner":
          raise AttributeError("locked")
        super().__setattr__(name, value)
    before = set(trace.retained_traces())
    bind, install = trace.Trace._bind, trace.Trace._install
    def interrupted_bind(hook):
      bind(hook)
      def cleanup_fails(*_args):
        raise RuntimeError("cleanup interruption")
      hook._native_uninstall = cleanup_fails
    def interrupted_install(hook, *args):
      self.assertEqual(install(hook, *args), 0)
      raise LockedError("original")
    with mock.patch.object(trace.Trace, "_bind", interrupted_bind):
      with mock.patch.object(trace.Trace, "_install", interrupted_install):
        with self.assertRaisesRegex(LockedError, "original"):
          self.open_trace()
    retained = set(trace.retained_traces()) - before
    self.assertEqual(len(retained), 1)
    owner = retained.pop()
    owner._native_uninstall = owner.shim.sms_trace_uninstall
    owner.retry_cleanup()
    self.assert_original_slots()

  def test_forward_once_and_order_and_restore_on_exception(self):
    with self.assertRaisesRegex(ValueError, "test exception"):
      with self.open_trace() as hook:
        hook.begin_frame(7)
        self.core.fake_cycle(100)
        self.assertEqual(self.core.fake_read(0x12dc), 0x89)
        self.core.fake_cycle(370)
        self.core.fake_write(0x3fbf, 0x3f)
        hook.end_frame()
        events = hook.events()
        self.assertEqual([(e.frame, e.cycle, e.port, e.value, e.kind) for e in events],
                         [(7, 100, 0x12dc, 0x89, 0), (7, 370, 0x3fbf, 0x3f, 1)])
        self.assertEqual(self.core.fake_count(), 2)
        self.assertEqual([(self.core.fake_kind(i), self.core.fake_port(i), self.core.fake_value(i))
                          for i in range(2)], [(0, 0x12dc, 0x89), (1, 0x3fbf, 0x3f)])
        raise ValueError("test exception")
    self.assertEqual(self.core.fake_read(1), 0x54)
    self.core.fake_write(2, 3)
    self.assertEqual(self.core.fake_count(), 4)
    with self.open_trace():
      pass

  def test_byte_abi_forwards_truncated_values(self):
    with self.open_trace() as hook:
      hook.begin_frame(0)
      self.assertEqual(self.core.fake_read(0x12ff), 0xaa)
      self.core.fake_write(0x56bf, 0x1fe)
      hook.end_frame()
      self.assertEqual([(e.port, e.value, e.kind) for e in hook.events()],
                       [(0x12ff, 0xaa, 0), (0x56bf, 0xfe, 1)])
      self.assertEqual(self.core.fake_count(), 2)
      self.assertEqual([self.core.fake_value(i) for i in range(2)], [0xaa, 0xfe])

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

  def test_full_capacity_and_fake_log_bounds(self):
    hook = self.open_trace()
    try:
      hook.begin_frame(0)
      for i in range(4096):
        self.core.fake_write(i, i + 1)
      hook.end_frame()
      events = hook.events()
      self.assertEqual(len(events), 4096)
      self.assertEqual((events[-1].port, events[-1].value), (4095, 0))
      self.assertEqual(self.core.fake_count(), 4096)
      self.assertEqual(self.core.fake_value(63), 64)
      self.assertEqual(self.core.fake_value(64), 0xffffffff)
      hook.begin_frame(1)
      self.assertEqual(self.core.fake_read(7), 7 ^ 0x55)
      with self.assertRaisesRegex(RuntimeError, "loss"):
        hook.end_frame()
      self.assertEqual(self.core.fake_count(), 4097)
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

  def test_interruption_after_native_begin_invalidates_trace(self):
    with self.assertRaisesRegex(RuntimeError, "interrupted"):
      with self.open_trace() as hook:
        native_begin = hook.shim.sms_trace_begin
        def interrupted(frame):
          self.assertEqual(native_begin(frame), 0)
          raise KeyboardInterrupt("after begin")
        with mock.patch.object(hook.shim, "sms_trace_begin", interrupted):
          with self.assertRaisesRegex(KeyboardInterrupt, "after begin"):
            hook.begin_frame(0)
        with self.assertRaisesRegex(RuntimeError, "interrupted"):
          hook.events()
    self.assert_original_slots()

  def test_interruption_before_native_end_invalidates_trace(self):
    with self.assertRaisesRegex(RuntimeError, "interrupted"):
      with self.open_trace() as hook:
        hook.begin_frame(0)
        self.core.fake_write(1, 2)
        with mock.patch.object(hook.shim, "sms_trace_end", side_effect=KeyboardInterrupt("before end")):
          with self.assertRaisesRegex(KeyboardInterrupt, "before end"):
            hook.end_frame()
        with self.assertRaisesRegex(RuntimeError, "interrupted"):
          hook.events()
    self.assert_original_slots()

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
    with mock.patch.dict(trace._VETTED_LAYOUTS, {}, clear=True):
      with self.assertRaisesRegex(RuntimeError, "hash"):
        self.open_trace()
    with mock.patch.dict(trace._VETTED_LAYOUTS, {self.digest: trace.Layout(68, 64, read_symbol="missing")}):
      with self.assertRaisesRegex(RuntimeError, "missing core symbol"):
        self.open_trace()
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

  def test_invalid_layouts_fail_before_library_access(self):
    for layout in (trace.Layout(68, -4), trace.Layout(68, 1), trace.Layout(68, 68),
                   trace.Layout(3, 0), trace.Layout(68, 0x100000), trace.Layout(68, True)):
      with self.subTest(layout=layout):
        with mock.patch.dict(trace._VETTED_LAYOUTS, {self.digest: layout}):
          with mock.patch.object(trace.ctypes, "CDLL", side_effect=AssertionError("library accessed")):
            with self.assertRaisesRegex(RuntimeError, "invalid core layout"):
              self.open_trace()
        self.assert_original_slots()

  def test_singleton_and_pointer_replacement_fail_closed(self):
    with self.open_trace() as hook:
      with self.assertRaisesRegex(RuntimeError, "identity"):
        self.open_trace()
      other_path = Path(self.temp.name) / "other.so"
      shutil.copyfile(self.core_path, other_path)
      with self.assertRaisesRegex(RuntimeError, "installation"):
        trace.Trace(other_path, self.shim_path)
      self.core.fake_replace_read()
      with self.assertRaisesRegex(RuntimeError, "pointer"):
        hook.begin_frame(0)
      with self.assertRaisesRegex(RuntimeError, "pointer"):
        hook.events()
      with self.assertRaisesRegex(RuntimeError, "pointer"):
        hook.close()

  def test_failed_second_install_interruption_cannot_uninstall_owner(self):
    with self.open_trace() as owner:
      other_token = ctypes.c_byte()
      wrong_keys = (ctypes.byref(other_token), *owner._keys[1:])
      self.assertEqual(owner.shim.sms_trace_owned(*wrong_keys), 0)
      self.assertNotEqual(owner.shim.sms_trace_uninstall(*wrong_keys), 0)
      wrong_slot = ctypes.c_void_p()
      wrong_keys = (owner._keys[0], ctypes.byref(wrong_slot), owner._keys[2])
      self.assertEqual(owner.shim.sms_trace_owned(*wrong_keys), 0)
      self.assertNotEqual(owner.shim.sms_trace_uninstall(*wrong_keys), 0)
      other_path = Path(self.temp.name) / "other-interrupted.so"
      shutil.copyfile(self.core_path, other_path)
      native_install = trace.Trace._install
      def interrupted(hook, *args):
        self.assertNotEqual(native_install(hook, *args), 0)
        raise KeyboardInterrupt("second install")
      with mock.patch.object(trace.Trace, "_install", interrupted):
        with self.assertRaisesRegex(KeyboardInterrupt, "second install"):
          trace.Trace(other_path, self.shim_path)
      owner.begin_frame(0)
      self.core.fake_write(1, 2)
      owner.end_frame()
      self.assertEqual(len(owner.events()), 1)

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
