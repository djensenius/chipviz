"""Host-only observation contract for the unlinked C64 display fixture.

No emulator adapter is provided: callers must independently establish the
provenance of marker, raster and calibration observations before using this.
"""
from __future__ import annotations

WIDTH, HEIGHT = 320, 200
MAX_DIMENSION = 1024
MAX_PITCH = MAX_DIMENSION * 4 + 4096
READY = {False: b"SVD1", True: b"SVB1"}
CALIBRATION_READY = b"SVC1"
MODELS = {"pal": (1, 312), "ntsc": (0, 263), "ntsc-old": (0, 262)}
# VICE pepto-pal.vpl, pinned source and required libretro options in C64 README.
PALETTE_OPTIONS = {"vice_external_palette": "pepto-pal", "vice_gfx_colors": "24bit"}
PEPTO = {0: (0x00, 0x00, 0x00), 1: (0xff, 0xff, 0xff), 4: (0x6f, 0x3d, 0x86),
         5: (0x58, 0x8d, 0x43), 6: (0x35, 0x28, 0x79)}
# Independently specified corner pixels of the four solid 8x8 cells.
FIDUCIALS = ((0, 0, PEPTO[1]), (319, 0, PEPTO[4]),
             (0, 199, PEPTO[5]), (319, 199, PEPTO[6]))


def check_frame(frame: tuple[int, int, int, bytes]) -> None:
  width, height, pitch, data = frame
  if any(isinstance(value, bool) or not isinstance(value, int) for value in (width, height, pitch)):
    raise ValueError("dimensions and pitch must be integers")
  if not 1 <= width <= MAX_DIMENSION or not 1 <= height <= MAX_DIMENSION:
    raise ValueError("malformed dimensions")
  if pitch < width * 4 or pitch > MAX_PITCH:
    raise ValueError("malformed pitch or buffer length")
  try:
    size = memoryview(data).nbytes
  except (TypeError, ValueError) as error:
    raise ValueError("malformed pitch or buffer length") from error
  if size != pitch * height:
    raise ValueError("malformed pitch or buffer length")


def copy_frame(frame: tuple[int, int, int, bytes]) -> tuple[int, int, int, bytes]:
  """Validate byte size before and after copying a buffer."""
  check_frame(frame)
  width, height, pitch, data = frame
  copied = (width, height, pitch, bytes(memoryview(data)))
  check_frame(copied)
  return copied


def expected_rgb(blank: bool) -> bytes:
  """Explicit pixel rectangles, independent of the model and fixture."""
  image = bytearray(WIDTH * HEIGHT * 3)
  if not blank:
    for x, y, w, h, ink in ((48, 64, 8, 8, 6), (40, 120, 8, 8, 4),
                            (32, 176, 16, 8, 5)):
      for row in range(y, y + h):
        start = (row * WIDTH + x) * 3
        image[start:start + w * 3] = bytes(PEPTO[ink]) * w
  return bytes(image)


class Observation:
  """Sticky-failure boundary; callbacks cannot reuse a previous run's frame."""

  def __init__(self) -> None:
    self.active = False
    self.serial = 0
    self.generation = 0
    self.events = []
    self.error = None

  def latch(self, message: str) -> None:
    if self.error is None:
      self.error = message

  def check(self) -> None:
    if self.error is not None:
      raise ValueError(self.error)

  def begin(self):
    self.check()
    if self.active:
      self.latch("ambiguous nested run")
      self.check()
    self.active = True
    self.generation += 1
    generation = self.generation
    self.events = []
    return lambda data, width, height, pitch, pixel_format: self.video(
      generation, data, width, height, pitch, pixel_format)

  def video(self, generation: int, data: bytes | None, width: int, height: int, pitch: int,
            pixel_format: str) -> None:
    if not self.active or generation != self.generation:
      self.latch("stale callback")
      return
    if self.error is not None:
      return
    self.serial += 1
    if self.events:
      first_fresh = self.events[0][1] is not None
      self.latch("extra fresh frames" if first_fresh and data is not None else
                 "duplicate-only callbacks" if not first_fresh and data is None else
                 "ambiguous fresh and duplicate callbacks")
      return
    if data is None:
      self.events.append((self.serial, None))
    else:
      try:
        if pixel_format != "XRGB8888-LE":
          raise ValueError("unsupported pixel format")
        self.events.append((self.serial, copy_frame((width, height, pitch, data))))
      except (ValueError, TypeError, OverflowError) as error:
        self.latch(str(error) or "frame callback failed")
        return
      except BaseException:
        # Native callback boundaries can swallow even non-Exception failures.
        self.latch("frame buffer conversion failed")
        return

  def end(self) -> tuple[int, tuple[int, int, int, bytes]]:
    if self.error is not None:
      self.active = False
      self.check()
    if not self.active:
      self.latch("no active run")
      self.check()
    self.active = False
    events, self.events = self.events, []
    if not events:
      self.latch("missing callback")
      self.check()
    fresh = [event for event in events if event[1] is not None]
    if not fresh:
      self.latch("duplicate-only callbacks")
      self.check()
    return fresh[0]


