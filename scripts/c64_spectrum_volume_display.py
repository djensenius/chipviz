"""Host-only observation contract for the unlinked C64 display fixture.

No emulator adapter is provided: callers must independently establish the
provenance of marker, raster and calibration observations before using this.
"""
from __future__ import annotations

WIDTH, HEIGHT = 320, 200
MAX_DIMENSION = 1024
MAX_PITCH = MAX_DIMENSION * 4 + 4096
READY = {False: b"SVD1", True: b"SVB1"}
MODELS = {"pal": (1, 312), "ntsc": (0, 263), "ntsc-old": (0, 262)}
PEPTO = {0: (0x00, 0x00, 0x00), 4: (0x6f, 0x3d, 0x86),
         5: (0x58, 0x8d, 0x43), 6: (0x35, 0x28, 0x79)}
# Calibration must be a separate, independently generated reference image,
# not the scene or its blank control. These are RGB corner fiducials.
FIDUCIALS = ((0, 0, (255, 0, 0)), (319, 0, (0, 255, 0)),
             (0, 199, (0, 0, 255)), (319, 199, (255, 255, 0)))


def check_frame(frame: tuple[int, int, int, bytes]) -> None:
  width, height, pitch, data = frame
  if not 1 <= width <= MAX_DIMENSION or not 1 <= height <= MAX_DIMENSION:
    raise ValueError("malformed dimensions")
  if pitch < width * 4 or pitch > MAX_PITCH or len(data) != pitch * height:
    raise ValueError("malformed pitch or buffer length")


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
  """One retro_run boundary; callbacks cannot reuse a previous run's frame."""

  def __init__(self) -> None:
    self.active = False
    self.serial = 0
    self.generation = 0
    self.events = []
    self.error = None

  def begin(self):
    if self.active:
      raise ValueError("ambiguous nested run")
    self.active = True
    self.generation += 1
    generation = self.generation
    self.events = []
    self.error = None
    return lambda data, width, height, pitch, pixel_format: self.video(
      generation, data, width, height, pitch, pixel_format)

  def video(self, generation: int, data: bytes | None, width: int, height: int, pitch: int,
            pixel_format: str) -> None:
    if not self.active or generation != self.generation:
      if self.active:
        self.error = "stale callback"
      raise ValueError("stale callback")
    self.serial += 1
    if data is None:
      self.events.append((self.serial, None))
      return
    try:
      if pixel_format != "XRGB8888-LE":
        raise ValueError("unsupported pixel format")
      check_frame((width, height, pitch, data))
    except ValueError as error:
      self.error = str(error)
      raise
    self.events.append((self.serial, (width, height, pitch, bytes(data))))

  def end(self) -> tuple[int, tuple[int, int, int, bytes]]:
    if not self.active:
      raise ValueError("no active run")
    self.active = False
    events, self.events = self.events, []
    if self.error:
      raise ValueError(self.error)
    if not events:
      raise ValueError("missing callback")
    fresh = [event for event in events if event[1] is not None]
    if not fresh:
      raise ValueError("duplicate-only callbacks")
    if len(fresh) > 1:
      raise ValueError("extra fresh frames")
    if len(events) != 1:
      raise ValueError("ambiguous fresh and duplicate callbacks")
    return fresh[0]


def pixel(frame: tuple[int, int, int, bytes], x: int, y: int) -> tuple[int, int, int]:
  width, height, pitch, data = frame
  if not (0 <= x < width and 0 <= y < height):
    raise ValueError("pixel outside frame")
  pos = y * pitch + x * 4
  return data[pos + 2], data[pos + 1], data[pos]


def calibrate(frame: tuple[int, int, int, bytes]) -> tuple[int, int]:
  """Find exactly one set of four known fiducials in a separate frame."""
  check_frame(frame)
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
  return matches[0]


def validate(frame: tuple[int, int, int, bytes], origin: tuple[int, int], blank: bool) -> None:
  check_frame(frame)
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
            calibration: tuple[int, int, int, bytes], limit: int) -> int:
  """Bounded fake-friendly boundary. run() returns the callback events for one run.

  Model flag and raster lines must be measured independently by a future
  adapter; the requested model option is not evidence of either measurement.
  """
  if model not in MODELS or (model_flag, raster_lines) != MODELS[model]:
    raise ValueError("model marker or independent raster geometry mismatch")
  if not 1 <= limit <= 16000:
    raise ValueError("invalid frame limit")
  origin = calibrate(calibration)
  boundary = Observation()
  settled = False
  if read_ready() == READY[blank]:
    raise ValueError("ready marker was already present before boot")
  for count in range(1, limit + 1):
    if read_ready() != READY[blank]:
      if settled:
        raise ValueError("ready marker changed")
      run(None)  # boot only; no observation can be reused
      continue
    callback = boundary.begin()
    run(callback)
    _, frame = boundary.end()
    if read_ready() != READY[blank]:
      raise ValueError("ready marker changed")
    if frame[:2] != calibration[:2]:
      raise ValueError("frame geometry changed after calibration")
    if not settled:
      # The first frame after ready may have begun before publication.
      settled = True
      continue
    validate(frame, origin, blank)
    return count
  raise TimeoutError(f"fixture not ready and settled in {limit} runs")
