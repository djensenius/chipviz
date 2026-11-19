#!/usr/bin/env python3
"""Raspberry Pi/development host bridge for live and recorded frame output."""

from __future__ import annotations

import argparse
import collections
import contextlib
import importlib
import socket
import time
from collections.abc import Callable, Iterable, Iterator
from pathlib import Path

from chipviz_bridge import (
  COLOUR_FIELD_SCENE,
  MAGIC,
  MAX_NOTES,
  SCENE_OFFSET,
  TRANSPORT_SCENE_MASKS,
  VERSION,
  ControlFrame,
  Note,
  WIRE_SIZE,
  add_allow_flash_argument,
  announce_allow_flash,
  checksum,
  clamp_byte,
  make_procedural_frame,
  pack_frame,
  parse_udp_endpoint,
  sanitise_flash,
)
from chipsynth_stream import encode_event_log
from n64_joybus import JOYBUS_MARKER_MASK, JOYBUS_SIZE, convert_stream


DEFAULT_AUDIO_RATE = 48000
DEFAULT_FPS = 60
# Host CC1 debounce mirrors the colour-field-v0 target scene-7 debounce.
SCENE_P_STABLE = 30
SCENE_CUT_MIN = 20
# colour-field-v0 step rate: at most 60 colour-field frames per second.
COLOUR_FIELD_MAX_FPS = 60
NS_PER_SECOND = 1_000_000_000
FLAGS_OFFSET = 22
NOTE_COUNT_OFFSET = 23
# control-frame-v0 flag bits 5..7 are reserved and must be clear.
RESERVED_FLAGS = 0xE0
NOTE_VEL = 64


def require_module(module_name: str, install_hint: str):
  try:
    return importlib.import_module(module_name)
  except ImportError as error:
    raise RuntimeError(f"{module_name} is required; install {install_hint}") from error


def frame_records(stream: bytes) -> list[bytes]:
  if len(stream) % WIRE_SIZE != 0:
    raise ValueError("control-frame stream is not aligned to 33-byte records")
  return [stream[offset:offset + WIRE_SIZE] for offset in range(0, len(stream), WIRE_SIZE)]


def band_edges(sample_rate: int, bands: int) -> list[tuple[float, float]]:
  if bands < 1:
    raise ValueError("bands must be at least 1")
  if sample_rate < 8000:
    raise ValueError("audio sample rate must be at least 8000 Hz")
  low = 40.0
  high = min(12000.0, sample_rate / 2.0)
  ratio = (high / low) ** (1.0 / bands)
  edges = []
  start = low
  for _ in range(bands):
    end = start * ratio
    edges.append((start, end))
    start = end
  return edges


def normalize_bands(values: list[float]) -> tuple[int, ...]:
  peak = max(values, default=0.0)
  if peak <= 0.0:
    return tuple(0 for _ in values)
  return tuple(clamp_byte(round((value / peak) * 255.0)) for value in values)


