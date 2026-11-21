"""Synthetic-only tests: no emulator, BIOS, capture, or target execution."""
from array import array
import contextlib
import ctypes
import functools
import io
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import c64_spectrum_volume_display as display  # noqa: E402

W, H, PITCH = 336, 216, 336 * 4 + 8
ORIGIN = (7, 9)
observe = functools.partial(display.observe, synthetic_only=True)


def frame(blank=False):
  data = bytearray(PITCH * H)
  if not blank:
    # Manually specified pixels, not rendered from the oracle.
    for x, y, width, colour in ((48, 64, 8, (53, 40, 121)), (40, 120, 8, (111, 61, 134)),
                                (32, 176, 16, (88, 141, 67))):
      for yy in range(y, y + 8):
        for xx in range(x, x + width):
          put(data, xx + ORIGIN[0], yy + ORIGIN[1], colour)
  return data


def put(data, x, y, rgb, pitch=PITCH):
  pos = y * pitch + x * 4
  data[pos:pos + 4] = bytes((rgb[2], rgb[1], rgb[0], 0))


def draw_calibration(data, origin, pitch):
  # Independent full-cell specification, not read from the fixture/oracle.
  for x, y, rgb in ((0, 0, (255, 255, 255)), (312, 0, (111, 61, 134)),
                    (0, 192, (88, 141, 67)), (312, 192, (53, 40, 121))):
    for yy in range(y, y + 8):
      for xx in range(x, x + 8):
        put(data, xx + origin[0], yy + origin[1], rgb, pitch)


def calibration():
  data = frame(blank=True)
  draw_calibration(data, ORIGIN, PITCH)
  return W, H, PITCH, bytes(data)


def callback_args(data):
  return (bytes(data), W, H, PITCH, "XRGB8888-LE")


def native_callback(callback, *args):
  # ctypes swallows exceptions crossing this callback boundary.
  ctypes.CFUNCTYPE(None)(lambda: callback(*args))()


class FailingBuffer:
  def __init__(self, error):
    self.error = error
    self.calls = 0

  def __buffer__(self, flags):
    self.calls += 1
    raise self.error("synthetic buffer failure")


class ReentrantBuffer:
  def __init__(self, callback, trigger, duplicate=False):
    self.callback = callback
    self.trigger = trigger
    self.duplicate = duplicate
    self.calls = 0
    self.data = bytes(frame())

  def __buffer__(self, flags):
    self.calls += 1
    if self.calls == self.trigger:
      args = (None, 0, 0, 0, "") if self.duplicate else callback_args(self.data)
      native_callback(self.callback, *args)
    return memoryview(self.data)


