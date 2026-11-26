"""Observe every SMS controller-port read in the pinned Genesis Plus GX core.

The host-only function-pointer shim changes emulator execution, not ROM bytes.
Snapshots are after the preceding step's finish, not pre-finish breakpoints.
This is not VBlank, instruction-cycle or physical timing evidence.
"""

import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import tempfile

import sms_capture_oracle as oracle
import sms_libretro_capture as capture


CORE_HASH = "27bc2ef8ca07c775f022ae90be381b2a2a500245c9c7e362356ba3077b25c268"
ROM_HASH = "736d1ed5f4b088936f211cbdff9696cb2f798015ae672c5579152fdfecfc6ef6"
POLL_PC = 0x02e4  # PC following IN A,($DC) in the pinned candidate.
OBSERVE = ctypes.CFUNCTYPE(None, ctypes.c_uint, ctypes.c_uint)


class FastCore(capture.SmsCore):
  def video(self, data, width, height, pitch):
    self.video_calls += 1


class PollObserver:
  def __init__(self, core, shim, pads):
    self.core, self.shim, self.pads = core, shim, pads
    self.events, self.records, self.error = [], [], None
    self.frame = 0
    self.slot = ctypes.c_void_p.in_dll(core.lib, "z80_readport")
    self.original = self.slot.value
    if self.original != ctypes.cast(core.lib.z80_ms_port_r, ctypes.c_void_p).value:
      raise RuntimeError("unexpected installed GX port-read function")
    self.pc = ctypes.c_uint16.in_dll(core.lib, "Z80")
    self.ram = (ctypes.c_ubyte * 8192).in_dll(core.lib, "work_ram")
    self.callback = OBSERVE(self.observe)
    shim.sms_hook_configure.argtypes = [ctypes.c_void_p, OBSERVE]
    shim.sms_hook_configure(self.original, self.callback)
    self.slot.value = ctypes.cast(shim.sms_hook_read, ctypes.c_void_p).value

  def observe(self, port, value):
    try:
      index = len(self.events)
      if self.pc.value != POLL_PC:
        raise ValueError(f"poll {index}: unexpected Z80 PC ${self.pc.value:04x}")
      pad = (~value) & 0x3f
      wanted = self.pads[index] if index < len(self.pads) else 0
      if pad != wanted:
        raise ValueError(f"poll {index}: sampled pad {pad:#x} != scheduled {wanted:#x}")
      if index:
        snapshot = capture.target_snapshot(bytes(self.ram))
        if snapshot["buttons"] != self.events[-1]["pad"]:
          raise ValueError(f"poll {index}: prior completed step has wrong buttons")
        self.records.append({"step": index - 1, **snapshot})
      self.events.append({"poll": index, "port": port, "pc": self.pc.value,
                          "pad": pad, "frame": self.frame})
    except Exception as error:
      self.error = error

  def close(self):
    self.slot.value = self.original


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--rom", type=Path, required=True)
  parser.add_argument("--core", type=Path, required=True)
  parser.add_argument("--shim", type=Path, required=True)
  parser.add_argument("--region", choices=("ntsc", "pal"), required=True)
  parser.add_argument("--output", type=Path, required=True)
  args = parser.parse_args()
  if hashlib.sha256(args.rom.read_bytes()).hexdigest() != ROM_HASH:
    raise ValueError("unexpected ROM hash")
  if hashlib.sha256(args.core.read_bytes()).hexdigest() != CORE_HASH:
    raise ValueError("unexpected core hash")
  if args.output.exists():
    raise ValueError("refusing to overwrite an existing capture")
  pads = oracle.schedule()
  with tempfile.TemporaryDirectory(prefix="sms-gx-private-", dir=args.output.parent) as work:
    core = FastCore(args.core.resolve(), args.rom.resolve(), args.region, Path(work))
    hook = None
    try:
      if core.region != (0 if args.region == "ntsc" else 1):
        raise RuntimeError("requested region not selected")
      hook = PollObserver(core, ctypes.CDLL(str(args.shim.resolve())), pads)
      # One extra poll observes the last requested step after its finish.
      for frame in range(1200):
        hook.frame = frame
        before = len(hook.events)
        pad = pads[before] if before < len(pads) else 0
        core.run(pad)
        if hook.error:
          raise RuntimeError("poll observation failed") from hook.error
        if len(hook.events) - before > 1:
          raise RuntimeError(f"frame {frame}: more than one Z80 poll")
        if len(hook.events) == len(pads) + 1:
          break
      else:
        raise RuntimeError("poll capture did not complete")
      result = {"kind": "sms-gx-port-v1", "start": "reset", "region": core.region,
                "fps": core.fps, "core": core.core_name, "version": core.core_version,
                "core_sha256": CORE_HASH, "rom_sha256": ROM_HASH,
                "events": hook.events, "records": hook.records}
      args.output.write_text(json.dumps(result, indent=2) + "\n")
      print(f"{args.region}: {len(hook.events)} actual port reads, {len(hook.records)} completed steps, "
            f"{core.fps:.6f} Hz -> {args.output}")
    finally:
      if hook is not None:
        hook.close()
      core.close()


if __name__ == "__main__":
  main()
