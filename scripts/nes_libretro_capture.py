#!/usr/bin/env python3
"""Run the NES ROM headlessly through the Mesen libretro core.

This is a minimal libretro frontend for deterministic captures: it loads the
core with ctypes, steps whole frames with a scripted controller-1 input, reads
CPU RAM after each frame and can write the frame buffer as an RGB PNG. No
window, audio device or RetroArch config is involved, so every run of the same
ROM, core and input script gives the same frames.

    python3 scripts/nes_libretro_capture.py --frames 120 --png build/nes.png
"""

from __future__ import annotations

import argparse
import ctypes
import os
import pathlib
import struct
import sys
import zlib
from typing import Callable, Iterable


ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_ROM = ROOT / "cores" / "nes" / "homebrew" / "chipviz-nes.nes"
CORE_EXT = ".dll" if sys.platform == "win32" else ".dylib" if sys.platform == "darwin" else ".so"

# libretro.h constants.
ENV_GET_CAN_DUPE = 3
ENV_GET_SYSTEM_DIRECTORY = 9
ENV_SET_PIXEL_FORMAT = 10
ENV_GET_SAVE_DIRECTORY = 31
PIXEL_0RGB1555, PIXEL_XRGB8888, PIXEL_RGB565 = 0, 1, 2
MEMORY_SYSTEM_RAM = 2
DEVICE_JOYPAD = 1
BUTTONS = {"b": 0, "select": 2, "start": 3, "up": 4, "down": 5, "left": 6, "right": 7, "a": 8}

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


def default_core() -> pathlib.Path:
  cores = os.environ.get("RETROARCH_CORES", str(pathlib.Path.home() / "Library/Application Support/RetroArch/cores"))
  return pathlib.Path(os.environ.get("NES_LIBRETRO_CORE", str(pathlib.Path(cores) / f"mesen_libretro{CORE_EXT}")))


