#!/usr/bin/env python3
"""Run the Game Boy ROM headlessly through the SameBoy libretro core.

A minimal libretro frontend for deterministic captures. It loads the core with
ctypes, forces the DMG model with the greyscale palette and no border, steps
whole frames with a scripted joypad, reads and writes WRAM, and can save the
frame buffer as an RGB PNG. No window, audio device or RetroArch config is
involved, so every run of the same ROM, core and input gives the same frames.

    python3 scripts/gb_libretro_capture.py --frames 120 --png build/gb.png
"""

from __future__ import annotations

import argparse
import ctypes
import os
import pathlib
import struct
import sys
import tempfile
import zlib
from typing import Iterable


ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_ROM = ROOT / "cores" / "gb" / "homebrew" / "chipviz-gb.gb"
CORE_EXT = ".dll" if sys.platform == "win32" else ".dylib" if sys.platform == "darwin" else ".so"

# libretro.h constants.
ENV_GET_CAN_DUPE = 3
ENV_GET_SYSTEM_DIRECTORY = 9
ENV_SET_PIXEL_FORMAT = 10
ENV_GET_VARIABLE = 15
ENV_GET_SAVE_DIRECTORY = 31
PIXEL_0RGB1555, PIXEL_XRGB8888, PIXEL_RGB565 = 0, 1, 2
MEMORY_SYSTEM_RAM = 2
DEVICE_JOYPAD = 1
BUTTONS = {"b": 0, "select": 2, "start": 3, "up": 4, "down": 5, "left": 6, "right": 7, "a": 8}
OPTIONS = {
  b"sameboy_model": b"Game Boy",
  b"sameboy_mono_palette": b"greyscale",
  b"sameboy_border": b"never",
  b"sameboy_color_correction_mode": b"off",
}

ENV_CB = ctypes.CFUNCTYPE(ctypes.c_bool, ctypes.c_uint, ctypes.c_void_p)
VIDEO_CB = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint, ctypes.c_size_t)
AUDIO_CB = ctypes.CFUNCTYPE(None, ctypes.c_int16, ctypes.c_int16)
AUDIO_BATCH_CB = ctypes.CFUNCTYPE(ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t)
POLL_CB = ctypes.CFUNCTYPE(None)
INPUT_CB = ctypes.CFUNCTYPE(ctypes.c_int16, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint)


class GameInfo(ctypes.Structure):
  _fields_ = [
    ("path", ctypes.c_char_p),
    ("data", ctypes.c_void_p),
    ("size", ctypes.c_size_t),
    ("meta", ctypes.c_char_p),
  ]


class Variable(ctypes.Structure):
  _fields_ = [("key", ctypes.c_char_p), ("value", ctypes.c_char_p)]


def default_core() -> pathlib.Path:
  cores = os.environ.get("RETROARCH_CORES", str(pathlib.Path.home() / "Library/Application Support/RetroArch/cores"))
  return pathlib.Path(os.environ.get("GB_LIBRETRO_CORE", str(pathlib.Path(cores) / f"sameboy_libretro{CORE_EXT}")))


def png_bytes(width: int, height: int, rgb: bytes) -> bytes:
  def chunk(kind: bytes, body: bytes) -> bytes:
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

  stride = width * 3
  raw = b"".join(b"\x00" + rgb[y * stride:(y + 1) * stride] for y in range(height))
  return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
          + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


class Frame:
  """One RGB frame."""

  def __init__(self, width: int, height: int, rgb: bytes) -> None:
    self.width = width
    self.height = height
    self.rgb = rgb

  def pixel(self, x: int, y: int) -> tuple[int, int, int]:
    offset = (y * self.width + x) * 3
    return self.rgb[offset], self.rgb[offset + 1], self.rgb[offset + 2]

  def png(self) -> bytes:
    return png_bytes(self.width, self.height, self.rgb)


