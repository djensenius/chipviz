"""Frame-level SMS libretro capture; no instruction or VDP-write timing."""

import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import zlib


ENV_GET_CAN_DUPE, ENV_GET_SYSTEM_DIR, ENV_SET_PIXEL_FORMAT = 3, 9, 10
ENV_GET_VARIABLE, ENV_GET_SAVE_DIR = 15, 31
PIXEL_1555, PIXEL_XRGB8888, PIXEL_RGB565 = 0, 1, 2
MEMORY_SYSTEM_RAM, MEMORY_VIDEO_RAM = 2, 3
DEVICE_JOYPAD = 1
# SMS active-high bit order; IDs are libretro joypad IDs, not port $DC bits.
BUTTON_IDS = (4, 5, 6, 7, 0, 8)
CALLBACK_ENV = ctypes.CFUNCTYPE(ctypes.c_bool, ctypes.c_uint, ctypes.c_void_p)
CALLBACK_VIDEO = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint, ctypes.c_size_t)
CALLBACK_AUDIO = ctypes.CFUNCTYPE(None, ctypes.c_int16, ctypes.c_int16)
CALLBACK_BATCH = ctypes.CFUNCTYPE(ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t)
CALLBACK_POLL = ctypes.CFUNCTYPE(None)
CALLBACK_INPUT = ctypes.CFUNCTYPE(ctypes.c_int16, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint)


class GameInfo(ctypes.Structure):
  _fields_ = [("path", ctypes.c_char_p), ("data", ctypes.c_void_p),
              ("size", ctypes.c_size_t), ("meta", ctypes.c_char_p)]


class Variable(ctypes.Structure):
  _fields_ = [("key", ctypes.c_char_p), ("value", ctypes.c_char_p)]


class Geometry(ctypes.Structure):
  _fields_ = [("base_width", ctypes.c_uint), ("base_height", ctypes.c_uint),
              ("max_width", ctypes.c_uint), ("max_height", ctypes.c_uint),
              ("aspect_ratio", ctypes.c_float)]


class Timing(ctypes.Structure):
  _fields_ = [("fps", ctypes.c_double), ("sample_rate", ctypes.c_double)]


class AvInfo(ctypes.Structure):
  _fields_ = [("geometry", Geometry), ("timing", Timing)]


class SystemInfo(ctypes.Structure):
  _fields_ = [("library_name", ctypes.c_char_p), ("library_version", ctypes.c_char_p),
              ("valid_extensions", ctypes.c_char_p), ("need_fullpath", ctypes.c_bool),
              ("block_extract", ctypes.c_bool)]


def decode_frame(data, width, height, pitch, pixel_format):
  size = 4 if pixel_format == PIXEL_XRGB8888 else 2
  if pixel_format not in (PIXEL_1555, PIXEL_XRGB8888, PIXEL_RGB565) or pitch < width * size:
    raise ValueError("unsupported libretro framebuffer")
  if len(data) != pitch * height:
    raise ValueError("truncated libretro framebuffer")
  out = bytearray(width * height * 3)
  for y in range(height):
    for x in range(width):
      offset = y * pitch + x * size
      value = int.from_bytes(data[offset:offset + size], "little")
      if pixel_format == PIXEL_XRGB8888:
        rgb = ((value >> 16) & 255, (value >> 8) & 255, value & 255)
      elif pixel_format == PIXEL_RGB565:
        r, g, b = (value >> 11) & 31, (value >> 5) & 63, value & 31
        rgb = ((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2))
      else:
        r, g, b = (value >> 10) & 31, (value >> 5) & 31, value & 31
        rgb = tuple((v << 3) | (v >> 2) for v in (r, g, b))
      index = (y * width + x) * 3
      out[index:index + 3] = bytes(rgb)
  return bytes(out)


def png(width, height, rgb):
  if len(rgb) != width * height * 3:
    raise ValueError("invalid RGB dimensions")
  def chunk(kind, body):
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xffffffff)
  rows = b"".join(b"\0" + rgb[y * width * 3:(y + 1) * width * 3] for y in range(height))
  return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
          + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def schedule(step):
  pad = 0
  if 10 <= step <= 12 or step == 750:
    pad |= 1
  if step in (60, 110, 550, 600):
    pad |= 8
  if step in range(180, 511, 30):
    pad |= 16
  if step == 680:
    pad |= 12
  return pad