class Frame:
  """One RGB frame: `pixels[y][x]` is an `(r, g, b)` tuple."""

  def __init__(self, width: int, height: int, rgb: bytes) -> None:
    self.width = width
    self.height = height
    self.rgb = rgb

  def pixel(self, x: int, y: int) -> tuple[int, int, int]:
    offset = (y * self.width + x) * 3
    return self.rgb[offset], self.rgb[offset + 1], self.rgb[offset + 2]

  def colors(self, x0: int, y0: int, x1: int, y1: int) -> set[tuple[int, int, int]]:
    found: set[tuple[int, int, int]] = set()
    for y in range(y0, y1):
      row = self.rgb[(y * self.width + x0) * 3:(y * self.width + x1) * 3]
      found.update(zip(row[0::3], row[1::3], row[2::3]))
    return found

  def png(self) -> bytes:
    def chunk(kind: bytes, body: bytes) -> bytes:
      return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    stride = self.width * 3
    raw = b"".join(b"\x00" + self.rgb[y * stride:(y + 1) * stride] for y in range(self.height))
    header = struct.pack(">IIBBBBB", self.width, self.height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


class NesCore:
  """A loaded core and ROM. `close()` one before loading another."""

  def __init__(self, rom: pathlib.Path, core: pathlib.Path | None = None) -> None:
    core = core or default_core()
    self.lib = ctypes.CDLL(str(core))
    self.pixel_format = PIXEL_0RGB1555
    self.frame: Frame | None = None
    self.buttons: frozenset[str] = frozenset()
    self._system_dir = ctypes.create_string_buffer(str(ROOT / "build").encode())
    self._callbacks = [
      ENV_CB(self._environment),
      VIDEO_CB(self._video),
      AUDIO_CB(lambda left, right: None),
      AUDIO_BATCH_CB(lambda data, frames: frames),
      POLL_CB(lambda: None),
      INPUT_CB(self._input),
    ]
    self.lib.retro_set_environment(self._callbacks[0])
    self.lib.retro_init()
    self.lib.retro_set_video_refresh(self._callbacks[1])
    self.lib.retro_set_audio_sample(self._callbacks[2])
    self.lib.retro_set_audio_sample_batch(self._callbacks[3])
    self.lib.retro_set_input_poll(self._callbacks[4])
    self.lib.retro_set_input_state(self._callbacks[5])
    self.rom_bytes = rom.read_bytes()
    self._rom_buffer = ctypes.create_string_buffer(self.rom_bytes, len(self.rom_bytes))
    info = GameInfo(str(rom).encode(), ctypes.cast(self._rom_buffer, ctypes.c_void_p), len(self.rom_bytes), None)
    self.lib.retro_load_game.restype = ctypes.c_bool
    if not self.lib.retro_load_game(ctypes.byref(info)):
      raise RuntimeError(f"core refused {rom}")
    self.lib.retro_get_memory_data.restype = ctypes.c_void_p
    self.lib.retro_get_memory_size.restype = ctypes.c_size_t

  def _environment(self, cmd: int, data: int | None) -> bool:
    cmd &= 0xFFFF  # strip RETRO_ENVIRONMENT_EXPERIMENTAL/PRIVATE bits
    if cmd == ENV_SET_PIXEL_FORMAT and data:
      self.pixel_format = ctypes.c_int.from_address(data).value
      return self.pixel_format in (PIXEL_0RGB1555, PIXEL_XRGB8888, PIXEL_RGB565)
    if cmd in (ENV_GET_SYSTEM_DIRECTORY, ENV_GET_SAVE_DIRECTORY) and data:
      ctypes.c_void_p.from_address(data).value = ctypes.addressof(self._system_dir)
      return True
    if cmd == ENV_GET_CAN_DUPE and data:
      ctypes.c_bool.from_address(data).value = True
      return True
    return False

  def _video(self, data: int | None, width: int, height: int, pitch: int) -> None:
    if not data:
      return  # duplicated frame
    raw = ctypes.string_at(data, pitch * height)
    out = bytearray(width * height * 3)
    o = 0
    for y in range(height):
      row = raw[y * pitch:y * pitch + width * (4 if self.pixel_format == PIXEL_XRGB8888 else 2)]
      if self.pixel_format == PIXEL_XRGB8888:
        out[o:o + width * 3:3] = row[2::4]
        out[o + 1:o + width * 3:3] = row[1::4]
        out[o + 2:o + width * 3:3] = row[0::4]
      else:
        for x in range(width):
          v = row[2 * x] | (row[2 * x + 1] << 8)
          if self.pixel_format == PIXEL_RGB565:
            r, g, b = (v >> 11) & 31, (v >> 5) & 63, v & 31
            out[o + 3 * x:o + 3 * x + 3] = bytes(((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)))
          else:
            r, g, b = (v >> 10) & 31, (v >> 5) & 31, v & 31
            out[o + 3 * x:o + 3 * x + 3] = bytes(((r << 3) | (r >> 2), (g << 3) | (g >> 2), (b << 3) | (b >> 2)))
      o += width * 3
    self.frame = Frame(width, height, bytes(out))

  def _input(self, port: int, device: int, index: int, button: int) -> int:
    if port != 0 or device != DEVICE_JOYPAD:
      return 0
    return int(any(BUTTONS[name] == button for name in self.buttons))

  def run_frame(self, buttons: Iterable[str] = ()) -> None:
    self.buttons = frozenset(buttons)
    self.lib.retro_run()

  def close(self) -> None:
    self.lib.retro_unload_game()
    self.lib.retro_deinit()

  def ram(self) -> bytes:
    size = self.lib.retro_get_memory_size(MEMORY_SYSTEM_RAM)
    address = self.lib.retro_get_memory_data(MEMORY_SYSTEM_RAM)
    if not address or not size:
      raise RuntimeError("core exposes no system RAM")
    return ctypes.string_at(address, size)


def run_script(
    rom: pathlib.Path,
    frames: int,
    buttons_at: Callable[[int], Iterable[str]] = lambda frame: (),
    on_frame: Callable[[int, NesCore], None] | None = None,
    core: pathlib.Path | None = None) -> NesCore:
  """Run `frames` frames; `buttons_at(i)` is the pad state for frame `i`."""
  nes = NesCore(rom, core)
  for index in range(frames):
    nes.run_frame(buttons_at(index))
    if on_frame is not None:
      on_frame(index, nes)
  return nes


def main() -> int:
  parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
  parser.add_argument("--rom", type=pathlib.Path, default=DEFAULT_ROM)
  parser.add_argument("--core", type=pathlib.Path, default=None)
  parser.add_argument("--frames", type=int, default=120)
  parser.add_argument("--png", type=pathlib.Path, required=True)
  args = parser.parse_args()
  nes = run_script(args.rom, args.frames, core=args.core)
  if nes.frame is None:
    print("core produced no frame", file=sys.stderr)
    return 1
  args.png.parent.mkdir(parents=True, exist_ok=True)
  args.png.write_bytes(nes.frame.png())
  print(f"{args.png} {nes.frame.width}x{nes.frame.height}")
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
