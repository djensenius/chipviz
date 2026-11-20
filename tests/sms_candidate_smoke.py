"""Build and inspect the candidate ROM; this does not execute Z80 instructions."""

import argparse
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
HOME = ROOT / "cores/sms/homebrew"
SPEC = importlib.util.spec_from_file_location("sms_assets", HOME / "field_assets.py")
assets = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(assets)


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--wla", required=True)
  parser.add_argument("--linker", required=True)
  parser.add_argument("--workdir", required=True, type=Path)
  args = parser.parse_args()
  with tempfile.TemporaryDirectory(prefix="sms-rom-smoke-", dir=args.workdir) as directory:
    work = Path(directory)
    rom_path, sym_path = work / "candidate.sms", work / "candidate.sym"
    subprocess.run([sys.executable, "-B", str(HOME / "build_candidate.py"),
                    "--wla", args.wla, "--linker", args.linker,
                    "--output", str(rom_path), "--symbols", str(sym_path)], check=True)
    rom = rom_path.read_bytes()
    labels = {}
    for line in sym_path.read_text().splitlines():
      parts = line.split()
      if len(parts) == 2 and parts[0].startswith("00:"):
        labels[parts[1]] = int(parts[0][3:], 16)
    assert len(rom) == 0x8000
    assert rom[0x7ff0:0x7ff8] == b"TMR SEGA" and rom[0x7fff] == 0x4c
    assert int.from_bytes(rom[0x7ffa:0x7ffc], "little") == sum(rom[:0x7ff0]) & 0xffff
    assert rom[:4] == bytes((0xf3, 0xc3, labels["sms_reset"] & 255, labels["sms_reset"] >> 8))
    for label, blob in (("sms_field_patterns", assets.patterns()),
                        ("sms_field_maps", b"".join(assets.name_map(i) for i in range(4))),
                        ("sms_field_cram_levels", assets.cram_levels()),
                        ("sms_field_combos", assets.combos())):
      start = labels[label]
      assert rom[start:start + len(blob)] == blob, label
    assert rom[labels["sms_vdp_registers"]:labels["sms_vdp_registers"] + 11] == bytes.fromhex(
        "04 80 f7 ff ff 3e 00 00 00 00 ff")
    reset = rom[labels["sms_reset"]:labels["sms_main_loop"]]
    for data in ("31 f0 df af 32 fc ff 32 fd ff 3c 32 fe ff 3c 32 ff ff",
                 "11 00 58", "01 80 03", "11 00 5f", "3e d0 d3 be",
                 "11 00 60 01 00 20"):
      assert bytes.fromhex(data) in reset, data
    publish = rom[labels["sms_main_publish"]:labels["sms_publish_colours"]]
    assert publish == (b"\xcd" + labels["sms_wait_vblank"].to_bytes(2, "little")
                       + bytes.fromhex("3e 01 d3 bf 3e c0 d3 bf 21 01 c1 06 03"))
    assert rom[labels["sms_publish_colours"]:labels["sms_publish_map"]] == bytes.fromhex(
        "7e d3 be 23 10 fa")
    assert rom[labels["sms_publish_map"]:labels["sms_copy_vram"]] == bytes.fromhex(
        "3a 00 c1 d3 bf 3e 82 d3 bf c3") + labels["sms_main_loop"].to_bytes(2, "little")
    loop = rom[labels["sms_main_loop"]:labels["sms_copy_vram"]]
    offsets = []
    for label in ("sms_sample_pad", "sms_make_input", "sms_begin_step", "sms_detector_entry",
                  "sms_field_scene_entry", "sms_detector_step", "sms_field_step",
                  "sms_field_make_trace", "sms_prepare_field", "sms_finish_step",
                  "sms_field_mode_entry", "sms_ack_mode_changed", "sms_wait_vblank"):
      offsets.append(loop.index(b"\xcd" + labels[label].to_bytes(2, "little")))
    assert offsets == sorted(offsets)
    print("PASS: candidate links; header, embedded assets, startup and publication opcodes checked (not executed)")


if __name__ == "__main__":
  main()
