#!/usr/bin/env python3
"""Deterministic C64 colour-field captures through the VICE x64sc libretro core.

This is a minimal headless libretro frontend (ctypes, standard library only).
It loads `chipviz-c64.prg`, presses usb-hid-transport-v0 keyboard keys on fixed
emulated frames, saves PNG captures and checks each one: every pixel of the
320x200 display window belongs to its region's expected colour, region
boundaries sit at the colour-field-v0 formula positions, and the border is
black. Like `scripts/retroarch_smoke.py` it needs a local RetroArch core, so it
is not part of `just check`.

Only the held and hard-cut modes are captured (keys Q/E/T/U). Flash/gate is
never armed: palette keys R and I are refused, except for one negative check
that holds R without the scene key 8, which must run as hard cuts (mode 2) with
a steady picture. `--model ntsc` runs the same sequence on an NTSC C64 (one
model per process: the VICE core is a singleton). The script also reads the core's RAM (the `_cf` address from
the linker map) to measure the scene-7 entry and exit timing in frames.
"""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
import pathlib
import struct
import sys
import tempfile
import zlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "shared" / "tools"))

import capture_assert  # noqa: E402

CORE_EXT = ".dll" if sys.platform == "win32" else ".dylib" if sys.platform == "darwin" else ".so"
DEFAULT_CORES = pathlib.Path.home() / "Library/Application Support/RetroArch/cores"

RETRO_DEVICE_KEYBOARD = 3
RETRO_PIXEL_FORMAT_XRGB8888 = 1
ENV_GET_CAN_DUPE = 3
ENV_GET_SYSTEM_DIRECTORY = 9
ENV_SET_PIXEL_FORMAT = 10
ENV_GET_VARIABLE = 15
ENV_SET_VARIABLES = 16
ENV_GET_VARIABLE_UPDATE = 17
ENV_SET_KEYBOARD_CALLBACK = 12
ENV_GET_LOG_INTERFACE = 27
ENV_GET_SAVE_DIRECTORY = 31
ENV_GET_CORE_OPTIONS_VERSION = 52

# Core options pinned for reproducible captures.
CORE_OPTIONS = {
  "vice_c64_model": "C64 PAL auto",
  "vice_gfx_colors": "24bit",
  "vice_physical_keyboard_pass_through": "enabled",
  "vice_autostart": "enabled",
  "vice_autoloadwarp": "disabled",
  "vice_drive_true_emulation": "disabled",
  "vice_external_palette": "pepto-pal",
  "vice_crop": "disabled",
  "vice_aspect_ratio": "pal",
  "vice_retropad_options": "disabled",
}

MODELS = {"pal": "C64 PAL auto", "ntsc": "C64 NTSC auto"}
# KERNAL PAL/NTSC flag ($02A6) that each model must set: the program picks
# its raster IRQ line from it.
KERNAL_PALNTSC = 0x02A6
PALNTSC_FLAG = {"pal": 1, "ntsc": 0}
NATIVE_FRAMES = 60  # frames sampled to check the native demo animates

# Pepto PAL palette (VICE pepto-pal.vpl), indexed by C64 colour number.
PEPTO = {
  0: (0x00, 0x00, 0x00), 1: (0xFF, 0xFF, 0xFF), 2: (0x68, 0x37, 0x2B), 3: (0x70, 0xA4, 0xB2),
  4: (0x6F, 0x3D, 0x86), 5: (0x58, 0x8D, 0x43), 6: (0x35, 0x28, 0x79), 7: (0xB8, 0xC7, 0x6F),
  10: (0x9A, 0x67, 0x59),
}

# Per colour-field-v0: [family][regions - 2][combo] -> entries (C64 indices).
PALETTE = {
  0: {2: [(10, 6), (7, 6), (10, 7), (6, 1)], 3: [(10, 7, 6), (10, 1, 6), (7, 0, 6), (10, 0, 7)]},
  1: {2: [(2, 3), (5, 4), (6, 7), (3, 4)], 3: [(2, 5, 6), (3, 4, 7), (2, 0, 3), (6, 0, 7)]},
}

