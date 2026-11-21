"""Fail-closed, host-only management of the uncalibrated SMS port trace."""

import ctypes
from dataclasses import dataclass
import hashlib
from pathlib import Path


GX_HASH = "27bc2ef8ca07c775f022ae90be381b2a2a500245c9c7e362356ba3077b25c268"
RATIO = 1 << 20


@dataclass(frozen=True)
class Layout:
  size: int
  cycle_offset: int
  read_symbol: str = "z80_ms_port_r"
  write_symbol: str = "z80_ms_port_w"


# No GX layout is registered: the Z80 symbol's actual extent is not yet vetted.
# Tests register their own compiled fake's known layout temporarily.
_VETTED_LAYOUTS = {}
# Retain both libraries and owner tokens until native ownership is released.
_RETAINED = {}


def retained_traces():
  """Return retained owners, including active traces and failed constructions."""
  return tuple(_RETAINED.values())


def _layout_for_hash(digest):
  layout = _VETTED_LAYOUTS.get(digest)
  if not isinstance(layout, Layout):
    raise RuntimeError("core hash has no vetted layout")
  size, offset = layout.size, layout.cycle_offset
  width = ctypes.sizeof(ctypes.c_uint32)
  if (type(size) is not int or type(offset) is not int or size < width or
      offset < 0 or offset % ctypes.alignment(ctypes.c_uint32) or offset > size - width):
    raise RuntimeError("invalid core layout")
  return layout


class Event(ctypes.Structure):
  _fields_ = [(name, ctypes.c_uint32) for name in ("frame", "cycle", "port", "value", "kind")]