def pixel(frame: tuple[int, int, int, bytes], x: int, y: int) -> tuple[int, int, int]:
  width, height, pitch, data = frame
  if not (0 <= x < width and 0 <= y < height):
    raise ValueError("pixel outside frame")
  pos = y * pitch + x * 4
  return data[pos + 2], data[pos + 1], data[pos]


def calibrate(frame: tuple[int, int, int, bytes]) -> tuple[int, int]:
  """Find one corner set and verify the complete independent reference."""
  frame = copy_frame(frame)
  width, height, _, _ = frame
  matches = []
  for y in range(max(0, height - HEIGHT + 1)):
    for x in range(max(0, width - WIDTH + 1)):
      if all(pixel(frame, x + dx, y + dy) == rgb for dx, dy, rgb in FIDUCIALS):
        matches.append((x, y))
        if len(matches) > 1:
          raise ValueError("ambiguous calibration")
  if len(matches) != 1:
    raise ValueError("missing calibration")
  x0, y0 = matches[0]
  for y in range(HEIGHT):
    for x in range(WIDTH):
      if y < 8 and x < 8:
        want = PEPTO[1]
      elif y < 8 and x >= 312:
        want = PEPTO[4]
      elif y >= 192 and x < 8:
        want = PEPTO[5]
      elif y >= 192 and x >= 312:
        want = PEPTO[6]
      else:
        want = PEPTO[0]
      if pixel(frame, x0 + x, y0 + y) != want:
        raise ValueError(f"calibration pixel ({x},{y}) differs")
  return x0, y0


def validate(frame: tuple[int, int, int, bytes], origin: tuple[int, int], blank: bool) -> None:
  frame = copy_frame(frame)
  width, height, _, _ = frame
  x0, y0 = origin
  if x0 < 0 or y0 < 0 or x0 + WIDTH > width or y0 + HEIGHT > height:
    raise ValueError("display window outside frame")
  expected = expected_rgb(blank)
  for y in range(HEIGHT):
    for x in range(WIDTH):
      pos = (y * WIDTH + x) * 3
      got = pixel(frame, x0 + x, y0 + y)
      if got != tuple(expected[pos:pos + 3]):
        raise ValueError(f"pixel ({x},{y}): {got} != {tuple(expected[pos:pos + 3])}")


def observe(run, read_ready, *, blank: bool, model: str, model_flag: int, raster_lines: int,
            calibration: tuple[int, int, int, bytes], limit: int, synthetic_only: bool = False) -> int:
  """Synthetic-only bounded boundary. run() invokes callbacks for one run.

  Model flag and raster lines must be measured independently by a future
  adapter; the requested model option is not evidence of either measurement.
  """
  if synthetic_only is not True:
    raise RuntimeError("trusted real adapter, callback quiescence and palette configuration are not verified")
  if model not in MODELS or (model_flag, raster_lines) != MODELS[model]:
    raise ValueError("model marker or independent raster geometry mismatch")
  if not 1 <= limit <= 16000:
    raise ValueError("invalid frame limit")
  origin = calibrate(calibration)
  boundary = Observation()
  settled = False

  def marker():
    value = read_ready()
    boundary.check()
    return value

  if marker() == READY[blank]:
    raise ValueError("ready marker was already present before boot")
  for count in range(1, limit + 1):
    if marker() != READY[blank]:
      if settled:
        raise ValueError("ready marker changed")
      run(None)  # boot only; no observation can be reused
      boundary.check()
      continue
    callback = boundary.begin()
    run(callback)
    _, frame = boundary.end()
    if marker() != READY[blank]:
      raise ValueError("ready marker changed")
    if frame[:2] != calibration[:2]:
      raise ValueError("frame geometry changed after calibration")
    if not settled:
      # The first frame after ready may have begun before publication.
      settled = True
      continue
    validate(frame, origin, blank)
    boundary.check()
    return count
  boundary.check()
  raise TimeoutError(f"fixture not ready and settled in {limit} runs")