KEYS = {"8": ord("8"), "1": ord("1"), "q": ord("q"), "e": ord("e"), "r": ord("r"), "t": ord("t"),
        "u": ord("u"), "space": 32}
FORBIDDEN = {"r", "i"}  # palettes 3 and 7: flash/gate.

BOOT = 400   # frames for VICE autostart to load and RUN the program
ENTER_LIMIT = 40  # frames: 30-step debounce at one step per frame, plus margin
EXIT_LIMIT = 40
RETRO_MEMORY_SYSTEM_RAM = 2
# cf_state_t byte offsets (cores/c64/homebrew/src/colour_field.h).
CF_MODE = 0
CF_SHOWN = 9
CF_FLASH = 17
GAP = 60     # frames between scripted steps (P_STABLE + CUT_MIN margin)
TAP = 3      # frames a Space tap is held


def boundaries(size_cells: int, regions: int) -> list[int]:
  """Pixel boundaries b_i = g * ((i * (S / g)) / n) with g = 8."""
  return [8 * ((i * size_cells) // regions) for i in range(regions)] + [8 * size_cells]


def region_rects(layout: int) -> list[tuple[int, int, int, int]]:
  """(x, y, w, h) within the 320x200 window for each region, region 0 first."""
  regions = 3 if layout & 2 else 2
  if layout & 1:
    b = boundaries(25, regions)
    return [(0, 200 - b[i + 1], 320, b[i + 1] - b[i]) for i in range(regions)]
  b = boundaries(40, regions)
  return [(b[i], 0, b[i + 1] - b[i], 200) for i in range(regions)]


def expected_colours(family: int, layout: int, combo: int, rot: int) -> list[int]:
  regions = 3 if layout & 2 else 2
  entries = PALETTE[family][regions][combo]
  return [entries[(i + rot) % regions] for i in range(regions)]


def cut_states(layout: int, combo: int, rot: int, count: int) -> list[tuple[int, int, int]]:
  """States after each hard cut with no drop: actions rotate, next_combo,
  rotate, next_layout (colour-field-v0 mode 2), from act = 0."""
  states = []
  for act in range(count):
    regions = 3 if layout & 2 else 2
    if act % 4 == 1:
      combo = (combo + 1) & 3
    elif act % 4 == 3:
      layout = (layout + 1) & 3
      rot %= 3 if layout & 2 else 2
    else:
      rot = (rot + 1) % regions
    states.append((layout, combo, rot))
  return states


def write_png(path: pathlib.Path, width: int, height: int, rgb: bytes) -> None:
  rows = b"".join(b"\x00" + rgb[y * width * 3:(y + 1) * width * 3] for y in range(height))

  def chunk(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

  png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
  png += chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")
  path.write_bytes(png)


class RetroVariable(ctypes.Structure):
  _fields_ = [("key", ctypes.c_char_p), ("value", ctypes.c_char_p)]


class RetroGameInfo(ctypes.Structure):
  _fields_ = [("path", ctypes.c_char_p), ("data", ctypes.c_void_p), ("size", ctypes.c_size_t),
              ("meta", ctypes.c_char_p)]


ENV_CB = ctypes.CFUNCTYPE(ctypes.c_bool, ctypes.c_uint, ctypes.c_void_p)
VIDEO_CB = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint, ctypes.c_size_t)
AUDIO_CB = ctypes.CFUNCTYPE(None, ctypes.c_int16, ctypes.c_int16)
AUDIO_BATCH_CB = ctypes.CFUNCTYPE(ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t)
POLL_CB = ctypes.CFUNCTYPE(None)
LOG_CB = ctypes.CFUNCTYPE(None, ctypes.c_int, ctypes.c_char_p)  # variadic tail ignored
STATE_CB = ctypes.CFUNCTYPE(ctypes.c_int16, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint)
# retro_keyboard_event_t(down, keycode, character, key_modifiers)
KEYBOARD_CB = ctypes.CFUNCTYPE(None, ctypes.c_bool, ctypes.c_uint, ctypes.c_uint32, ctypes.c_uint16)


class Emulator:
  def __init__(self, core: pathlib.Path, workdir: pathlib.Path) -> None:
    self.lib = ctypes.CDLL(str(core))
    self.lib.retro_load_game.restype = ctypes.c_bool
    self._held: set[int] = set()
    self._keyboard: KEYBOARD_CB | None = None
    self.frame: tuple[int, int, bytes] | None = None
    self._strings = {k: ctypes.c_char_p(v.encode()) for k, v in CORE_OPTIONS.items()}
    self._dir = ctypes.c_char_p(str(workdir).encode())
    self._callbacks = [ENV_CB(self._env), VIDEO_CB(self._video), AUDIO_CB(lambda left, right: None),
                       AUDIO_BATCH_CB(lambda data, frames: frames), POLL_CB(lambda: None),
                       STATE_CB(self._state), LOG_CB(self._log)]
    env, video, audio, batch, poll, state, _ = self._callbacks
    self.lib.retro_set_environment(env)
    self.lib.retro_set_video_refresh(video)
    self.lib.retro_set_audio_sample(audio)
    self.lib.retro_set_audio_sample_batch(batch)
    self.lib.retro_set_input_poll(poll)
    self.lib.retro_set_input_state(state)
    self.lib.retro_init()

  def _env(self, cmd: int, data: int) -> bool:
    cmd &= 0xFFFF
    if cmd == ENV_GET_CAN_DUPE:
      ctypes.cast(data, ctypes.POINTER(ctypes.c_bool))[0] = True
      return True
    if cmd in (ENV_GET_SYSTEM_DIRECTORY, ENV_GET_SAVE_DIRECTORY):
      ctypes.cast(data, ctypes.POINTER(ctypes.c_char_p))[0] = self._dir.value
      return True
    if cmd == ENV_SET_PIXEL_FORMAT:
      return ctypes.cast(data, ctypes.POINTER(ctypes.c_int))[0] == RETRO_PIXEL_FORMAT_XRGB8888
    if cmd == ENV_SET_KEYBOARD_CALLBACK:
      # VICE reads keys from these events, not from input_state polling.
      self._keyboard = ctypes.cast(data, ctypes.POINTER(KEYBOARD_CB))[0]
      return True
    if cmd == ENV_SET_VARIABLES:
      # "Description; default|other|..." per key; unset options use defaults.
      variables = ctypes.cast(data, ctypes.POINTER(RetroVariable))
      index = 0
      while variables[index].key:
        key = variables[index].key.decode()
        default = variables[index].value.decode().split("; ", 1)[-1].split("|", 1)[0]
        if key not in self._strings:
          self._strings[key] = ctypes.c_char_p(default.encode())
        index += 1
      return True
    if cmd == ENV_GET_VARIABLE:
      var = ctypes.cast(data, ctypes.POINTER(RetroVariable))[0]
      value = self._strings.get(var.key.decode() if var.key else "")
      if value is None:
        return False
      ctypes.cast(data, ctypes.POINTER(RetroVariable))[0].value = value.value
      return True
    if cmd == ENV_GET_LOG_INTERFACE:
      ctypes.cast(data, ctypes.POINTER(LOG_CB))[0] = self._callbacks[-1]
      return True
    if cmd == ENV_GET_VARIABLE_UPDATE:
      ctypes.cast(data, ctypes.POINTER(ctypes.c_bool))[0] = False
      return True
    if cmd == ENV_GET_CORE_OPTIONS_VERSION:
      ctypes.cast(data, ctypes.POINTER(ctypes.c_uint))[0] = 0
      return True
    return False

  def _log(self, level: int, fmt: bytes) -> None:
    if os.environ.get("CHIPVIZ_CAPTURE_DEBUG"):
      sys.stderr.write(f"core[{level}]: {fmt.decode(errors='replace')}")

  def _video(self, data: int, width: int, height: int, pitch: int) -> None:
    if not data:
      return
    raw = ctypes.string_at(data, pitch * height)
    rgb = bytearray(width * height * 3)
    for y in range(height):
      row = raw[y * pitch:y * pitch + width * 4]
      out = rgb[y * width * 3:(y + 1) * width * 3]
      out[0::3] = row[2::4]
      out[1::3] = row[1::4]
      out[2::3] = row[0::4]
      rgb[y * width * 3:(y + 1) * width * 3] = out
    self.frame = (width, height, bytes(rgb))

  def _state(self, port: int, device: int, index: int, key: int) -> int:
    return int(port == 0 and device == RETRO_DEVICE_KEYBOARD and key in self._held)

  @property
  def held(self) -> set[int]:
    return set(self._held)

  @held.setter
  def held(self, keys: set[int]) -> None:
    if self._keyboard is None:
      raise RuntimeError("core did not register a keyboard callback")
    for key in sorted(self._held - keys):
      self._keyboard(False, key, 0, 0)
    for key in sorted(keys - self._held):
      self._keyboard(True, key, key if key < 128 else 0, 0)
    self._held = set(keys)

  def load(self, prg: pathlib.Path) -> None:
    info = RetroGameInfo(str(prg).encode(), None, 0, None)
    if not self.lib.retro_load_game(ctypes.byref(info)):
      raise RuntimeError(f"core refused {prg}")

  def peek(self, address: int) -> int:
    self.lib.retro_get_memory_data.restype = ctypes.c_void_p
    self.lib.retro_get_memory_size.restype = ctypes.c_size_t
    base = self.lib.retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM)
    if not base or address >= self.lib.retro_get_memory_size(RETRO_MEMORY_SYSTEM_RAM):
      raise RuntimeError("core exposes no system RAM")
    return ctypes.string_at(base + address, 1)[0]

  def run(self, frames: int) -> None:
    for _ in range(frames):
      self.lib.retro_run()


def symbol(map_file: pathlib.Path, name: str) -> int:
  words = map_file.read_text().split()
  return int(words[words.index(name) + 1], 16)


def colour_count(frame: tuple[int, int, bytes]) -> int:
  rgb = frame[2]
  return len(set(rgb[i:i + 3] for i in range(0, len(rgb), 3)))


def find_window(width: int, height: int, rgb: bytes) -> tuple[int, int]:
  """Top-left of the 320x200 display window in the native boot frame, whose
  border colour differs from its background (the title starts at column 1)."""
  border = rgb[:3]
  for y in range(height):
    for x in range(width):
      if rgb[(y * width + x) * 3:(y * width + x) * 3 + 3] != border:
        return x, y
  raise ValueError("no display window found")


def check(name: str, png: pathlib.Path, family: int, layout: int, combo: int, rot: int,
          origin: tuple[int, int]) -> dict[str, object]:
  image = capture_assert.decode_png(png.read_bytes())
  ox, oy = origin
  failures = []
  for index, (rect, colour) in enumerate(zip(region_rects(layout), expected_colours(family, layout, combo, rot))):
    x, y, w, h = rect
    _, rows = capture_assert.region_rows(image, capture_assert.Region(ox + x, oy + y, w, h))
    seen = {rows[r][c:c + 3] for r in range(h) for c in range(0, w * 3, 3)}
    if seen != {bytes(PEPTO[colour])}:
      failures.append(f"region {index} {rect} expected colour {colour} {PEPTO[colour]}, saw {sorted(seen)[:4]}")
  window = [(ox - 1, oy, 1, 200), (ox + 320, oy, 1, 200), (ox, oy - 1, 320, 1), (ox, oy + 200, 320, 1)]
  for rect in window:
    _, rows = capture_assert.region_rows(image, capture_assert.Region(*rect))
    if any(byte for row in rows for byte in row):
      failures.append(f"border {rect} is not black")
  return {"capture": name, "png": str(png), "sha256": hashlib.sha256(png.read_bytes()).hexdigest(),
          "family": family, "layout": layout, "combo": combo, "rot": rot,
          "colours": expected_colours(family, layout, combo, rot), "failures": failures}


def main(argv: list[str] | None = None) -> int:
  parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
  parser.add_argument("--core", type=pathlib.Path,
                      default=pathlib.Path(os.environ.get("RETROARCH_CORES", DEFAULT_CORES)) /
                      f"vice_x64sc_libretro{CORE_EXT}")
  parser.add_argument("--prg", type=pathlib.Path, default=ROOT / "cores/c64/homebrew/chipviz-c64.prg")
  parser.add_argument("--map", type=pathlib.Path, default=ROOT / "cores/c64/homebrew/build/chipviz-c64.map")
  parser.add_argument("--model", choices=sorted(MODELS), default="pal")
  parser.add_argument("--output", type=pathlib.Path, default=ROOT / "build/c64-colour-field")
  args = parser.parse_args(argv)
  if not args.core.exists() or not args.prg.exists() or not args.map.exists():
    print(f"missing core, prg or map: {args.core} {args.prg} {args.map}", file=sys.stderr)
    return 2
  cf = symbol(args.map, "_cf")
  CORE_OPTIONS["vice_c64_model"] = MODELS[args.model]
  prefix = "c64-" if args.model == "pal" else f"c64-{args.model}-"
  args.output.mkdir(parents=True, exist_ok=True)

  with tempfile.TemporaryDirectory() as workdir:
    emu = Emulator(args.core, pathlib.Path(workdir))
    emu.load(args.prg.resolve())
    emu.run(BOOT)
    frame = BOOT
    results = []
    assert emu.frame is not None
    origin = find_window(*emu.frame)
    palntsc = emu.peek(KERNAL_PALNTSC)
    if palntsc != PALNTSC_FLAG[args.model]:
      print(f"$02A6 is {palntsc}, expected {PALNTSC_FLAG[args.model]} for {args.model}", file=sys.stderr)
      return 1
    native_frames = set()
    for _ in range(NATIVE_FRAMES):
      emu.run(1)
      assert emu.frame is not None
      native_frames.add(hashlib.sha256(emu.frame[2]).hexdigest())
    frame += NATIVE_FRAMES

    def hold(*keys: str, unarmed: bool = False) -> None:
      if FORBIDDEN & set(keys) and (unarmed is False or "8" in keys):
        raise ValueError("flash/gate palettes are never captured")
      emu.held = {KEYS[k] for k in keys}

    def advance(frames: int) -> None:
      nonlocal frame
      emu.run(frames)
      frame += frames

    def tap(*keys: str) -> None:
      held = {k for k, v in KEYS.items() if v in emu.held}
      hold(*(held | {"space"}))
      advance(TAP)
      hold(*keys)
      advance(GAP - TAP)

    def capture(name: str, family: int, layout: int, combo: int, rot: int) -> None:
      assert emu.frame is not None
      width, height, rgb = emu.frame
      png = args.output / f"{prefix}{name}.png"
      write_png(png, width, height, rgb)
      result = check(name, png, family, layout, combo, rot, origin)
      result["frame"] = frame
      results.append(result)

    def until(test, limit: int) -> int:
      """Frames run until test() holds (checked after each frame)."""
      for count in range(1, limit + 1):
        advance(1)
        if test():
          return count
      raise RuntimeError(f"condition not met within {limit} frames")

    # Held (Q, p=0) family A: entry after the scene-7 debounce, V2 combo dom=0.
    # Steps run once per frame from a raster IRQ, whatever the native demo does.
    hold("8", "q")
    timing = {"entry_step_frames": until(lambda: emu.peek(cf + CF_SHOWN) == 1, ENTER_LIMIT)}
    timing["entry_visible_frames"] = timing["entry_step_frames"] + until(
      lambda: emu.frame is not None and colour_count(emu.frame) == 3, 3)
    advance(GAP)
    capture("held-a-v2", 0, 0, 0, 0)
    # Hard cuts (E, p=2): Space taps (beat) cut through all four layouts.
    names = ["v2", "h2", "v3", "h3"]
    state = (0, 0, 0)

    def cuts(family: int, cuts_key: str, count: int, at: tuple[int, ...]) -> None:
      nonlocal state
      for index, state in enumerate(cut_states(*state, count)):
        tap("8", cuts_key)
        if index in at:
          layout, combo, rot = state
          capture(f"cuts-{'ab'[family]}-{names[layout]}-c{combo}r{rot}", family, *state)

    for family, held_key, cuts_key in ((0, "q", "e"), (1, "t", "u")):
      if family:
        # Held family B keeps layout and rot; combo = dom = 0 (equal bands).
        state = (state[0], 0, state[2])
        hold("8", held_key)
        advance(GAP)
        capture(f"held-b-{names[state[0]]}", family, *state)
      hold("8", cuts_key)
      advance(GAP)
      cuts(family, cuts_key, 16, (3, 7, 11))
      if not family:
        # White (index 1) is family A combo 1 in V3 (10/1/6). Combo and layout
        # advance in lockstep under cuts, so cut to V3, reset combo to 0 with
        # held (keeps layout and rot), then cut once more to combo 1.
        cuts(family, cuts_key, 8, ())
        state = (state[0], 0, state[2])
        hold("8", held_key)
        advance(GAP)
        hold("8", cuts_key)
        advance(GAP)
        cuts(family, cuts_key, 2, (1,))
    # Negative check: R (palette 3) without the scene key is not an absolute
    # report, so it never arms flash/gate and runs as hard cuts (family A).
    hold("r", unarmed=True)
    advance(GAP)
    modes = set()
    frames = set()
    for _ in range(2 * GAP):
      advance(1)
      modes.add((emu.peek(cf + CF_MODE), emu.peek(cf + CF_FLASH)))
      assert emu.frame is not None
      frames.add(hashlib.sha256(emu.frame[2]).hexdigest())
    capture(f"unarmed-r-{names[state[0]]}", 0, *state)
    unarmed = {"modes_flash": sorted(modes), "distinct_frames": len(frames)}
    # Scene key 1: the debounce leaves scene 7 for the native demo, whose last
    # drawn frame is still in screen RAM at $0400.
    hold("1", "q")
    timing["exit_step_frames"] = until(lambda: emu.peek(cf + CF_SHOWN) == 0, EXIT_LIMIT)
    timing["exit_visible_frames"] = timing["exit_step_frames"] + until(
      lambda: emu.frame is not None and colour_count(emu.frame) > 3, 3)
    advance(2 * GAP)
    native = emu.frame
    assert native is not None
    write_png(args.output / f"{prefix}native-after-exit.png", *native)
    failed = [r for r in results if r["failures"]]
    report = {"core": str(args.core), "prg_sha256": hashlib.sha256(args.prg.read_bytes()).hexdigest(),
              "model": args.model, "palntsc_flag": palntsc, "core_options": CORE_OPTIONS, "boot_frames": BOOT,
              "window_origin": origin, "native_distinct_frames": len(native_frames),
              "native_after_exit_colours": colour_count(native), "timing": timing, "unarmed_r": unarmed,
              "captures": results}
    print(json.dumps(report, indent=2))
    if len(native_frames) < NATIVE_FRAMES // 2:
      print("native demo is not animating", file=sys.stderr)
      return 1
    if report["native_after_exit_colours"] < 3:
      print("native scene did not return after scene-7 exit", file=sys.stderr)
      return 1
    if unarmed["modes_flash"] != [(2, 0)] or unarmed["distinct_frames"] != 1:
      print("palette R without scene key 8 must stay in steady hard cuts", file=sys.stderr)
      return 1
    return 1 if failed else 0


if __name__ == "__main__":
  raise SystemExit(main())