class Trace:
  """Install only after core initialization; run and inspect on one thread."""

  def __init__(self, core_path, shim_path, *, capacity=4096):
    core_path = Path(core_path).resolve()
    layout = _layout_for_hash(hashlib.sha256(core_path.read_bytes()).hexdigest())
    self.core = ctypes.CDLL(str(core_path))
    self.shim = ctypes.CDLL(str(Path(shim_path).resolve()))
    try:
      self.read_slot = ctypes.c_void_p.in_dll(self.core, "z80_readport")
      self.write_slot = ctypes.c_void_p.in_dll(self.core, "z80_writeport")
      self.ratio = ctypes.c_uint32.in_dll(self.core, "z80_cycle_ratio")
      z80 = (ctypes.c_uint8 * layout.size).in_dll(self.core, "Z80")
      read = ctypes.cast(getattr(self.core, layout.read_symbol), ctypes.c_void_p).value
      write = ctypes.cast(getattr(self.core, layout.write_symbol), ctypes.c_void_p).value
    except (ValueError, AttributeError) as error:
      raise RuntimeError("missing core symbol") from error
    if not read or not write or self.read_slot.value != read or self.write_slot.value != write:
      raise RuntimeError("unexpected port pointer identity")
    if self.ratio.value != RATIO:
      raise RuntimeError("cycle ratio mismatch")
    if ctypes.addressof(z80) % ctypes.alignment(ctypes.c_uint32):
      raise RuntimeError("unaligned core layout")
    self.z80 = z80
    self.cycles = ctypes.c_uint32.from_buffer(z80, layout.cycle_offset)
    self._bind()
    self._owner = ctypes.c_byte()
    self._keys = (ctypes.byref(self._owner), ctypes.byref(self.read_slot), ctypes.byref(self.write_slot))
    self.active = False
    self.in_frame = False
    self._invalid = False
    self.last_frame = -1
    _RETAINED[id(self)] = self
    self.active = True
    try:
      if self._install(read, write, capacity):
        raise RuntimeError("trace installation failed")
    except BaseException as error:
      if not self._recover(error):
        try:
          error.trace_owner = self
        except BaseException:
          pass
        BaseException.add_note(error, "trace retained: retry trace_owner.retry_cleanup() or inspect retained_traces()")
      raise

  def _bind(self):
    shim = self.shim
    shim.sms_trace_install.argtypes = [ctypes.c_void_p] * 6 + [ctypes.c_size_t, ctypes.c_void_p]
    shim.sms_trace_install.restype = ctypes.c_int
    shim.sms_trace_begin.argtypes = [ctypes.c_uint32]
    shim.sms_trace_begin.restype = ctypes.c_int
    shim.sms_trace_end.argtypes = []
    shim.sms_trace_end.restype = ctypes.c_int
    shim.sms_trace_count.argtypes = []
    shim.sms_trace_count.restype = ctypes.c_size_t
    shim.sms_trace_lost.argtypes = []
    shim.sms_trace_lost.restype = ctypes.c_int
    shim.sms_trace_healthy.argtypes = []
    shim.sms_trace_healthy.restype = ctypes.c_int
    shim.sms_trace_event.argtypes = [ctypes.c_size_t, ctypes.POINTER(Event)]
    shim.sms_trace_event.restype = ctypes.c_int
    shim.sms_trace_owned.argtypes = [ctypes.c_void_p] * 3
    shim.sms_trace_owned.restype = ctypes.c_int
    shim.sms_trace_uninstall.argtypes = [ctypes.c_void_p] * 3
    shim.sms_trace_uninstall.restype = ctypes.c_int
    self._native_uninstall = shim.sms_trace_uninstall

  def _install(self, read, write, capacity):
    return self.shim.sms_trace_install(ctypes.byref(self.read_slot), ctypes.byref(self.write_slot),
                                       read, write, ctypes.byref(self.cycles),
                                       ctypes.byref(self.ratio), capacity, self._keys[0])

  def _uninstall(self):
    return self._native_uninstall(*self._keys)

  def retry_cleanup(self):
    """Release only this token's native ownership; retry after interruption."""
    status = 0
    if self.shim.sms_trace_owned(*self._keys):
      status = self._native_uninstall(*self._keys)
    self.active = False
    _RETAINED.pop(id(self), None)
    if status:
      raise RuntimeError("port pointer changed while tracing")

  def _recover(self, error):
    try:
      self.retry_cleanup()
      return True
    except BaseException as cleanup_error:
      try:
        BaseException.add_note(error, f"trace cleanup failed: {cleanup_error!r}")
      except BaseException:
        pass
      return id(self) not in _RETAINED

  def begin_frame(self, frame):
    if (not self.active or self.in_frame or self._invalid or not 0 <= frame <= 0xffffffff or
        (self.last_frame >= 0 and frame != self.last_frame + 1)):
      raise RuntimeError("invalid frame boundary")
    self._invalid = True
    if self.shim.sms_trace_begin(frame):
      raise RuntimeError("cannot begin frame (trace loss, pointer or cycle ratio mismatch)")
    self.in_frame = True
    self.last_frame = frame
    self._invalid = False

  def end_frame(self):
    if not self.active or not self.in_frame or self._invalid:
      raise RuntimeError("no active frame")
    self._invalid = True
    if self.shim.sms_trace_end():
      raise RuntimeError("trace loss, pointer or cycle ratio mismatch")
    self.in_frame = False
    self._invalid = False

  def events(self):
    if not self.active:
      raise RuntimeError("trace unavailable during frame or after close")
    if not self.shim.sms_trace_healthy():
      raise RuntimeError("trace loss, pointer or cycle ratio mismatch")
    if self._invalid:
      raise RuntimeError("trace loss or interrupted frame transition")
    if self.in_frame:
      raise RuntimeError("trace unavailable during frame or after close")
    result = []
    for index in range(self.shim.sms_trace_count()):
      event = Event()
      if self.shim.sms_trace_event(index, ctypes.byref(event)):
        raise RuntimeError("missing trace event")
      result.append(event)
    for previous, current in zip(result, result[1:]):
      if current.frame < previous.frame or (current.frame == previous.frame and current.cycle < previous.cycle):
        raise RuntimeError("counter decreased within frame")
    return result

  def close(self):
    if self.active:
      try:
        if not self.shim.sms_trace_owned(*self._keys):
          self.retry_cleanup()
          return
        healthy = self.shim.sms_trace_healthy()
        incomplete = self.in_frame or self._invalid
        if self._uninstall():
          raise RuntimeError("port pointer changed while tracing")
        self.active = False
        _RETAINED.pop(id(self), None)
        if not healthy or incomplete:
          raise RuntimeError("trace loss, pointer or cycle ratio mismatch, unfinished frame or interrupted transition")
      except BaseException as error:
        self._recover(error)
        raise

  def __enter__(self):
    return self

  def __exit__(self, exc_type, exc, traceback):
    try:
      self.close()
    except BaseException as error:
      if exc is None:
        raise
      BaseException.add_note(exc, f"trace cleanup: {error}")
      for note in getattr(error, "__notes__", ()):
        BaseException.add_note(exc, note)


def check_seed(events):
  """Check the complete seed write sequence, not calibration or bus timing."""
  expected = [(0xbf, value) for value in (4, 0x80, 0xe0, 0x81, 0xff, 0x82, 0, 0xc0)]
  expected += [(0xbe, value) for value in (0x3f, 3, 0x0c, 0x30)]
  if [(event.port & 255, event.value, event.kind) for event in events] != [
      (port, value, 1) for port, value in expected]:
    raise RuntimeError("missing, duplicate or unexpected seed port event")
  for previous, current in zip(events, events[1:]):
    if current.frame != previous.frame or current.cycle - previous.cycle != 270:
      raise RuntimeError("seed spacing not observed within one frame")