def frame_from_bands(frame_index: int, bands: tuple[int, ...], midi_velocity: int = 0) -> bytes:
  if len(bands) != 8:
    raise ValueError("frame_from_bands requires exactly 8 bands")
  midi_velocity = clamp_byte(midi_velocity)
  bands = tuple(clamp_byte(value) for value in bands)
  bass = max(bands[0], bands[1])
  mid = max(bands[3], bands[4])
  treble = max(bands[6], bands[7])
  energy = sum(bands) // len(bands)
  flags = 0x01 if energy > 160 or bass > 180 else 0x00
  note = (Note(60, midi_velocity),) if midi_velocity > 0 else ()
  return pack_frame(
    ControlFrame(
      frame=frame_index & 0xFFFF,
      bpm=120,
      beat_phase=(frame_index * 4) & 0xFF,
      beat_count=(frame_index // 30) & 0xFFFF,
      energy=energy,
      bass=bass,
      mid=mid,
      treble=treble,
      spectrum=bands,
      # Skip colour-field scene 7: its palette would cycle every 60 frames.
      scene=(frame_index // 240) % 7,
      palette=(frame_index // 60) & 0x07,
      flags=flags,
      notes=note,
    )
  )


def build_audio_frames(duration: float, sample_rate: int, fps: int, device: str | None) -> list[bytes]:
  if duration <= 0:
    raise ValueError("audio duration must be greater than zero")
  if fps < 1:
    raise ValueError("audio fps must be at least 1")
  numpy = require_module("numpy", "numpy")
  sounddevice = require_module("sounddevice", "sounddevice")
  samples_per_frame = max(1, sample_rate // fps)
  sample_count = int(duration * sample_rate)
  if sample_count < samples_per_frame:
    raise ValueError("audio duration is too short to produce a frame at the requested frame rate")

  recording = sounddevice.rec(sample_count, samplerate=sample_rate, channels=1, dtype="float32", device=device)
  sounddevice.wait()
  mono = numpy.asarray(recording, dtype=numpy.float32).reshape(-1)
  edges = band_edges(sample_rate, 8)
  frames = []
  for frame_index, offset in enumerate(range(0, len(mono) - samples_per_frame + 1, samples_per_frame)):
    chunk = mono[offset:offset + samples_per_frame]
    windowed = chunk * numpy.hanning(len(chunk))
    spectrum = numpy.abs(numpy.fft.rfft(windowed))
    freqs = numpy.fft.rfftfreq(len(windowed), d=1.0 / sample_rate)
    raw_bands = []
    for low, high in edges:
      mask = (freqs >= low) & (freqs < high)
      raw_bands.append(float(spectrum[mask].mean()) if mask.any() else 0.0)
    frames.append(frame_from_bands(frame_index, normalize_bands(raw_bands)))
  return frames


class SceneDebounce:
  """Debounce a continuous scene control (MIDI CC1) across scene 7.

  A change into or out of scene 7 is sent only after the raw value has been
  unchanged for ``SCENE_P_STABLE`` consecutive 60 Hz steps: the change is made
  at the end of the 30th step and sent from the next frame. It is also never
  sent sooner than ``SCENE_CUT_MIN`` steps after the previous scene-7 entry or
  exit. Changes between other scenes are sent at once.

  ``step`` runs once per frame at ``rate`` frames per second. Above 60 Hz the
  step counts scale up (60 frames at 120 Hz), so the debounce lasts at least
  500 ms; at or below 60 Hz it stays 30 frames, which is longer.
  """

  def __init__(self, scene: int = 0, rate: int = DEFAULT_FPS) -> None:
    self.scene = scene
    self._candidate = scene
    self._count = 0
    scale = max(rate, DEFAULT_FPS)
    self._stable = -(-SCENE_P_STABLE * scale // DEFAULT_FPS)
    self._cut_min = -(-SCENE_CUT_MIN * scale // DEFAULT_FPS)
    self._since_cut = self._cut_min

  def step(self, raw: int) -> int:
    """Advance one frame with the raw scene value; return the scene to send."""
    if raw == self._candidate:
      self._count += 1
    else:
      self._candidate = raw
      self._count = 1
    self._since_cut += 1
    if (raw == COLOUR_FIELD_SCENE) == (self.scene == COLOUR_FIELD_SCENE):
      self.scene = raw
      return self.scene
    sent = self.scene
    if self._count >= self._stable and self._since_cut >= self._cut_min:
      self.scene = raw
      self._since_cut = 0
    return sent


def _wire_notes(wire: bytes) -> list[tuple[int, int]]:
  count = min(wire[NOTE_COUNT_OFFSET], MAX_NOTES)
  return [(wire[24 + index * 2], wire[25 + index * 2]) for index in range(count)]


def _wire_valid(wire: bytes) -> bool:
  return (
    len(wire) == WIRE_SIZE
    and wire[0] == MAGIC
    and wire[1] == VERSION
    and not wire[FLAGS_OFFSET] & RESERVED_FLAGS
    and wire[NOTE_COUNT_OFFSET] <= MAX_NOTES
    and checksum(wire[:-1]) == wire[-1]
  )


def _max_velocity(wire: bytes) -> int:
  return max((velocity for _, velocity in _wire_notes(wire)), default=0)


class ColourFieldRateCap:
  """Send at most 60 colour-field frames per second on one output.

  colour-field-v0 "Step rate": only frames whose decoded scene on this
  transport (as in ``sanitise_flash``) is 7 are limited. They follow a 60 Hz
  slot grid: a frame is sent once it is no more than a quarter slot early,
  which absorbs pacing jitter, and never when 60 colour-field frames have
  already been sent in the half-open second before it. Other frames always
  pass. Frames are dropped, never renumbered, so the frame counter stays
  monotonic and only skips values. ``clock`` returns integer nanoseconds.

  Events on dropped frames are carried per target event class, so the target
  sees each class at most once per sent frame, as if it had resampled the
  input itself: ``B`` (a rising edge of beat or bar), ``X`` (a rising edge of
  drop) and ``N`` (a note onset as each transport sees it: a new
  ``(note, velocity)`` pair at ``NOTE_VEL`` or more on the full path; byte 15,
  the highest velocity, rising across ``NOTE_VEL`` on Joybus; none on HID).
  A pending class is satisfied, and cleared, as soon as a sent frame shows that
  class against the previous sent frame, whichever event raised it. Otherwise
  one pending event is written into the frame: a flag bit the previous sent
  frame lacks, or a note in a free slot or replacing the quietest note below
  ``NOTE_VEL`` (on Joybus this raises byte 15 to the carried velocity). A
  class whose pending events the previous sent frame already shows is held
  for at most ``CARRY_HOLD`` sends; notes that only lack a slot (all four hold
  louder notes) wait up to ``CARRY_FULL_HOLD`` sends. Only valid frames (magic, version, reserved flag bits
  clear, note count and checksum) raise events, act as the previous frame, or
  are rewritten; invalid frames pass through unchanged.
  """

  CARRY_HOLD = 2
  CARRY_FULL_HOLD = 30
  _NOTE_EVENTS = {"full": "note", "joybus": "vel", "hid": None}

  def __init__(self, transport: str, clock: Callable[[], int] | None = None) -> None:
    self._mask = TRANSPORT_SCENE_MASKS[transport]
    self._note_event = self._NOTE_EVENTS[transport]
    self._clock = clock or time.monotonic_ns
    # Times are kept in units of 1/60 ns, so one slot is NS_PER_SECOND.
    self._due: int | None = None
    self._window: collections.deque[int] = collections.deque(maxlen=COLOUR_FIELD_MAX_FPS)
    self._last_sent = b""
    self._previous = b""
    # Event class "B", "X" or "N" -> [flag bits or notes, sends held as
    # already shown, sends without a free note slot].
    self._pending: dict[str, list] = {}

  def filter(self, wire: bytes) -> bytes | None:
    """Return the frame to send now, or ``None`` to drop it."""
    if len(wire) != WIRE_SIZE:
      return wire
    now = self._clock()
    colour_field = wire[SCENE_OFFSET] & self._mask == COLOUR_FIELD_SCENE
    send = not colour_field or self._admit(now)
    valid = _wire_valid(wire)
    if valid:
      if self._previous:
        for event_class, item in self._events(self._previous, wire):
          entry = self._pending.setdefault(event_class, [0 if event_class != "N" else [], 0, 0])
          if event_class != "N":
            entry[0] |= item
          elif self._note_event == "vel":
            entry[0] = [item]
          elif item not in entry[0]:
            entry[0].append(item)
          entry[1] = entry[2] = 0
      self._previous = wire
    if not send:
      return None
    if colour_field:
      self._due = max(self._due or 0, now * COLOUR_FIELD_MAX_FPS) + NS_PER_SECOND
      self._window.append(now)
    if not valid:
      return wire
    sent = self._deliver(wire)
    self._last_sent = sent
    return sent

  def _admit(self, now: int) -> bool:
    if len(self._window) == COLOUR_FIELD_MAX_FPS and now - self._window[0] < NS_PER_SECOND:
      return False
    return self._due is None or now * COLOUR_FIELD_MAX_FPS >= self._due - NS_PER_SECOND // 4

  def _events(self, previous: bytes, wire: bytes) -> list[tuple[str, object]]:
    rises = wire[FLAGS_OFFSET] & ~previous[FLAGS_OFFSET]
    events: list[tuple[str, object]] = [
      ("X" if bit == 0x04 else "B", bit) for bit in (0x01, 0x02, 0x04) if rises & bit
    ]
    if self._note_event == "note":
      previous_notes = _wire_notes(previous)
      events += [
        ("N", pair)
        for pair in _wire_notes(wire)
        if pair[1] >= NOTE_VEL and pair not in previous_notes
      ]
    elif self._note_event == "vel" and _max_velocity(previous) < NOTE_VEL <= _max_velocity(wire):
      events.append(("N", max(_wire_notes(wire), key=lambda pair: pair[1])))
    return events

  def _shows_class(self, event_class: str, out: bytes, last: bytes) -> bool:
    """Whether the target sees ``event_class`` at ``out`` after ``last``."""
    last_flags = last[FLAGS_OFFSET] if last else 0
    if event_class != "N":
      bits = 0x04 if event_class == "X" else 0x03
      return bool(out[FLAGS_OFFSET] & ~last_flags & bits)
    if self._note_event == "vel":
      return (_max_velocity(last) if last else 0) < NOTE_VEL <= _max_velocity(out)
    last_notes = _wire_notes(last) if last else []
    return any(pair[1] >= NOTE_VEL and pair not in last_notes for pair in _wire_notes(out))

  def _deliver(self, wire: bytes) -> bytes:
    out = bytearray(wire)
    last = self._last_sent
    for event_class, entry in list(self._pending.items()):
      if self._shows_class(event_class, out, last):
        del self._pending[event_class]
        continue
      if event_class != "N":
        bits = entry[0] & ~(last[FLAGS_OFFSET] if last else 0)
        if bits:
          out[FLAGS_OFFSET] |= bits
          del self._pending[event_class]
          continue
        candidates = []
      elif self._note_event == "vel":
        candidates = entry[0] if not (last and _max_velocity(last) >= NOTE_VEL) else []
      else:
        last_notes = _wire_notes(last) if last else []
        candidates = [pair for pair in entry[0] if pair not in last_notes]
      if not candidates:
        entry[1] += 1
        if entry[1] > self.CARRY_HOLD:
          del self._pending[event_class]
      elif any(self._place_note(out, pair) for pair in candidates):
        del self._pending[event_class]
      else:
        entry[2] += 1
        if entry[2] > self.CARRY_FULL_HOLD:
          del self._pending[event_class]
    out[-1] = checksum(out[:-1])
    return bytes(out)

  @staticmethod
  def _place_note(out: bytearray, pair: tuple[int, int]) -> bool:
    """Put a carried note in a free slot or the quietest sub-threshold one."""
    count = out[NOTE_COUNT_OFFSET]
    if count < MAX_NOTES:
      slot = count
      out[NOTE_COUNT_OFFSET] = count + 1
    else:
      quiet = [index for index in range(MAX_NOTES) if out[25 + index * 2] < NOTE_VEL]
      if not quiet:
        return False
      slot = min(quiet, key=lambda index: out[25 + index * 2])
    out[24 + slot * 2], out[25 + slot * 2] = pair
    return True


def iter_midi_frames(port_name: str | None, frame_count: int, fps: int) -> Iterator[bytes]:
  """Yield control frames captured from a MIDI port at real-time cadence.

  Each iteration spends one ``1/fps`` interval polling the port, so the
  generator paces itself to wall-clock time and downstream sinks must not add
  a second pacing delay.
  """
  if frame_count < 1:
    raise ValueError("MIDI frame count must be at least 1")
  mido = require_module("mido", "mido python-rtmidi")
  velocity = 0
  raw_scene = 0
  debounce = SceneDebounce(rate=fps)
  palette = 0
  interval = 1.0 / fps
  with mido.open_input(port_name) as port:
    for frame_index in range(frame_count):
      deadline = time.monotonic() + interval
      for message in port.iter_pending():
        if message.type == "note_on":
          velocity = int(message.velocity)
        elif message.type == "note_off":
          velocity = 0
        elif message.type == "control_change":
          if message.control == 1:
            raw_scene = int(message.value) // 16
          elif message.control == 2:
            palette = int(message.value) // 16
      scene = debounce.step(raw_scene)
      base = make_procedural_frame(frame_index)
      notes = (Note(60, clamp_byte(velocity * 2)),) if velocity > 0 else ()
      yield pack_frame(ControlFrame(
        frame=base.frame,
        bpm=base.bpm,
        beat_phase=base.beat_phase,
        beat_count=base.beat_count,
        energy=max(base.energy, clamp_byte(velocity * 2)),
        bass=base.bass,
        mid=base.mid,
        treble=base.treble,
        spectrum=base.spectrum,
        scene=scene,
        palette=palette,
        flags=base.flags | (0x02 if velocity > 0 else 0x00),
        notes=notes,
      ))
      remaining = deadline - time.monotonic()
      if remaining > 0:
        time.sleep(remaining)


def open_frame_source(args: argparse.Namespace) -> tuple[Iterable[bytes], bool]:
  """Return a frame iterable and whether it paces itself to real time.

  Live MIDI capture produces frames at wall-clock cadence (``realtime=True``)
  so it must be streamed straight to the sinks; every other source yields
  pre-computed frames that the sinks pace on output.
  """
  if args.input:
    return frame_records(args.input.read_bytes()), False
  if args.chipsynth_log:
    return frame_records(encode_event_log(args.chipsynth_log.read_bytes())), False
  if args.audio_seconds is not None:
    return build_audio_frames(args.audio_seconds, args.audio_rate, args.rate, args.audio_device), False
  if args.midi_port is not None:
    return iter_midi_frames(args.midi_port, args.frames, args.rate), True
  return [pack_frame(make_procedural_frame(index)) for index in range(args.frames)], False


class FileSink:
  """Write the raw control-frame stream to disk; never paced.

  Not rate-capped: a file has no wall-clock rate, and playback is paced (and
  capped) by whatever replays it.
  """

  paced = False
  allow_flash = False

  def __init__(self, path: Path, allow_flash: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    self._handle = path.open("wb")
    self.allow_flash = allow_flash

  def send(self, frame: bytes) -> None:
    self._handle.write(sanitise_flash(frame, "full", self.allow_flash))

  def close(self) -> None:
    self._handle.close()


class UdpSink:
  """Send each control frame to a UDP endpoint; paced and capped on output."""

  paced = True
  allow_flash = False

  def __init__(self, endpoint: tuple[str, int], allow_flash: bool = False) -> None:
    self._endpoint = endpoint
    self._socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    self._cap = ColourFieldRateCap("full")
    self.allow_flash = allow_flash

  def send(self, frame: bytes) -> None:
    frame = self._cap.filter(frame)
    if frame is None:
      return
    self._socket.sendto(sanitise_flash(frame, "full", self.allow_flash), self._endpoint)

  def close(self) -> None:
    self._socket.close()


class N64SerialSink:
  """Send packed 16-byte N64 Joybus frames over serial; paced and capped on output."""

  paced = True
  allow_flash = False

  def __init__(self, device: str, baud: int, allow_flash: bool = False) -> None:
    serial_module = require_module("serial", "pyserial")
    self._port = serial_module.Serial(device, baudrate=baud, timeout=1)
    self._sequence = 0
    self._cap = ColourFieldRateCap("joybus")
    self.allow_flash = allow_flash

  def send(self, frame: bytes) -> None:
    frame = self._cap.filter(frame)
    if frame is None:
      return
    # Own per-packet marker sequence: advances even if the source repeats or
    # stalls its control-frame counter.
    packets = convert_stream(frame, self._sequence, allow_flash=self.allow_flash)
    self._sequence = (self._sequence + len(packets) // JOYBUS_SIZE) & JOYBUS_MARKER_MASK
    self._port.write(packets)
    self._port.flush()

  def close(self) -> None:
    self._port.close()


def stream_frames(frames: Iterable[bytes], realtime: bool, rate: int, sinks: list) -> None:
  """Fan each frame out to every sink in a single pass.

  Pacing is applied only when the source does not already pace itself and at
  least one paced (network/serial) sink is present, so live MIDI capture is no
  longer replayed a second time and file-only output stays instant.
  """
  needs_pacing = (not realtime) and any(sink.paced for sink in sinks)
  interval = 1.0 / rate
  for frame in frames:
    deadline = time.monotonic() + interval
    for sink in sinks:
      sink.send(frame)
    if needs_pacing:
      remaining = deadline - time.monotonic()
      if remaining > 0:
        time.sleep(remaining)


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--input", type=Path, help="raw control-frame-v0 stream")
  parser.add_argument("--chipsynth-log", type=Path, help="chipsynth CSV0 event log")
  parser.add_argument("--frames", type=int, default=120, help="procedural frames if no input is supplied")
  parser.add_argument("--rate", type=int, default=60, help="output frame rate")
  parser.add_argument("--audio-seconds", type=float, help="record this many seconds of audio, then emit frames (offline record-then-replay, not live)")
  parser.add_argument("--audio-rate", type=int, default=DEFAULT_AUDIO_RATE, help="audio capture sample rate")
  parser.add_argument("--audio-device", help="sounddevice input device name or index")
  parser.add_argument("--midi-port", help="mido input port name; omit only when listing ports outside this tool")
  parser.add_argument("--udp", type=parse_udp_endpoint, help="send frames to HOST:PORT over UDP")
  parser.add_argument("--n64-serial", help="send packed 16-byte N64 Joybus frames to this serial device")
  parser.add_argument("--serial-baud", type=int, default=115200, help="serial baud for --n64-serial")
  parser.add_argument("--output", type=Path, help="write normalized control-frame-v0 stream")
  add_allow_flash_argument(parser)
  return parser


def main() -> int:
  args = build_parser().parse_args()
  if args.frames < 1:
    raise SystemExit("--frames must be at least 1")
  if args.rate < 1:
    raise SystemExit("--rate must be at least 1")
  if args.audio_seconds is not None and args.audio_rate < 8000:
    raise SystemExit("--audio-rate must be at least 8000")
  if not args.output and not args.udp and not args.n64_serial:
    raise SystemExit("choose --output, --udp, --n64-serial, or a combination")
  announce_allow_flash(args.allow_flash)

  frames, realtime = open_frame_source(args)
  with contextlib.ExitStack() as stack:
    sinks: list = []
    if args.output:
      sinks.append(stack.enter_context(contextlib.closing(FileSink(args.output, args.allow_flash))))
    if args.udp:
      sinks.append(stack.enter_context(contextlib.closing(UdpSink(args.udp, args.allow_flash))))
    if args.n64_serial:
      sinks.append(stack.enter_context(contextlib.closing(N64SerialSink(args.n64_serial, args.serial_baud, args.allow_flash))))
    stream_frames(frames, realtime, args.rate, sinks)
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
