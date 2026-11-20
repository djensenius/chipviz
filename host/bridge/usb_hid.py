#!/usr/bin/env python3
"""Map control-frame-v0 records to reduced USB HID states."""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

from n64_joybus import decode_control_frame
from chipviz_bridge import WIRE_SIZE, add_allow_flash_argument, announce_allow_flash, sanitise_flash

PALETTE_KEYS = ("Q", "W", "E", "R", "T", "Y", "U", "I")
GBA_SCENES = (0, 1, 2, 3, 7)
STABLE_POLLS = 60
SOURCE_STABLE = 30


@dataclass(frozen=True)
class HidState:
  buttons: int
  left_x: int
  left_y: int
  keys: tuple[str, ...]


@dataclass(frozen=True)
class GamepadState:
  buttons: int
  left_x: int
  left_y: int
  dpad: tuple[str, ...]
  keys: tuple[str, ...] = ()


def gba_targets(wire: bytes, allow_flash: bool) -> tuple[int, int]:
  frame = decode_control_frame(sanitise_flash(wire, "hid", allow_flash))
  scene = frame.scene & 7
  return (7 if scene == 7 else scene & 3, frame.palette & 7)


class GbaGamepadMapper:
  """Estimate receiver rings assuming every output is delivered in order."""

  def __init__(self, scene: int, palette: int, allow_flash: bool = False) -> None:
    self.allow_flash = allow_flash
    self.resync(scene, palette)

  def disconnect(self) -> None:
    self.scene = None
    self.palette = None
    self.wait = 0
    self.target_scene = None
    self.target_palette = None

  def resync(self, scene: int, palette: int) -> None:
    if scene not in GBA_SCENES or palette not in range(8):
      raise ValueError("known GBA scene must be 0..3 or 7, and palette 0..7")
    if not self.allow_flash and scene == 7 and palette in (3, 7):
      raise ValueError("resynchronize to a safe palette before using scene 7")
    self.scene: int | None = scene
    self.palette: int | None = palette
    self.wait = 0
    self.target_scene: int | None = scene
    self.target_palette: int | None = palette
    self.source_candidate = scene
    self.source_count = 0

  @staticmethod
  def _direction(ring: tuple[int, ...], current: int, target: int,
                 forbidden: tuple[int, ...] = ()) -> int:
    if current == target:
      return 0
    choices = []
    for direction in (1, -1):
      index = ring.index(current)
      for distance in range(1, len(ring)):
        index = (index + direction) % len(ring)
        if ring[index] in forbidden:
          break
        if ring[index] == target:
          choices.append((distance, -direction, direction))
          break
    if not choices:
      raise ValueError("no permitted ring route")
    return min(choices)[2]

  def step(self, wire: bytes, *, padding: bool = False) -> GamepadState:
    if self.scene is None or self.palette is None:
      raise ValueError("receiver state unknown; resynchronize before conversion")
    base = map_frame(wire, self.allow_flash)
    if not padding:
      source_scene, palette = gba_targets(wire, self.allow_flash)
      if source_scene != self.source_candidate:
        self.source_candidate = source_scene
        self.source_count = 1
      else:
        self.source_count = min(SOURCE_STABLE, self.source_count + 1)
      if ((source_scene == 7) == (self.target_scene == 7)
          or self.source_count >= SOURCE_STABLE):
        self.target_scene = source_scene
      if not self.allow_flash and self.target_scene == 7 and palette in (3, 7):
        palette &= ~1
      self.target_palette = palette
    target_scene = self.target_scene
    target_palette = self.target_palette
    assert target_scene is not None and target_palette is not None
    dpad: tuple[str, ...] = ()
    if self.wait:
      self.wait -= 1
    else:
      # Leave scene 7 before changing a native palette; prepare the palette
      # before entering it. Native-to-native routes never pass through 7.
      palette_first = target_scene == 7 and self.scene != 7
      scene_first = self.scene == 7 and target_scene != 7
      if palette_first and self.palette != target_palette:
        axis = "palette"
      elif scene_first or self.scene != target_scene:
        axis = "scene"
      else:
        axis = "palette"
      if axis == "scene":
        forbidden = (7,) if self.scene != 7 and target_scene != 7 else ()
        direction = self._direction(GBA_SCENES, self.scene, target_scene, forbidden)
        if direction:
          self.scene = GBA_SCENES[(GBA_SCENES.index(self.scene) + direction) % len(GBA_SCENES)]
          dpad = ("up" if direction == 1 else "down",)
      else:
        # Relative positions 3/7 do not arm a conforming data receiver (F24).
        direction = self._direction(tuple(range(8)), self.palette, target_palette)
        if direction:
          self.palette = (self.palette + direction) % 8
          dpad = ("right" if direction == 1 else "left",)
      if dpad:
        self.wait = STABLE_POLLS - 1
    buttons = base.buttons
    if padding:
      # Beat, bar and drop are onsets; fill, manual and energy are levels.
      flags = decode_control_frame(wire).flags
      buttons &= ~0x07
      if flags & 0x08:
        buttons |= 0x04
    return GamepadState(buttons, base.left_x, base.left_y, dpad)


