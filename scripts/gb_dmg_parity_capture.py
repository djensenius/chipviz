#!/usr/bin/env python3
"""Test-only SameBoy fixture capture. Binary RGB frames go to stdout."""

import hashlib
import pathlib
import subprocess
import sys
import tempfile

import gb_libretro_capture as glc


ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "cores" / "gb" / "homebrew"
# Each number is the number of acknowledged native steps, not retro_run calls.
STEPS = (1, 2, 15, 16, 17, 31, 32, 33, 47, 48, 49, 63, 64, 65, 127, 128, 255, 256, 257)
WIDTH, HEIGHT = 160, 144
FRAME_BYTES = WIDTH * HEIGHT * 3
RECORD_BYTES = 2 + FRAME_BYTES


def registers(step):
  counter = step & 255
  scx = (counter >> 1) | ((counter & 1) << 7)
  return scx, counter, 0xE4 | (scx & 0x18)


def symbols(path):
  required = {
    "wReady", "wFxSeq", "wAckSeq", "wLastSeq", "wStep", "wFrame",
    "wFxCtl", "wFxScene", "wFxPin", "wTraceShow", "wShownShow",
    "wShSCX", "wShSCY", "wShBGP", "wShLCDC", "wShown7", "wShowMode",
  }
  result = {}
  for line in path.read_text().splitlines():
    parts = line.split()
    if len(parts) != 2 or parts[1] not in required:
      continue
    bank, address = parts[0].split(":")
    if bank != "00" or not 0xC000 <= int(address, 16) < 0xE000 or parts[1] in result:
      raise ValueError(f"invalid or duplicate WRAM symbol: {line}")
    result[parts[1]] = int(address, 16)
  if result.keys() != required:
    raise ValueError(f"missing symbols: {sorted(required - result.keys())}")
  return result


def fresh_frame(callbacks):
  """Require exactly one real, complete callback from this retro_run."""
  if len(callbacks) != 1 or callbacks[0] is None:
    raise ValueError("missing, duplicate, or null video callback")
  frame = callbacks[0]
  if (frame.width, frame.height) != (WIDTH, HEIGHT) or len(frame.rgb) != FRAME_BYTES:
    raise ValueError("malformed native video frame")
  return frame.rgb


class CaptureCore(glc.GbCore):
  def __init__(self, *args):
    self.callbacks = []
    super().__init__(*args)

  def _video(self, data, width, height, pitch):
    # Do not let a callback error escape through ctypes, which would swallow it.
    if (not data or (width, height) != (WIDTH, HEIGHT)
        or self.format not in (glc.PIXEL_XRGB8888, glc.PIXEL_RGB565, glc.PIXEL_0RGB1555)
        or pitch < width * (4 if self.format == glc.PIXEL_XRGB8888 else 2)):
      self.callbacks.append(None)
      return
    try:
      super()._video(data, width, height, pitch)
      self.callbacks.append(self.frame)
    except Exception:
      self.callbacks.append(None)

  def capture_run(self):
    self.callbacks = []
    self.frame = None
    self.run_frame()
    return fresh_frame(self.callbacks)


def capture(rom, core, sym):
  gb = CaptureCore(rom, core)

  def read(name, size=1):
    return gb.read(sym[name], size)

  def byte(name):
    return read(name)[0]

  def write(name, value):
    gb.write(sym[name], bytes([value]))

  def state(step):
    expected = registers(step)
    actual = (byte("wShSCX"), byte("wShSCY"), byte("wShBGP"))
    if (int.from_bytes(read("wStep", 2), "little") != step or byte("wFrame") != step % 256
        or actual != expected or byte("wShLCDC") != 0x91 or byte("wTraceShow") != 0
        or byte("wShownShow") != 0 or byte("wShown7") != 0 or byte("wShowMode") != 0
        or byte("wFxSeq") != step % 256 or byte("wAckSeq") != step % 256
        or byte("wLastSeq") != step % 256):
      raise AssertionError(f"unexpected native state at step {step}: registers={actual!r}")

  try:
    gb.reset()
    write("wReady", 0)  # Reset WRAM can contain the ready marker.
    for _ in range(600):
      gb.run_frame()
      if byte("wReady") == 0xA5:
        break
    else:
      raise AssertionError("fixture did not become ready")
    write("wFxCtl", 0)
    write("wFxScene", 0)
    write("wFxPin", 0xFF)
    if byte("wFxSeq") != 0 or byte("wAckSeq") != 0 or byte("wFrame") != 0:
      raise AssertionError("unexpected fixture initial state")
    selected = set(STEPS)
    frames = []
    records = []
    for step in range(1, max(STEPS) + 1):
      write("wFxSeq", step % 256)
      for _ in range(4):
        gb.run_frame()
        if byte("wAckSeq") == step % 256:
          break
      else:
        raise AssertionError(f"step {step} not acknowledged")
      state(step)
      if step in selected:
        # Ack follows Step, not ApplyDisplay. Hold the mailbox unchanged:
        # after multiple whole retro_run calls the new shadows have been
        # applied and two *fresh* completed LCD frames must agree.
        gb.capture_run()
        first = gb.capture_run()
        second = gb.capture_run()
        state(step)
        if first != second:
          raise AssertionError(f"frozen frames differ at step {step}")
        if frames and frames[-1] == second:
          raise AssertionError(f"consecutive selected states have identical frames at step {step}")
        frames.append(second)
        # state() has checked this actual 16-bit native step and the mailbox
        # acknowledgement after the freeze; do not label it from the loop.
        records.append(read("wStep", 2) + second)
    return b"".join(records)
  finally:
    gb.close()


def main():
  core = glc.default_core().resolve(strict=True)
  with tempfile.TemporaryDirectory(prefix="chipviz-dmg-parity-") as directory:
    tmp = pathlib.Path(directory)
    obj, rom, sym = (tmp / "main.o", tmp / "fixture.gb", tmp / "fixture.sym")
    for command in (
      ["rgbasm", "-D", "CHIPVIZ_FIXTURE", "-o", str(obj), "main.asm"],
      ["rgblink", "-n", str(sym), "-o", str(rom), str(obj)],
      ["rgbfix", "-v", "-p", "0", "-t", "CHIPVIZGB", str(rom)],
    ):
      subprocess.run(command, cwd=SOURCE, check=True, stdout=subprocess.PIPE)
    for label, path in (("source", SOURCE / "main.asm"), ("included source", SOURCE / "colour_field.asm"),
                        ("ROM", rom), ("core", core)):
      print(f"{label}: {path} sha256={hashlib.sha256(path.read_bytes()).hexdigest()}", file=sys.stderr)
    data = capture(rom, core, symbols(sym))
    if len(data) != len(STEPS) * RECORD_BYTES:
      raise AssertionError("incomplete capture")
    sys.stdout.buffer.write(data)


if __name__ == "__main__":
  main()