class GbCore:
  """A loaded SameBoy core running one ROM. Only one instance at a time."""

  def __init__(self, rom: pathlib.Path, core: pathlib.Path | None = None, video: bool = True) -> None:
    self.lib = ctypes.CDLL(str(core or default_core()))
    self.video = video
    self.buttons: frozenset[str] = frozenset()
    self.format = PIXEL_0RGB1555
    self.frame: Frame | None = None
    self._dirs = tempfile.TemporaryDirectory()
    self._dir = ctypes.c_char_p(self._dirs.name.encode())
    self._values = {key: ctypes.c_char_p(value) for key, value in OPTIONS.items()}
    self._callbacks = [
      ENV_CB(self._environment),
      VIDEO_CB(self._video),
      AUDIO_CB(lambda left, right: None),
      AUDIO_BATCH_CB(lambda data, frames: frames),
      POLL_CB(lambda: None),
      INPUT_CB(self._input),
    ]
    env, video_cb, audio, batch, poll, inp = self._callbacks
    self.lib.retro_set_environment(env)
    self.lib.retro_init()
    self.lib.retro_set_video_refresh(video_cb)
    self.lib.retro_set_audio_sample(audio)
    self.lib.retro_set_audio_sample_batch(batch)
    self.lib.retro_set_input_poll(poll)
    self.lib.retro_set_input_state(inp)
    self._rom = pathlib.Path(rom).read_bytes()
    self._rom_buf = ctypes.create_string_buffer(self._rom, len(self._rom))
    info = GameInfo(str(rom).encode(), ctypes.cast(self._rom_buf, ctypes.c_void_p), len(self._rom), None)
    self.lib.retro_load_game.restype = ctypes.c_bool
    if not self.lib.retro_load_game(ctypes.byref(info)):
      raise RuntimeError(f"core refused {rom}")
    self.lib.retro_get_memory_data.restype = ctypes.c_void_p
    self.lib.retro_get_memory_size.restype = ctypes.c_size_t
    size = self.lib.retro_get_memory_size(MEMORY_SYSTEM_RAM)
    address = self.lib.retro_get_memory_data(MEMORY_SYSTEM_RAM)
    if not address or size < 0x2000:
      raise RuntimeError("core exposes no WRAM")
    self._wram = (ctypes.c_uint8 * size).from_address(address)

  def _environment(self, cmd: int, data: int) -> bool:
    if cmd == ENV_SET_PIXEL_FORMAT:
      self.format = ctypes.cast(data, ctypes.POINTER(ctypes.c_int)).contents.value
      return self.format in (PIXEL_XRGB8888, PIXEL_RGB565, PIXEL_0RGB1555)
    if cmd == ENV_GET_VARIABLE:
      var = ctypes.cast(data, ctypes.POINTER(Variable)).contents
      value = self._values.get(var.key)
      if value is None:
        return False
      var.value = value
      return True
    if cmd in (ENV_GET_SYSTEM_DIRECTORY, ENV_GET_SAVE_DIRECTORY):
      ctypes.cast(data, ctypes.POINTER(ctypes.c_char_p))[0] = self._dir
      return True
    if cmd == ENV_GET_CAN_DUPE:
      ctypes.cast(data, ctypes.POINTER(ctypes.c_bool))[0] = True
      return True
    return False

  def _input(self, port: int, device: int, index: int, button: int) -> int:
    if port != 0 or device != DEVICE_JOYPAD:
      return 0
    return int(any(BUTTONS[name] == button for name in self.buttons))

  def _video(self, data: int, width: int, height: int, pitch: int) -> None:
    if not data or not self.video:
      return
    raw = ctypes.string_at(data, pitch * height)
    rgb = bytearray(width * height * 3)
    out = 0
    for y in range(height):
      row = raw[y * pitch:y * pitch + width * (4 if self.format == PIXEL_XRGB8888 else 2)]
      if self.format == PIXEL_XRGB8888:
        rgb[out:out + width * 3:3] = row[2::4]
        rgb[out + 1:out + width * 3:3] = row[1::4]
        rgb[out + 2:out + width * 3:3] = row[0::4]
      else:
        for x in range(width):
          v = row[2 * x] | row[2 * x + 1] << 8
          if self.format == PIXEL_RGB565:
            r, g, b = (v >> 11) & 31, (v >> 5) & 63, v & 31
            rgb[out + 3 * x:out + 3 * x + 3] = bytes((r * 255 // 31, g * 255 // 63, b * 255 // 31))
          else:
            r, g, b = (v >> 10) & 31, (v >> 5) & 31, v & 31
            rgb[out + 3 * x:out + 3 * x + 3] = bytes((r * 255 // 31, g * 255 // 31, b * 255 // 31))
      out += width * 3
    self.frame = Frame(width, height, bytes(rgb))

  def run_frame(self, buttons: Iterable[str] = ()) -> None:
    self.buttons = frozenset(buttons)
    unknown = self.buttons - BUTTONS.keys()
    if unknown:
      raise ValueError(f"unknown button(s): {', '.join(sorted(unknown))}; expected one of {', '.join(BUTTONS)}")
    self.lib.retro_run()

  def read(self, address: int, size: int = 1) -> bytes:
    offset = address - 0xC000
    return bytes(self._wram[offset:offset + size])

  def write(self, address: int, data: bytes) -> None:
    offset = address - 0xC000
    for i, value in enumerate(data):
      self._wram[offset + i] = value

  def reset(self) -> None:
    self.lib.retro_reset()

  def close(self) -> None:
    self.lib.retro_unload_game()
    self.lib.retro_deinit()
    self._dirs.cleanup()


def main(argv: list[str] | None = None) -> int:
  parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
  parser.add_argument("--rom", type=pathlib.Path, default=DEFAULT_ROM)
  parser.add_argument("--core", type=pathlib.Path, default=None)
  parser.add_argument("--frames", type=int, default=120)
  parser.add_argument("--press", action="append", default=[], metavar="FRAME:BUTTON[+BUTTON]",
                      help="hold buttons for one frame, e.g. 1:up")
  parser.add_argument("--png", type=pathlib.Path, help="write the last frame as an RGB PNG")
  args = parser.parse_args(argv)
  presses: dict[int, set[str]] = {}
  for item in args.press:
    frame, _, names = item.partition(":")
    buttons = set(names.split("+"))
    unknown = buttons - BUTTONS.keys()
    if unknown:
      parser.error(f"--press {item}: unknown button(s) {', '.join(sorted(unknown))}; expected one of {', '.join(BUTTONS)}")
    presses.setdefault(int(frame), set()).update(buttons)
  gb = GbCore(args.rom, args.core)
  try:
    for frame in range(args.frames):
      gb.run_frame(presses.get(frame, ()))
    if args.png and gb.frame is not None:
      args.png.parent.mkdir(parents=True, exist_ok=True)
      args.png.write_bytes(gb.frame.png())
    print("trace", gb.read(0xC000, 9).hex(" "))
  finally:
    gb.close()
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