def convert_gba_stream(stream: bytes, scene: int, palette: int,
                       allow_flash: bool = False) -> list[GamepadState]:
  if len(stream) % WIRE_SIZE != 0:
    raise ValueError("control-frame stream is not aligned to 33-byte records")
  mapper = GbaGamepadMapper(scene, palette, allow_flash)
  states = []
  for offset in range(0, len(stream), WIRE_SIZE):
    wire = stream[offset:offset + WIRE_SIZE]
    states.append(mapper.step(wire))
  if stream:
    # Finish the latest selection and its stability interval.
    last_wire = stream[-WIRE_SIZE:]
    while mapper.wait or (mapper.scene, mapper.palette) != (mapper.target_scene, mapper.target_palette):
      states.append(mapper.step(last_wire, padding=True))
    if states[-1].buttons:
      states.append(GamepadState(0, states[-1].left_x, states[-1].left_y, ()))
  return states


def signed_bucket(value: int) -> int:
  return max(-127, min(127, value - 128))


def map_frame(wire: bytes, allow_flash: bool = False) -> HidState:
  # Flash guard for the HID decoded scene (`scene & 0x07`).
  frame = decode_control_frame(sanitise_flash(wire, "hid", allow_flash))
  buttons = 0
  if frame.flags & 0x01:
    buttons |= 1 << 0
  if frame.flags & 0x02:
    buttons |= 1 << 1
  if frame.flags & 0x0C:
    buttons |= 1 << 2
  if frame.flags & 0x10:
    buttons |= 1 << 3
  if frame.energy > 170:
    buttons |= 1 << 4
  if frame.spectrum[7] > 170:
    buttons |= 1 << 5

  keys = []
  keys.append(str((frame.scene & 0x07) + 1))
  keys.append(PALETTE_KEYS[frame.palette & 0x07])
  if frame.flags & 0x01:
    keys.append("Space")
  if frame.flags & 0x02:
    keys.append("Return")
  if frame.flags & 0x0C:
    keys.append("F")
  if frame.flags & 0x10:
    keys.append("M")

  return HidState(
    buttons=buttons,
    left_x=signed_bucket(frame.energy),
    left_y=signed_bucket(frame.beat_phase),
    keys=tuple(keys),
  )


def convert_stream(stream: bytes, allow_flash: bool = False) -> list[HidState]:
  if len(stream) % WIRE_SIZE != 0:
    raise ValueError("control-frame stream is not aligned to 33-byte records")
  return [
    map_frame(stream[offset:offset + WIRE_SIZE], allow_flash)
    for offset in range(0, len(stream), WIRE_SIZE)
  ]


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--input", type=Path, required=True, help="raw control-frame-v0 stream")
  parser.add_argument("--output", type=Path, help="JSON HID state stream; stdout if omitted")
  parser.add_argument("--gba-gamepad", action="store_true", help="opt in to GBA relative D-pad output")
  parser.add_argument("--initial-scene", type=int, choices=GBA_SCENES)
  parser.add_argument("--initial-palette", type=int, choices=range(8))
  add_allow_flash_argument(parser)
  return parser


def main_from_args(argv: list[str] | None = None) -> int:
  args = build_parser().parse_args(argv)
  if args.gba_gamepad != (args.initial_scene is not None and args.initial_palette is not None):
    raise SystemExit("--gba-gamepad requires both --initial-scene and --initial-palette")
  if not args.gba_gamepad and (args.initial_scene is not None or args.initial_palette is not None):
    raise SystemExit("initial selections require --gba-gamepad")
  announce_allow_flash(args.allow_flash)
  stream = args.input.read_bytes()
  if args.gba_gamepad:
    mapped = convert_gba_stream(stream, args.initial_scene, args.initial_palette, args.allow_flash)
  else:
    mapped = convert_stream(stream, args.allow_flash)
  states = [asdict(state) for state in mapped]
  output = json.dumps(states, indent=2).encode("utf-8") + b"\n"
  if args.output is None or str(args.output) == "-":
    sys.stdout.buffer.write(output)
  else:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(output)
  return 0


def main() -> int:
  return main_from_args()


if __name__ == "__main__":
  raise SystemExit(main())