class DisplayTests(unittest.TestCase):
  def test_reentrant_buffer_callbacks_are_rejected_in_both_observation_frames(self):
    for frame_number in (1, 2):  # settling frame and frame to be validated
      for trigger in (1, 2):  # initial size check and subsequent copy
        for duplicate in (False, True):
          with self.subTest(frame=frame_number, trigger=trigger, duplicate=duplicate):
            markers = iter((b"", b"SVD1", b"SVD1", b"SVD1", b"SVD1"))
            calls = []

            def run(callback):
              calls.append(1)
              data = (ReentrantBuffer(callback, trigger, duplicate) if len(calls) == frame_number
                      else bytes(frame()))
              native_callback(callback, data, W, H, PITCH, "XRGB8888-LE")

            message = "ambiguous" if duplicate else "extra fresh"
            with self.assertRaisesRegex(ValueError, message):
              observe(run, lambda: next(markers), blank=False, model="pal", model_flag=1,
                      raster_lines=312, calibration=calibration(), limit=2)
            self.assertEqual(len(calls), frame_number)

  def test_reentrant_failure_remains_sticky_after_outer_conversion_fails(self):
    for duplicate in (False, True):
      with self.subTest(duplicate=duplicate):
        boundary = display.Observation()
        callback = boundary.begin()
        expected = "ambiguous" if duplicate else "extra fresh"

        class NestedThenFail:
          def __buffer__(self, flags):
            args = (None, 0, 0, 0, "") if duplicate else callback_args(frame())
            native_callback(callback, *args)
            raise MemoryError("outer conversion failed")

        native_callback(callback, NestedThenFail(), W, H, PITCH, "XRGB8888-LE")
        with self.assertRaisesRegex(ValueError, expected):
          boundary.end()
        with self.assertRaisesRegex(ValueError, expected):
          boundary.begin()

  def test_reentrant_stale_callback_during_buffer_conversion_stays_latched(self):
    boundary = display.Observation()
    old = boundary.begin()
    old(*callback_args(frame()))
    boundary.end()
    current = boundary.begin()
    native_callback(current, ReentrantBuffer(old, 1), W, H, PITCH, "XRGB8888-LE")
    with self.assertRaisesRegex(ValueError, "stale"):
      boundary.end()
    with self.assertRaisesRegex(ValueError, "stale"):
      boundary.begin()

  def test_reentrant_format_comparison_rejects_nested_callback(self):
    boundary = display.Observation()
    callback = boundary.begin()
    data = bytes(frame())

    class NestedFormat:
      def __eq__(self, other):
        native_callback(callback, *callback_args(data))
        return True

    native_callback(callback, data, W, H, PITCH, NestedFormat())
    with self.assertRaisesRegex(ValueError, "extra fresh"):
      boundary.end()
    with self.assertRaisesRegex(ValueError, "extra fresh"):
      boundary.begin()

  def test_reentrant_end_cannot_complete_pending_frame(self):
    boundary = display.Observation()
    callback = boundary.begin()
    data = bytes(frame())

    class EndDuringCopy:
      def __buffer__(self, flags):
        native_callback(boundary.end)
        return memoryview(data)

    with contextlib.redirect_stderr(io.StringIO()):
      native_callback(callback, EndDuringCopy(), W, H, PITCH, "XRGB8888-LE")
    with self.assertRaisesRegex(ValueError, "incomplete frame callback"):
      boundary.end()
    with self.assertRaisesRegex(ValueError, "incomplete frame callback"):
      boundary.begin()

  def test_extra_failing_buffer_callbacks_are_rejected(self):
    for error in (BufferError, MemoryError):
      with self.subTest(error=error.__name__):
        broken = FailingBuffer(error)
        markers = iter((b"", b"SVD1", b"SVD1", b"SVD1", b"SVD1"))
        calls = []

        def run(callback):
          calls.append(1)
          callback(*callback_args(frame()))
          native_callback(callback, broken, W, H, PITCH, "XRGB8888-LE")

        with contextlib.redirect_stderr(io.StringIO()), self.assertRaisesRegex(ValueError, "extra fresh"):
          observe(run, lambda: next(markers), blank=False, model="pal", model_flag=1,
                  raster_lines=312, calibration=calibration(), limit=3)
        self.assertEqual(len(calls), 1)
        self.assertEqual(broken.calls, 0)

  def test_first_buffer_and_copy_failures_are_latched(self):
    for error in (BufferError, MemoryError, RuntimeError, KeyboardInterrupt, SystemExit):
      with self.subTest(stage="export", error=error.__name__):
        broken = FailingBuffer(error)
        boundary = display.Observation()
        callback = boundary.begin()
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
          native_callback(callback, broken, W, H, PITCH, "XRGB8888-LE")
        callback(*callback_args(frame()))
        with self.assertRaisesRegex(ValueError, "buffer|synthetic"):
          boundary.end()
        with self.assertRaisesRegex(ValueError, "buffer|synthetic"):
          boundary.begin()
        self.assertEqual(broken.calls, 1)
        self.assertEqual(stderr.getvalue(), "")
    for error in (BufferError, MemoryError):
      with self.subTest(stage="copy", error=error.__name__):
        data = bytes(frame())
        boundary = display.Observation()
        callback = boundary.begin()
        stderr = io.StringIO()
        with mock.patch.object(display, "bytes", side_effect=error("synthetic copy failure"), create=True), \
            contextlib.redirect_stderr(stderr):
          native_callback(callback, data, W, H, PITCH, "XRGB8888-LE")
        callback(*callback_args(frame()))
        with self.assertRaisesRegex(ValueError, "frame buffer conversion failed"):
          boundary.end()
        with self.assertRaisesRegex(ValueError, "frame buffer conversion failed"):
          boundary.begin()
        self.assertEqual(stderr.getvalue(), "")

  def test_duplicate_then_failing_fresh_callback_is_ambiguous_without_conversion(self):
    for error in (BufferError, MemoryError):
      with self.subTest(error=error.__name__):
        broken = FailingBuffer(error)
        boundary = display.Observation()
        callback = boundary.begin()
        callback(None, 0, 0, 0, "")
        native_callback(callback, broken, W, H, PITCH, "XRGB8888-LE")
        with self.assertRaisesRegex(ValueError, "ambiguous"):
          boundary.end()
        self.assertEqual(broken.calls, 0)

  def test_settling_frame_rejects_wide_memoryview_with_wrong_byte_count(self):
    bad = memoryview(array("H", [0]) * (PITCH * H))
    self.assertEqual(len(bad), PITCH * H)
    self.assertEqual(bad.nbytes, 2 * PITCH * H)
    markers = iter((b"", b"SVD1", b"SVD1", b"SVD1", b"SVD1"))
    calls = []

    def run(callback):
      calls.append(1)
      data = bad if len(calls) == 1 else bytes(frame())
      native_callback(callback, data, W, H, PITCH, "XRGB8888-LE")

    with self.assertRaisesRegex(ValueError, "buffer length"):
      observe(run, lambda: next(markers), blank=False, model="pal", model_flag=1,
              raster_lines=312, calibration=calibration(), limit=3)
    self.assertEqual(len(calls), 1)

  def test_wide_memoryview_wrong_byte_count_rejected_everywhere(self):
    bad = memoryview(array("H", [0]) * (PITCH * H))
    bad_frame = (W, H, PITCH, bad)
    for action in (lambda: display.check_frame(bad_frame), lambda: display.copy_frame(bad_frame),
                   lambda: display.calibrate(bad_frame), lambda: display.validate(bad_frame, ORIGIN, False)):
      with self.subTest(action=action), self.assertRaisesRegex(ValueError, "buffer length"):
        action()
    boundary = display.Observation()
    callback = boundary.begin()
    native_callback(callback, bad, W, H, PITCH, "XRGB8888-LE")
    callback(*callback_args(frame()))
    with self.assertRaisesRegex(ValueError, "buffer length"):
      boundary.end()
    with self.assertRaisesRegex(ValueError, "buffer length"):
      boundary.begin()

  def test_valid_byte_buffers_are_copied_and_interpreted_as_bytes(self):
    scene = bytes(frame())
    reference = calibration()[3]
    for data in (scene, bytearray(scene), memoryview(scene), memoryview(bytearray(scene)).cast("H")):
      with self.subTest(format=type(data).__name__):
        boundary = display.Observation()
        callback = boundary.begin()
        callback(data, W, H, PITCH, "XRGB8888-LE")
        _, copied = boundary.end()
        self.assertIsInstance(copied[3], bytes)
        self.assertEqual(copied[3], scene)
        display.validate(copied, ORIGIN, False)
    for data in (reference, bytearray(reference), memoryview(reference),
                 memoryview(bytearray(reference)).cast("H")):
      with self.subTest(calibration=type(data).__name__):
        self.assertEqual(display.calibrate((W, H, PITCH, data)), ORIGIN)
    mutable = bytearray(scene)
    boundary = display.Observation()
    callback = boundary.begin()
    callback(memoryview(mutable), W, H, PITCH, "XRGB8888-LE")
    mutable[:] = bytes(len(mutable))
    _, copied = boundary.end()
    self.assertEqual(copied[3], scene)
    markers = iter((b"", b"SVD1", b"SVD1", b"SVD1", b"SVD1"))
    wide_scene = memoryview(bytearray(scene)).cast("H")
    wide_reference = memoryview(bytearray(reference)).cast("H")
    self.assertEqual(observe(lambda cb: cb(wide_scene, W, H, PITCH, "XRGB8888-LE"),
                             lambda: next(markers), blank=False, model="pal", model_flag=1,
                             raster_lines=312, calibration=(W, H, PITCH, wide_reference), limit=2), 2)

  def test_frame_geometry_requires_integers_not_bool(self):
    source = calibration()
    for index in range(3):
      for value in (float(source[index]), 1.5, True, False):
        broken = list(source)
        broken[index] = value
        with self.subTest(field=index, value=value):
          with self.assertRaisesRegex(ValueError, "integers"):
            display.check_frame(tuple(broken))
          with self.assertRaisesRegex(ValueError, "integers"):
            display.calibrate(tuple(broken))
          with self.assertRaisesRegex(ValueError, "integers"):
            display.validate(tuple(broken), ORIGIN, False)
          boundary = display.Observation()
          callback = boundary.begin()
          native_callback(callback, broken[3], broken[0], broken[1], broken[2], "XRGB8888-LE")
          callback(*callback_args(frame()))
          with self.assertRaisesRegex(ValueError, "integers"):
            boundary.end()
          with self.assertRaisesRegex(ValueError, "integers"):
            boundary.begin()

  def test_settling_frame_rejects_integral_float_geometry(self):
    for field in range(3):
      markers = iter((b"", b"SVD1", b"SVD1", b"SVD1", b"SVD1"))
      calls = []

      def run(callback):
        calls.append(1)
        args = list(callback_args(frame()))
        if len(calls) == 1:
          args[field + 1] = float(args[field + 1])
        native_callback(callback, *args)

      with self.subTest(field=field):
        with self.assertRaisesRegex(ValueError, "integers"):
          observe(run, lambda: next(markers), blank=False, model="pal", model_flag=1,
                  raster_lines=312, calibration=calibration(), limit=3)
        self.assertEqual(len(calls), 1)

  def test_real_observation_is_blocked_without_verified_adapter(self):
    with self.assertRaisesRegex(RuntimeError, "trusted real adapter"):
      display.observe(None, None, blank=False, model="pal", model_flag=1, raster_lines=312,
                      calibration=calibration(), limit=2)

  def test_calibration_fixture_host_surfaces(self):
    with tempfile.TemporaryDirectory() as directory:
      library = pathlib.Path(directory) / "calibration.so"
      subprocess.run([os.environ.get("CC", "cc"), "-std=c99", "-Wall", "-Wextra", "-Werror",
                      "-pedantic", "-shared", "-fPIC", "-DSV_CALIBRATION_HOST_TEST",
                      str(ROOT / "tests/c64_spectrum_volume_calibration.c"), "-o", str(library)],
                     check=True, capture_output=True)
      fixture = ctypes.CDLL(str(library))
      ptr = ctypes.POINTER(ctypes.c_uint8)
      fixture.sv_calibration_prepare.argtypes = [ptr, ptr]
      screen = (ctypes.c_uint8 * 1032)(*([0xa5] * 1032))
      colour = (ctypes.c_uint8 * 1032)(*([0x5a] * 1032))
      fixture.sv_calibration_prepare(ctypes.cast(ctypes.byref(screen, 16), ptr),
                                     ctypes.cast(ctypes.byref(colour, 16), ptr))
      want_screen = bytearray([32] * 1000)
      want_colour = bytearray(1000)
      for pos, ink in ((0, 1), (39, 4), (960, 5), (999, 6)):
        want_screen[pos] = 160
        want_colour[pos] = ink
      self.assertEqual(bytes(screen), bytes([0xa5] * 16) + want_screen + bytes([0xa5] * 16))
      self.assertEqual(bytes(colour), bytes([0x5a] * 16) + want_colour + bytes([0x5a] * 16))
      rgb = {0: (0, 0, 0), 1: (255, 255, 255), 4: (111, 61, 134),
             5: (88, 141, 67), 6: (53, 40, 121)}
      fake = frame(blank=True)
      for y in range(200):
        for x in range(320):
          index = 16 + (y // 8) * 40 + x // 8
          ink = colour[index] if screen[index] == 160 else 0
          put(fake, x + ORIGIN[0], y + ORIGIN[1], rgb[ink])
      self.assertEqual(bytes(fake), calibration()[3])
      self.assertEqual(display.calibrate((W, H, PITCH, bytes(fake))), ORIGIN)

  def test_palette_options_and_pinned_rgb(self):
    self.assertEqual(display.CALIBRATION_READY, b"SVC1")
    self.assertEqual(display.PALETTE_OPTIONS, {"vice_external_palette": "pepto-pal", "vice_gfx_colors": "24bit"})
    self.assertEqual(display.PEPTO, {0: (0, 0, 0), 1: (255, 255, 255), 4: (111, 61, 134),
                                     5: (88, 141, 67), 6: (53, 40, 121)})

  def test_callback_faults_are_permanent_even_when_callback_exception_is_swallowed(self):
    for kind in ("stale", "invalid", "extra", "duplicate-only", "ambiguous"):
      with self.subTest(kind=kind):
        boundary = display.Observation()
        callback = boundary.begin()
        if kind == "stale":
          callback(*callback_args(frame()))
          boundary.end()
          native_callback(callback, *callback_args(frame()))
        else:
          if kind == "invalid":
            events = [(bytes(4), 1, 1, 4, "RGB565"), callback_args(frame())]
          elif kind == "extra":
            events = [callback_args(frame())] * 2
          elif kind == "duplicate-only":
            events = [(None, 0, 0, 0, "")]
          else:
            events = [callback_args(frame()), (None, 0, 0, 0, "")]
          for event in events:
            native_callback(callback, *event)
          with self.assertRaises(ValueError):
            boundary.end()
        with self.assertRaises(ValueError):
          boundary.begin()
        with self.assertRaises(ValueError):
          boundary.end()

  def test_observe_rejects_stale_callback_from_ready_reads(self):
    for trigger in (3, 5):  # after the discarded frame; after the final frame
      with self.subTest(trigger=trigger):
        saved = []
        reads = []

        def run(callback):
          saved.append(callback)
          callback(*callback_args(frame()))

        def ready():
          reads.append(1)
          if len(reads) == trigger:
            native_callback(saved[-1], *callback_args(frame()))
          return b"" if len(reads) == 1 else b"SVD1"

        with self.assertRaisesRegex(ValueError, "stale"):
          observe(run, ready, blank=False, model="pal", model_flag=1, raster_lines=312,
                  calibration=calibration(), limit=3)

  def test_observe_rejects_swallowed_invalid_and_duplicate_events(self):
    bad_format = (bytes(4), 1, 1, 4, "RGB565")
    duplicate = (None, 0, 0, 0, "")
    fresh = callback_args(frame())
    for events, message in (([bad_format, fresh], "unsupported"),
                            ([duplicate], "duplicate-only"),
                            ([fresh, fresh], "extra"),
                            ([fresh, duplicate], "ambiguous")):
      with self.subTest(message=message):
        calls = []
        markers = iter((b"", b"SVD1"))

        def run(callback):
          calls.append(1)
          for event in events:
            native_callback(callback, *event)

        with self.assertRaisesRegex(ValueError, message):
          observe(run, lambda: next(markers), blank=False, model="pal", model_flag=1,
                  raster_lines=312, calibration=calibration(), limit=3)
        self.assertEqual(len(calls), 1)

  def test_host_model_complete_surface_against_pixel_oracle(self):
    class State(ctypes.Structure):
      _fields_ = [("height", ctypes.c_uint8 * 8), ("palette", ctypes.c_uint8 * 8),
                  ("next_palette", ctypes.c_uint8)]

    source = ROOT / "cores/c64/homebrew/src"
    with tempfile.TemporaryDirectory() as directory:
      library = pathlib.Path(directory) / "model.so"
      subprocess.run([os.environ.get("CC", "cc"), "-std=c99", "-Wall", "-Wextra", "-Werror",
                      "-pedantic", "-shared", "-fPIC", str(source / "spectrum_volume.c"),
                      "-o", str(library)], check=True, capture_output=True)
      model = ctypes.CDLL(str(library))
      ptr = ctypes.POINTER(ctypes.c_uint8)
      model.sv_init.argtypes = [ctypes.POINTER(State), ctypes.c_uint8]
      model.sv_step.argtypes = [ctypes.POINTER(State), ptr, ctypes.c_uint8, ctypes.c_uint8, ptr, ptr]
      screen = (ctypes.c_uint8 * 1000)(*([32] * 1000))
      colour = (ctypes.c_uint8 * 1000)()
      state = State()
      model.sv_init(ctypes.byref(state), 0)
      self.assertEqual(bytes(screen), bytes([32]) * 1000)
      bands = (ctypes.c_uint8 * 8)(255, 0, 0, 0, 0, 0, 0, 0)
      model.sv_step(ctypes.byref(state), bands, 255, 0, screen, colour)
      # Interpret only the two assumed ROM glyphs; compare every pixel.
      pixels = bytearray()
      for y in range(200):
        for x in range(320):
          index = (y // 8) * 40 + x // 8
          self.assertIn(screen[index], (32, 160))
          ink = colour[index] if screen[index] == 160 else 0
          pixels.extend(display.PEPTO[ink])
      self.assertEqual(bytes(pixels), display.expected_rgb(False))

  def test_oracle_and_full_frame_failures(self):
    self.assertEqual(display.calibrate(calibration()), ORIGIN)
    for blank in (False, True):
      display.validate((W, H, PITCH, bytes(frame(blank))), ORIGIN, blank)
      expected = display.expected_rgb(blank)
      self.assertEqual(len(expected), 320 * 200 * 3)
      self.assertEqual(sum(expected[i:i + 3] != b"\0\0\0" for i in range(0, len(expected), 3)),
                       0 if blank else 256)
    for name in ("shifted", "missing", "extra", "wrong-colour", "blank"):
      data = frame(blank=name == "blank")
      if name == "shifted":
        data = bytearray(PITCH * H)
        original = frame()
        for y in range(H):
          data[y * PITCH + 4:y * PITCH + W * 4] = original[y * PITCH:y * PITCH + (W - 1) * 4]
      elif name == "missing":
        put(data, 48 + ORIGIN[0], 64 + ORIGIN[1], (0, 0, 0))
      elif name == "extra":
        put(data, ORIGIN[0], ORIGIN[1], (53, 40, 121))
      elif name == "wrong-colour":
        put(data, 48 + ORIGIN[0], 64 + ORIGIN[1], (111, 61, 134))
      with self.subTest(name=name), self.assertRaisesRegex(ValueError, "pixel"):
        display.validate((W, H, PITCH, bytes(data)), ORIGIN, False)
    with self.assertRaisesRegex(ValueError, "pixel"):
      display.validate((W, H, PITCH, bytes(frame())), ORIGIN, True)

  def test_calibration_rejects_missing_and_ambiguous(self):
    with self.assertRaisesRegex(ValueError, "missing calibration"):
      display.calibrate((W, H, PITCH, bytes(frame(True))))
    width, height, pitch = 640, 200, 640 * 4
    data = bytearray(pitch * height)
    for x0 in (0, 320):
      draw_calibration(data, (x0, 0), pitch)
    with self.assertRaisesRegex(ValueError, "ambiguous calibration"):
      display.calibrate((width, height, pitch, bytes(data)))
    for x, y, rgb in ((1, 1, (0, 0, 0)), (20, 20, (255, 255, 255)),
                      (318, 1, (88, 141, 67))):
      data = bytearray(calibration()[3])
      put(data, x + ORIGIN[0], y + ORIGIN[1], rgb)
      with self.subTest(x=x, y=y), self.assertRaisesRegex(ValueError, "calibration pixel"):
        display.calibrate((W, H, PITCH, bytes(data)))
    data = bytearray(calibration()[3])
    put(data, ORIGIN[0], ORIGIN[1], (0, 0, 0))
    with self.assertRaisesRegex(ValueError, "missing calibration"):
      display.calibrate((W, H, PITCH, bytes(data)))

  def test_callback_identity_and_malformed_frames(self):
    boundary = display.Observation()
    old = boundary.begin()
    old(*callback_args(frame()))
    first, _ = boundary.end()
    current = boundary.begin()
    current(*callback_args(frame()))
    second, _ = boundary.end()
    self.assertGreater(second, first)
    native_callback(old, *callback_args(frame()))
    with self.assertRaisesRegex(ValueError, "stale"):
      boundary.begin()
    with self.assertRaisesRegex(ValueError, "stale"):
      boundary.end()
    boundary = display.Observation()
    old = boundary.begin()
    old(*callback_args(frame()))
    boundary.end()
    current = boundary.begin()
    native_callback(old, *callback_args(frame()))
    current(*callback_args(frame()))
    with self.assertRaisesRegex(ValueError, "stale"):
      boundary.end()
    with self.assertRaisesRegex(ValueError, "stale"):
      boundary.begin()
    for events, message in (([], "missing"), ([(None, 0, 0, 0, "")], "duplicate-only"),
                            ([callback_args(frame())] * 2, "extra"),
                            ([callback_args(frame()), (None, 0, 0, 0, "")], "ambiguous")):
      boundary = display.Observation()
      callback = boundary.begin()
      for event in events:
        callback(*event)
      with self.assertRaisesRegex(ValueError, message):
        boundary.end()
    for args, message in (((bytes(4), 1, 1, 4, "RGB565"), "unsupported"),
                          ((bytes(4), 0, 1, 4, "XRGB8888-LE"), "dimensions"),
                          ((bytes(4), 1, 1, 3, "XRGB8888-LE"), "pitch"),
                          ((bytes(3), 1, 1, 4, "XRGB8888-LE"), "pitch")):
      boundary = display.Observation()
      callback = boundary.begin()
      native_callback(callback, *args)
      callback(*callback_args(frame()))
      with self.assertRaisesRegex(ValueError, message):
        boundary.end()

  def test_bounded_observation_and_model_evidence(self):
    for model, (flag, lines) in display.MODELS.items():
      for blank in (False, True):
        calls = []
        marker = [b""]

        def run(callback):
          calls.append(callback is not None)
          if callback:
            callback(*callback_args(frame(blank)))
          else:
            marker[0] = display.READY[blank]

        ready = lambda: marker[0]
        self.assertEqual(observe(run, ready, blank=blank, model=model, model_flag=flag,
                                 raster_lines=lines, calibration=calibration(), limit=3), 3)
        self.assertEqual(calls, [False, True, True])
        for bad_flag, bad_lines in ((1 - flag, lines), (flag, lines + 1)):
          with self.assertRaisesRegex(ValueError, "model marker"):
            observe(run, ready, blank=blank, model=model, model_flag=bad_flag,
                    raster_lines=bad_lines, calibration=calibration(), limit=2)
    with self.assertRaisesRegex(TimeoutError, "not ready"):
      observe(lambda cb: None, lambda: b"", blank=False, model="pal", model_flag=1,
              raster_lines=312, calibration=calibration(), limit=3)
    markers = iter((b"", b"SVD1", b"SVD1"))
    with self.assertRaisesRegex(TimeoutError, "settled"):
      observe(lambda cb: cb(*callback_args(frame())), lambda: next(markers), blank=False,
              model="pal", model_flag=1, raster_lines=312, calibration=calibration(), limit=1)
    markers = iter((b"", b"SVD1", b"bad!"))
    with self.assertRaisesRegex(ValueError, "marker changed"):
      observe(lambda cb: cb(*callback_args(frame())), lambda: next(markers), blank=False,
              model="pal", model_flag=1, raster_lines=312, calibration=calibration(), limit=2)
    markers = iter((b"", b"SVD1"))
    with self.assertRaisesRegex(ValueError, "missing callback"):
      observe(lambda cb: None, lambda: next(markers), blank=False, model="pal", model_flag=1,
              raster_lines=312, calibration=calibration(), limit=2)
    with self.assertRaisesRegex(ValueError, "already present"):
      observe(lambda cb: None, lambda: b"SVD1", blank=False, model="pal", model_flag=1,
              raster_lines=312, calibration=calibration(), limit=2)

  def test_observation_rejects_bad_frame_within_budget(self):
    for kind in ("shifted", "missing", "extra", "wrong-colour", "blank"):
      data = frame(blank=kind == "blank")
      if kind == "shifted":
        original = frame()
        data = bytearray(PITCH * H)
        for y in range(H):
          data[y * PITCH + 4:y * PITCH + W * 4] = original[y * PITCH:y * PITCH + (W - 1) * 4]
      elif kind == "missing":
        put(data, 55, 73, (0, 0, 0))
      elif kind == "extra":
        put(data, 7, 9, (53, 40, 121))
      elif kind == "wrong-colour":
        put(data, 55, 73, (111, 61, 134))
      calls = []
      markers = iter((b"", b"SVD1", b"SVD1", b"SVD1", b"SVD1"))

      def run(callback):
        calls.append(1)
        callback(*callback_args(data))

      with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, "pixel"):
        observe(run, lambda: next(markers), blank=False, model="pal", model_flag=1,
                raster_lines=312, calibration=calibration(), limit=10)
      self.assertEqual(len(calls), 2)


if __name__ == "__main__":
  unittest.main()
