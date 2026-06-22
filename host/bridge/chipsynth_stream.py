#!/usr/bin/env python3
"""Parse ChipStation visualization stream packets and map them to chipviz frames."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path

from chipviz_bridge import ControlFrame, Note, pack_frame


MAGIC = b"CSV0"
VERSION = 0
PACKET_SIZE = 81
CHANNEL_COUNT = 16
VOICE_COUNT = 4
VOICE_SIZE = 8

FLAG_PLAYING = 1 << 0
FLAG_BEAT = 1 << 1
FLAG_BAR = 1 << 2
FLAG_FILL = 1 << 3
KNOWN_FRAME_FLAGS = FLAG_PLAYING | FLAG_BEAT | FLAG_BAR | FLAG_FILL

VOICE_ACTIVE = 1 << 0
VOICE_GATE = 1 << 1
KNOWN_VOICE_FLAGS = VOICE_ACTIVE | VOICE_GATE

CHIP_IDS = range(0x00, 0x06)

# Demo packets advertise a fixed 128.50 BPM (bpm_x100, little-endian u16).
DEMO_BPM_X100 = 12850


@dataclass(frozen=True)
class Voice:
  voice: int
  channel: int
  chip: int
  note: int
  velocity: int
  level: int
  flags: int


@dataclass(frozen=True)
class ChipStationVizFrame:
  sequence: int
  bpm_x100: int
  beat_phase: int
  beat_count: int
  flags: int
  active_voice_count: int
  midi_note_count: int
  channel_levels: tuple[int, ...]
  channel_chips: tuple[int, ...]
  voices: tuple[Voice, ...]


def checksum(payload: bytes | bytearray) -> int:
  value = 0
  for byte in payload:
    value ^= byte
  return value


def clamp_byte(value: int) -> int:
  return max(0, min(255, value))


def parse_packet(packet: bytes) -> ChipStationVizFrame:
  if len(packet) != PACKET_SIZE:
    raise ValueError(f"chipsynth packet must be {PACKET_SIZE} bytes")
  if packet[:4] != MAGIC:
    raise ValueError("bad chipsynth packet magic")
  if packet[4] != VERSION:
    raise ValueError(f"unsupported chipsynth packet version: {packet[4]}")
  if packet[15] != 0:
    raise ValueError("chipsynth reserved header byte must be zero")
  if checksum(packet[:-1]) != packet[-1]:
    raise ValueError("bad chipsynth packet checksum")
  if packet[5] & ~KNOWN_FRAME_FLAGS:
    raise ValueError("chipsynth packet has unknown frame flag bits")
  if packet[13] > VOICE_COUNT or packet[14] > VOICE_COUNT:
    raise ValueError("chipsynth packet voice counts exceed fixed voice window")

  channel_levels = tuple(packet[16:32])
  channel_chips = tuple(packet[32:48])
  for chip in channel_chips:
    if chip not in CHIP_IDS:
      raise ValueError(f"invalid chipsynth channel chip id: {chip}")

  voices = []
  for index in range(VOICE_COUNT):
    offset = 48 + index * VOICE_SIZE
    if packet[offset + 7] != 0:
      raise ValueError("chipsynth voice reserved byte must be zero")
    voice = Voice(
      voice=packet[offset],
      channel=packet[offset + 1],
      chip=packet[offset + 2],
      note=packet[offset + 3],
      velocity=packet[offset + 4],
      level=packet[offset + 5],
      flags=packet[offset + 6],
    )
    if voice.channel >= CHANNEL_COUNT:
      raise ValueError(f"invalid chipsynth voice channel: {voice.channel}")
    if voice.chip not in CHIP_IDS:
      raise ValueError(f"invalid chipsynth voice chip id: {voice.chip}")
    if voice.note > 127:
      raise ValueError(f"invalid chipsynth voice note: {voice.note}")
    if voice.flags & ~KNOWN_VOICE_FLAGS:
      raise ValueError("chipsynth voice has unknown flag bits")
    voices.append(voice)

  return ChipStationVizFrame(
    sequence=int.from_bytes(packet[6:8], "little"),
    bpm_x100=int.from_bytes(packet[8:10], "little"),
    beat_phase=packet[10],
    beat_count=int.from_bytes(packet[11:13], "little"),
    flags=packet[5],
    active_voice_count=packet[13],
    midi_note_count=packet[14],
    channel_levels=channel_levels,
    channel_chips=channel_chips,
    voices=tuple(voices),
  )


def to_control_frame(source: ChipStationVizFrame) -> ControlFrame:
  active_voices = [
    voice
    for voice in source.voices
    if voice.flags & (VOICE_ACTIVE | VOICE_GATE) and voice.note <= 127
  ]
  levels = list(source.channel_levels)
  voice_levels = [voice.level for voice in active_voices]
  energy = max(levels + voice_levels + [0])
  spectrum = tuple(
    clamp_byte((levels[index * 2] + levels[index * 2 + 1]) // 2)
    for index in range(8)
  )

  dominant_voice = dominant_active_voice(active_voices)
  scene = dominant_voice.chip if dominant_voice else most_common_chip(source.channel_chips)
  palette = dominant_voice.channel if dominant_voice else dominant_channel(levels)
  flags = 0
  if source.flags & FLAG_BEAT:
    flags |= 1 << 0
  if source.flags & FLAG_BAR:
    flags |= 1 << 1
  if source.flags & FLAG_FILL:
    flags |= 1 << 3

  notes = tuple(
    Note(voice.note, voice.velocity)
    for voice in active_voices[:4]
  )

  return ControlFrame(
    frame=source.sequence & 0xFFFF,
    bpm=clamp_byte(round(source.bpm_x100 / 100)),
    beat_phase=source.beat_phase,
    beat_count=source.beat_count & 0xFFFF,
    energy=energy,
    bass=average(levels[:4]),
    mid=average(levels[4:12]),
    treble=average(levels[12:16]),
    spectrum=spectrum,
    scene=scene,
    palette=palette,
    flags=flags,
    notes=notes,
  )


def average(values: list[int]) -> int:
  if not values:
    return 0
  return clamp_byte(sum(values) // len(values))


def dominant_channel(levels: list[int]) -> int:
  return max(range(len(levels)), key=lambda index: levels[index])


def dominant_active_voice(voices: list[Voice]) -> Voice | None:
  if not voices:
    return None
  return max(voices, key=lambda voice: (voice.level, voice.velocity, -voice.voice))


def most_common_chip(chips: tuple[int, ...]) -> int:
  return max(CHIP_IDS, key=lambda chip: chips.count(chip))


def pack_demo_packet(
  sequence: int,
  beat_phase: int,
  beat_count: int,
  flags: int,
  voices: list[Voice],
) -> bytes:
  packet = bytearray(PACKET_SIZE)
  packet[:4] = MAGIC
  packet[4] = VERSION
  packet[5] = flags
  packet[6:8] = (sequence & 0xFFFF).to_bytes(2, "little")
  packet[8:10] = DEMO_BPM_X100.to_bytes(2, "little")
  packet[10] = beat_phase & 0xFF
  packet[11:13] = (beat_count & 0xFFFF).to_bytes(2, "little")
  packet[13] = sum(1 for voice in voices if voice.flags & VOICE_ACTIVE)
  packet[14] = sum(1 for voice in voices if voice.flags & VOICE_GATE)

  for index in range(CHANNEL_COUNT):
    packet[16 + index] = clamp_byte(16 + index * 8 + (sequence * 3) % 24)
    packet[32 + index] = 1 if index % 2 == 0 else 2

  for index, voice in enumerate(voices):
    offset = 48 + index * VOICE_SIZE
    packet[offset:offset + VOICE_SIZE] = bytes((
      voice.voice,
      voice.channel,
      voice.chip,
      voice.note,
      voice.velocity,
      voice.level,
      voice.flags,
      0,
    ))

  packet[-1] = checksum(packet[:-1])
  return bytes(packet)


def make_demo_packet() -> bytes:
  packet = bytearray(PACKET_SIZE)
  packet[:4] = MAGIC
  packet[4] = VERSION
  packet[5] = FLAG_PLAYING | FLAG_BEAT
  packet[6:8] = (42).to_bytes(2, "little")
  packet[8:10] = DEMO_BPM_X100.to_bytes(2, "little")
  packet[10] = 96
  packet[11:13] = (7).to_bytes(2, "little")
  packet[13] = 2
  packet[14] = 2

  for index in range(CHANNEL_COUNT):
    packet[16 + index] = 16 + index * 8
    packet[32 + index] = 1 if index % 2 == 0 else 2

  voices = [
    Voice(0, 0, 1, 60, 110, 120, VOICE_ACTIVE | VOICE_GATE),
    Voice(1, 1, 2, 67, 96, 88, VOICE_ACTIVE),
    Voice(2, 2, 1, 72, 0, 0, 0),
    Voice(3, 3, 2, 76, 0, 0, 0),
  ]
  for index, voice in enumerate(voices):
    offset = 48 + index * VOICE_SIZE
    packet[offset:offset + VOICE_SIZE] = bytes((
      voice.voice,
      voice.channel,
      voice.chip,
      voice.note,
      voice.velocity,
      voice.level,
      voice.flags,
      0,
    ))

  packet[-1] = checksum(packet[:-1])
  return bytes(packet)


def make_demo_log(count: int) -> bytes:
  """Build a deterministic chipsynth event log of ``count`` packets."""
  scale = (60, 62, 64, 67, 72, 76, 79, 84)
  packets = []
  for index in range(count):
    beat_phase = (index * 32) & 0xFF
    beat_count = index // 8
    flags = FLAG_PLAYING
    if beat_phase < 32:
      flags |= FLAG_BEAT
      if beat_count % 4 == 0:
        flags |= FLAG_BAR
    if beat_count % 4 == 3:
      flags |= FLAG_FILL

    lead = scale[index % len(scale)]
    bass = 36 + (beat_count % 4) * 2
    voices = [
      Voice(0, 0, 0x05, bass, 112, clamp_byte(140 - beat_phase // 2),
            VOICE_ACTIVE | VOICE_GATE),
      Voice(1, 2, 0x03, lead, clamp_byte(80 + (index * 7) % 48),
            clamp_byte(96 + (index * 5) % 80), VOICE_ACTIVE | VOICE_GATE),
      Voice(2, 5, 0x04, lead + 7, 64, clamp_byte(48 + (index * 11) % 64),
            VOICE_ACTIVE if index % 2 == 0 else 0),
      Voice(3, 9, 0x02, 0, 0, 0, 0),
    ]
    packets.append(pack_demo_packet(index, beat_phase, beat_count, flags, voices))
  return b"".join(packets)


def iter_packets(stream: bytes) -> list[bytes]:
  if not stream or len(stream) % PACKET_SIZE != 0:
    raise ValueError(
      f"chipsynth event log must be a positive multiple of {PACKET_SIZE} bytes"
    )
  return [stream[offset:offset + PACKET_SIZE] for offset in range(0, len(stream), PACKET_SIZE)]


def encode_event_log(stream: bytes) -> bytes:
  """Map a chipsynth event log into a raw control-frame-v0 stream."""
  return b"".join(
    pack_frame(to_control_frame(parse_packet(packet)))
    for packet in iter_packets(stream)
  )


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--input", type=Path, help="chipsynth stream packet to parse")
  parser.add_argument("--stream", type=Path, help="chipsynth event log (many packets) to map")
  parser.add_argument("--demo-packet", type=Path, help="write a deterministic demo packet")
  parser.add_argument(
    "--demo-log",
    type=Path,
    help="write a deterministic multi-packet demo event log",
  )
  parser.add_argument(
    "--demo-log-frames",
    type=int,
    default=96,
    help="packet count for --demo-log",
  )
  parser.add_argument("--output", type=Path, help="write mapped control-frame-v0 stream")
  return parser


def main() -> int:
  args = build_parser().parse_args()

  if args.demo_packet:
    args.demo_packet.parent.mkdir(parents=True, exist_ok=True)
    args.demo_packet.write_bytes(make_demo_packet())
  if args.demo_log:
    args.demo_log.parent.mkdir(parents=True, exist_ok=True)
    args.demo_log.write_bytes(make_demo_log(args.demo_log_frames))

  if args.stream:
    stream = encode_event_log(args.stream.read_bytes())
    if args.output:
      args.output.parent.mkdir(parents=True, exist_ok=True)
      args.output.write_bytes(stream)
    else:
      print(f"mapped {len(stream) // 33} chipsynth packets")
    return 0

  packet = args.input.read_bytes() if args.input else make_demo_packet()
  frame = to_control_frame(parse_packet(packet))
  if args.output:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(pack_frame(frame))
  if not args.demo_packet and not args.demo_log and not args.output:
    print(frame)
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
