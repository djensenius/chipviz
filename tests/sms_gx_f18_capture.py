"""Capture F18 callable fixture execution from a test-only SMS ROM."""

import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import tempfile

import sms_gx_poll_capture as gx
import sms_trace_tool as trace


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--rom", type=Path, required=True)
  parser.add_argument("--symbols", type=Path, required=True)
  parser.add_argument("--core", type=Path, required=True)
  parser.add_argument("--shim", type=Path, required=True)
  parser.add_argument("--oracle", type=Path, required=True)
  parser.add_argument("--region", choices=("ntsc", "pal"), required=True)
  parser.add_argument("--output", type=Path, required=True)
  args = parser.parse_args()
  if hashlib.sha256(args.core.read_bytes()).hexdigest() != gx.CORE_HASH:
    raise ValueError("unexpected GX core hash")
  labels = {}
  for line in args.symbols.read_text().splitlines():
    parts = line.split()
    if len(parts) == 2 and parts[0].startswith("00:"):
      labels[parts[1]] = int(parts[0][3:], 16)
  pc = labels["f18_emit"] + 2
  rom = args.rom.read_bytes()
  if rom[pc - 2:pc] != bytes.fromhex("db dc"):
    raise ValueError("fixture marker is not IN A,($DC)")
  rows = trace.inputs("f18")
  expected = trace.oracle("f18", rows, args.oracle.resolve())
  args.output.mkdir(parents=True, exist_ok=False)
  with tempfile.TemporaryDirectory(prefix="f18-core-", dir=args.output) as work:
    core = gx.FastCore(args.core.resolve(), args.rom.resolve(), args.region, Path(work))
    slot = ctypes.c_void_p.in_dll(core.lib, "z80_readport")
    original = slot.value
    try:
      if core.region != (0 if args.region == "ntsc" else 1):
        raise RuntimeError("requested region not selected")
      if original != ctypes.cast(core.lib.z80_ms_port_r, ctypes.c_void_p).value:
        raise RuntimeError("unexpected installed GX port-read function")
      z80_pc = ctypes.c_uint16.in_dll(core.lib, "Z80")
      ram = (ctypes.c_ubyte * 8192).in_dll(core.lib, "work_ram")
      records, errors = [], []

      def observe(port, value):
        del port, value
        try:
          step = len(records)
          if step >= 600 or z80_pc.value != pc or int.from_bytes(bytes(ram[0x110:0x112]), "little") != step:
            raise ValueError(f"invalid F18 marker at step {step}, PC ${z80_pc.value:04x}")
          actual_input = bytes(ram[0xb0:0xb2]) + bytes(ram[0xa0:0xb0])
          if actual_input != rows[step * 18:(step + 1) * 18]:
            raise ValueError(f"F18 input differs at step {step}")
          records.append(bytes(ram[0xc0:0xd0]))
        except Exception as error:
          errors.append(str(error))

      callback = gx.OBSERVE(observe)
      shim = ctypes.CDLL(str(args.shim.resolve()))
      shim.sms_hook_configure.argtypes = [ctypes.c_void_p, gx.OBSERVE]
      shim.sms_hook_configure(original, callback)
      slot.value = ctypes.cast(shim.sms_hook_read, ctypes.c_void_p).value
      for _ in range(800):
        core.run(0)
        if errors:
          raise RuntimeError(errors[0])
        if len(records) == 600:
          break
      else:
        raise RuntimeError(f"F18 capture incomplete: {len(records)} steps")
      actual = b"".join(records)
      (args.output / "f18.capture.bin").write_bytes(actual)
      metadata = {"rom_sha256": hashlib.sha256(rom).hexdigest(), "core_sha256": gx.CORE_HASH,
                  "region": core.region, "fps": core.fps, "marker_pc": pc, "steps": len(records)}
      (args.output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
      trace.compare_capture("f18", expected, actual)
      print(f"PASS: {args.region} 600 target F18 fixture traces and events match shared oracle -> {args.output}")
    finally:
      slot.value = original
      core.close()


if __name__ == "__main__":
  main()
