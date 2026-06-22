#!/usr/bin/env python3
"""Raspberry Pi/development host bridge for live and recorded frame output."""

from __future__ import annotations

import argparse
import importlib
import socket
import time
from pathlib import Path

from chipviz_bridge import ControlFrame, Note, WIRE_SIZE, clamp_byte, make_procedural_frame, pack_frame, parse_udp_endpoint
from chipsynth_stream import encode_event_log
from n64_joybus import convert_stream


DEFAULT_AUDIO_RATE = 48000
DEFAULT_FPS = 60


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
      scene=(frame_index // 240) & 0x07,
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
  sample_count = int(duration * sample_rate)
  if sample_count < fps:
    raise ValueError("audio duration is too short for the requested frame rate")

  recording = sounddevice.rec(sample_count, samplerate=sample_rate, channels=1, dtype="float32", device=device)
  sounddevice.wait()
  mono = numpy.asarray(recording, dtype=numpy.float32).reshape(-1)
  samples_per_frame = max(1, sample_rate // fps)
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


def build_midi_frames(port_name: str | None, frame_count: int, fps: int) -> list[bytes]:
  if frame_count < 1:
    raise ValueError("MIDI frame count must be at least 1")
  mido = require_module("mido", "mido python-rtmidi")
  frames = []
  velocity = 0
  scene = 0
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
            scene = int(message.value) // 16
          elif message.control == 2:
            palette = int(message.value) // 16
      base = make_procedural_frame(frame_index)
      notes = (Note(60, clamp_byte(velocity * 2)),) if velocity > 0 else ()
      frames.append(pack_frame(ControlFrame(
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
      )))
      remaining = deadline - time.monotonic()
      if remaining > 0:
        time.sleep(remaining)
  return frames


def build_frames(args: argparse.Namespace) -> list[bytes]:
  if args.input:
    return frame_records(args.input.read_bytes())
  if args.chipsynth_log:
    return frame_records(encode_event_log(args.chipsynth_log.read_bytes()))
  if args.audio_seconds is not None:
    return build_audio_frames(args.audio_seconds, args.audio_rate, args.rate, args.audio_device)
  if args.midi_port is not None:
    return build_midi_frames(args.midi_port, args.frames, args.rate)
  return [pack_frame(make_procedural_frame(index)) for index in range(args.frames)]


def send_udp(frames: list[bytes], endpoint: tuple[str, int], rate: int) -> None:
  interval = 1.0 / rate
  with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp_socket:
    for frame in frames:
      udp_socket.sendto(frame, endpoint)
      time.sleep(interval)


def send_n64_serial(frames: list[bytes], device: str, baud: int, rate: int) -> None:
  serial_module = require_module("serial", "pyserial")
  interval = 1.0 / rate
  joybus_stream = convert_stream(b"".join(frames))
  packets = [joybus_stream[offset:offset + 16] for offset in range(0, len(joybus_stream), 16)]
  with serial_module.Serial(device, baudrate=baud, timeout=1) as port:
    for packet in packets:
      port.write(packet)
      port.flush()
      time.sleep(interval)


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--input", type=Path, help="raw control-frame-v0 stream")
  parser.add_argument("--chipsynth-log", type=Path, help="chipsynth CSV0 event log")
  parser.add_argument("--frames", type=int, default=120, help="procedural frames if no input is supplied")
  parser.add_argument("--rate", type=int, default=60, help="output frame rate")
  parser.add_argument("--audio-seconds", type=float, help="capture live audio for this many seconds")
  parser.add_argument("--audio-rate", type=int, default=DEFAULT_AUDIO_RATE, help="audio capture sample rate")
  parser.add_argument("--audio-device", help="sounddevice input device name or index")
  parser.add_argument("--midi-port", help="mido input port name; omit only when listing ports outside this tool")
  parser.add_argument("--udp", type=parse_udp_endpoint, help="send frames to HOST:PORT over UDP")
  parser.add_argument("--n64-serial", help="send packed 16-byte N64 Joybus frames to this serial device")
  parser.add_argument("--serial-baud", type=int, default=115200, help="serial baud for --n64-serial")
  parser.add_argument("--output", type=Path, help="write normalized control-frame-v0 stream")
  return parser


def main() -> int:
  args = build_parser().parse_args()
  if args.frames < 1:
    raise SystemExit("--frames must be at least 1")
  if args.rate < 1:
    raise SystemExit("--rate must be at least 1")
  if args.audio_rate < 8000:
    raise SystemExit("--audio-rate must be at least 8000")

  frames = build_frames(args)
  if args.output:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(b"".join(frames))
  if args.udp:
    send_udp(frames, args.udp, args.rate)
  if args.n64_serial:
    send_n64_serial(frames, args.n64_serial, args.serial_baud, args.rate)
  if not args.output and not args.udp and not args.n64_serial:
    raise SystemExit("choose --output, --udp, --n64-serial, or a combination")
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
