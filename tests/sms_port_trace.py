"""Fail-closed, host-only management of the uncalibrated SMS port trace."""

import ctypes
from dataclasses import dataclass
import hashlib
from pathlib import Path


GX_HASH = "27bc2ef8ca07c775f022ae90be381b2a2a500245c9c7e362356ba3077b25c268"
RATIO = 1 << 20


@dataclass(frozen=True)
class Profile:
  # Alternative profiles are for fake-core tests, not evidence about GX.
  sha256: str
  cycle_offset: int = 0x40
  read_symbol: str = "z80_ms_port_r"
  write_symbol: str = "z80_ms_port_w"


GX = Profile(GX_HASH)


class Event(ctypes.Structure):
  _fields_ = [(name, ctypes.c_uint32) for name in ("frame", "cycle", "port", "value", "kind")]


class Trace:
  """Install only after core initialization; run and inspect on one thread."""

  def __init__(self, core_path, shim_path, *, profile=GX, capacity=4096):
    core_path = Path(core_path).resolve()
    if hashlib.sha256(core_path.read_bytes()).hexdigest() != profile.sha256:
      raise RuntimeError("core hash mismatch")
    self.core = ctypes.CDLL(str(core_path))
    self.shim = ctypes.CDLL(str(Path(shim_path).resolve()))
    try:
      self.read_slot = ctypes.c_void_p.in_dll(self.core, "z80_readport")
      self.write_slot = ctypes.c_void_p.in_dll(self.core, "z80_writeport")
      self.ratio = ctypes.c_uint32.in_dll(self.core, "z80_cycle_ratio")
      z80 = ctypes.c_uint8.in_dll(self.core, "Z80")
      read = ctypes.cast(getattr(self.core, profile.read_symbol), ctypes.c_void_p).value
      write = ctypes.cast(getattr(self.core, profile.write_symbol), ctypes.c_void_p).value
    except (ValueError, AttributeError) as error:
      raise RuntimeError("missing core symbol") from error
    if not read or not write or self.read_slot.value != read or self.write_slot.value != write:
      raise RuntimeError("unexpected port pointer identity")
    if self.ratio.value != RATIO:
      raise RuntimeError("cycle ratio mismatch")
    self.cycles = ctypes.c_uint32.from_address(ctypes.addressof(z80) + profile.cycle_offset)
    self._bind()
    self.active = False
    self.in_frame = False
    self.last_frame = -1
    if self.shim.sms_trace_install(ctypes.byref(self.read_slot), ctypes.byref(self.write_slot),
                                   read, write, ctypes.byref(self.cycles),
                                   ctypes.byref(self.ratio), capacity):
      raise RuntimeError("trace installation failed")
    self.active = True

  def _bind(self):
    shim = self.shim
    shim.sms_trace_install.argtypes = [ctypes.c_void_p] * 6 + [ctypes.c_size_t]
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
    shim.sms_trace_uninstall.argtypes = []
    shim.sms_trace_uninstall.restype = ctypes.c_int

  def begin_frame(self, frame):
    if (not self.active or self.in_frame or not 0 <= frame <= 0xffffffff or
        (self.last_frame >= 0 and frame != self.last_frame + 1)):
      raise RuntimeError("invalid frame boundary")
    if self.shim.sms_trace_begin(frame):
      raise RuntimeError("cannot begin frame (trace loss, pointer or cycle ratio mismatch)")
    self.in_frame = True
    self.last_frame = frame

  def end_frame(self):
    if not self.active or not self.in_frame:
      raise RuntimeError("no active frame")
    self.in_frame = False
    if self.shim.sms_trace_end():
      raise RuntimeError("trace loss, pointer or cycle ratio mismatch")

  def events(self):
    if not self.active or self.in_frame:
      raise RuntimeError("trace unavailable during frame or after close")
    if not self.shim.sms_trace_healthy():
      raise RuntimeError("trace loss, pointer or cycle ratio mismatch")
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
      healthy = self.shim.sms_trace_healthy()
      incomplete = self.in_frame
      self.active = False
      if self.shim.sms_trace_uninstall():
        raise RuntimeError("port pointer changed while tracing")
      if not healthy or incomplete:
        raise RuntimeError("trace loss, pointer or cycle ratio mismatch, or unfinished frame")

  def __enter__(self):
    return self

  def __exit__(self, exc_type, exc, traceback):
    try:
      self.close()
    except RuntimeError as error:
      if exc is None:
        raise
      exc.add_note(f"trace cleanup: {error}")


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