def target_snapshot(ram):
  if len(ram) < 0x110:
    raise ValueError(f"SMS system RAM unavailable: {len(ram)} bytes")
  return {"buttons": ram[1], "scene": ram[3], "palette": ram[4], "shown": ram[8],
          "visible": ram[10], "entry": ram[11], "peff": ram[13],
          "input": list(ram[0x20:0x30]), "procedural_counter": int.from_bytes(ram[0x30:0x32], "little"),
          "pad_active": ram[0x33], "trace": list(ram[0x60:0x68]),
          "present": list(ram[0x100:0x104])}


class SmsCore:
  def __init__(self, core, rom, region, workdir):
    self.lib = ctypes.CDLL(str(core))
    self.pad = 0
    self.polls = 0
    self.input_calls = 0
    self.pixel_format = PIXEL_1555
    self.frame = None
    self.fresh_frame = False
    self.video_error = None
    self.video_calls = 0
    self.options_seen = {}
    self._dir = ctypes.c_char_p(str(workdir).encode())
    settings = {
        b"genesis_plus_gx_system_hw": b"master system",
        b"genesis_plus_gx_region_detect": b"ntsc-u" if region == "ntsc" else b"pal",
        b"genesis_plus_gx_vdp_mode": b"60hz" if region == "ntsc" else b"50hz",
        b"genesis_plus_gx_bios": b"disabled",
        b"genesis_plus_gx_overclock": b"100%",
        b"genesis_plus_gx_overscan": b"disabled",
        b"genesis_plus_gx_blargg_ntsc_filter": b"disabled",
    }
    self._options = {key: ctypes.c_char_p(value) for key, value in settings.items()}
    self.callbacks = (CALLBACK_ENV(self.environment), CALLBACK_VIDEO(self.video),
                      CALLBACK_AUDIO(lambda left, right: None),
                      CALLBACK_BATCH(lambda data, frames: frames), CALLBACK_POLL(self.poll),
                      CALLBACK_INPUT(self.input))
    env, video, audio, batch, poll, state = self.callbacks
    self.lib.retro_set_environment(env)
    self.lib.retro_init()
    self.lib.retro_set_video_refresh(video)
    self.lib.retro_set_audio_sample(audio)
    self.lib.retro_set_audio_sample_batch(batch)
    self.lib.retro_set_input_poll(poll)
    self.lib.retro_set_input_state(state)
    self._rom_path = str(rom.resolve()).encode()
    info = GameInfo(self._rom_path, None, 0, None)
    self.lib.retro_load_game.restype = ctypes.c_bool
    if not self.lib.retro_load_game(ctypes.byref(info)):
      raise RuntimeError("core refused ROM")
    self.lib.retro_get_memory_data.restype = ctypes.c_void_p
    self.lib.retro_get_memory_size.restype = ctypes.c_size_t
    self.lib.retro_get_region.restype = ctypes.c_uint
    av = AvInfo()
    self.lib.retro_get_system_av_info(ctypes.byref(av))
    self.region = self.lib.retro_get_region()
    self.fps = av.timing.fps
    info = SystemInfo()
    self.lib.retro_get_system_info(ctypes.byref(info))
    self.core_name = info.library_name.decode()
    self.core_version = info.library_version.decode()

  def environment(self, command, data):
    command &= 0xffff
    if command == ENV_SET_PIXEL_FORMAT and data:
      self.pixel_format = ctypes.cast(data, ctypes.POINTER(ctypes.c_int)).contents.value
      return self.pixel_format in (PIXEL_1555, PIXEL_XRGB8888, PIXEL_RGB565)
    if command == ENV_GET_VARIABLE and data:
      variable = ctypes.cast(data, ctypes.POINTER(Variable)).contents
      value = self._options.get(variable.key)
      if value is None:
        variable.value = None
        return False
      variable.value = value
      self.options_seen[variable.key.decode()] = value.value.decode()
      return True
    if command in (ENV_GET_SYSTEM_DIR, ENV_GET_SAVE_DIR) and data:
      ctypes.cast(data, ctypes.POINTER(ctypes.c_char_p))[0] = self._dir
      return True
    if command == ENV_GET_CAN_DUPE and data:
      ctypes.cast(data, ctypes.POINTER(ctypes.c_bool))[0] = True
      return True
    return False

  def video(self, data, width, height, pitch):
    self.video_calls += 1
    if data:
      try:
        raw = ctypes.string_at(data, pitch * height)
        self.frame = (width, height, decode_frame(raw, width, height, pitch, self.pixel_format))
        self.fresh_frame = True
      except Exception as error:
        self.video_error = error

  def poll(self):
    self.polls += 1

  def input(self, port, device, index, button):
    self.input_calls += 1
    if port or device != DEVICE_JOYPAD or index:
      return 0
    return int(any((self.pad & (1 << bit)) and button == key for bit, key in enumerate(BUTTON_IDS)))

  def run(self, pad):
    self.pad = pad
    self.polls = self.input_calls = self.video_calls = 0
    self.fresh_frame = False
    self.video_error = None
    self.lib.retro_run()
    if self.video_error:
      raise RuntimeError("video callback failed") from self.video_error

  def memory(self, memory_id):
    size = self.lib.retro_get_memory_size(memory_id)
    address = self.lib.retro_get_memory_data(memory_id)
    return ctypes.string_at(address, size) if address and size else b""

  def close(self):
    self.lib.retro_unload_game()
    self.lib.retro_deinit()


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--rom", required=True, type=Path)
  parser.add_argument("--core", required=True, type=Path)
  parser.add_argument("--region", required=True, choices=("ntsc", "pal"))
  parser.add_argument("--output", required=True, type=Path)
  parser.add_argument("--scenario", choices=("candidate", "seed"), default="candidate")
  args = parser.parse_args()
  # Never overwrite an earlier run or another worker's evidence.
  args.output.mkdir(parents=True, exist_ok=False)
  with tempfile.TemporaryDirectory(prefix="core-private-", dir=args.output) as work:
    core = SmsCore(args.core.resolve(), args.rom.resolve(), args.region, Path(work))
    try:
      expected_region = 0 if args.region == "ntsc" else 1
      if core.region != expected_region or not (59 < core.fps < 61 if args.region == "ntsc" else 49 < core.fps < 51):
        raise RuntimeError(f"region/timing not confirmed: region={core.region} fps={core.fps}")
      warmup = []
      for step in range(120):
        core.run(0)
        if args.scenario == "candidate":
          warmup.append({"step": step, "pad": 0, "polls": core.polls,
                         **target_snapshot(core.memory(MEMORY_SYSTEM_RAM))})
      ram = core.memory(MEMORY_SYSTEM_RAM)
      if args.scenario == "candidate" and len(ram) < 0x110:
        raise RuntimeError(f"SMS system RAM unavailable: {len(ram)} bytes")
      vram = core.memory(MEMORY_VIDEO_RAM)
      records = []
      images = (0, 45, 95, 145, 181, 211, 241, 271, 391, 511, 555, 605, 645, 685, 755, 800)
      count = 820 if args.scenario == "candidate" else 2
      for step in range(count):
        pad = schedule(step) if args.scenario == "candidate" else 0
        core.run(pad)
        ram = core.memory(MEMORY_SYSTEM_RAM)
        if core.frame is None or not core.fresh_frame:
          raise RuntimeError(f"no fresh framebuffer at step {step}")
        width, height, rgb = core.frame
        record = {"step": step, "pad": pad, "polls": core.polls, "input_calls": core.input_calls,
                  "video_calls": core.video_calls, "width": width, "height": height,
                  "rgb_sha256": hashlib.sha256(rgb).hexdigest()}
        if args.scenario == "candidate":
          record.update(target_snapshot(ram))
        records.append(record)
        if step in images or (args.scenario == "seed" and step == 1):
          (args.output / f"step-{step:03d}.rgb").write_bytes(rgb)
          (args.output / f"step-{step:03d}.png").write_bytes(png(width, height, rgb))
      if vram:
        (args.output / "vram-final.bin").write_bytes(core.memory(MEMORY_VIDEO_RAM))
      result = {"core": core.core_name, "version": core.core_version, "region": core.region,
                "fps": core.fps, "pixel_format": core.pixel_format, "options_seen": core.options_seen,
                "system_ram_size": len(ram), "video_ram_size": len(vram),
                "core_sha256": hashlib.sha256(args.core.read_bytes()).hexdigest(),
                "rom_sha256": hashlib.sha256(args.rom.read_bytes()).hexdigest(),
                "warmup": warmup, "records": records}
      (args.output / "capture.json").write_text(json.dumps(result, indent=2) + "\n")
      print(f"{args.scenario} {args.region}: {core.core_name} {core.core_version}, fps={core.fps:.6f}, "
            f"RAM={len(ram)}, VRAM={len(vram)}, frames={len(records)} -> {args.output}")
    finally:
      core.close()


if __name__ == "__main__":
  main()
