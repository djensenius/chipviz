"""Assemble and inspect SMS control code without running a console emulator."""

import argparse
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
HOME = ROOT / "cores" / "sms" / "homebrew"


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--wla", type=Path, required=True)
  parser.add_argument("--linker", type=Path, required=True)
  parser.add_argument("--workdir", type=Path, required=True, help="existing private scratch directory")
  args = parser.parse_args()
  wla, linker = args.wla.resolve(), args.linker.resolve()

  with tempfile.TemporaryDirectory(prefix="sms-control-", dir=args.workdir) as directory:
    work = Path(directory)
    subprocess.run([sys.executable, "-B", str(HOME / "field_assets.py"), str(work)], check=True)
    (work / "fixture.s").write_text(
        ".MEMORYMAP\n DEFAULTSLOT 0\n SLOTSIZE $8000\n SLOT 0 $0000\n.ENDME\n"
        ".ROMBANKSIZE $8000\n.ROMBANKS 1\n.EMPTYFILL $00\n.BANK 0 SLOT 0\n.ORG 0\n"
        f'.INCLUDE "{HOME / "data_control.asm"}"\n.INCLUDE "{HOME / "data_input.asm"}"\n'
        f'.INCLUDE "{HOME / "spectral_levels.asm"}"\n'
        f'.INCLUDE "{HOME / "safe_modes.asm"}"\n'
        f'.INCLUDE "{HOME / "data_detector.asm"}"\n'
        f'.INCLUDE "{HOME / "detector_fixture.asm"}"\n'
        f'.INCLUDE "{HOME / "field_data.asm"}"\n')
    (work / "fixture.link").write_text("[objects]\nfixture.o\n")
    subprocess.run([str(wla), "-o", "fixture.o", "fixture.s"], cwd=work, check=True)
    subprocess.run([str(linker), "-r", "-S", "fixture.link", "fixture.bin"], cwd=work, check=True)

    image = (work / "fixture.bin").read_bytes()
    labels = {}
    for line in (work / "fixture.sym").read_text().splitlines():
      parts = line.split()
      if len(parts) == 2 and parts[0].startswith("00:"):
        labels[parts[1]] = int(parts[0][3:], 16)
    assert len(image) == 32768
    for label, filename in (
        ("sms_pad_edge_table", "field-pad-edges.bin"),
        ("sms_procedural_table", "field-procedural.bin"),
        ("sms_spectral_initial_table", "field-spectral-initial.bin"),
        ("sms_spectral_next_table", "field-spectral-next.bin"),
        ("sms_cut_action_table", "field-cut-actions.bin"),
        ("sms_field_patterns", "field-patterns.bin"),
        ("sms_field_maps", "field-maps.bin"),
        ("sms_field_cram_levels", "field-cram-levels.bin"),
        ("sms_field_combos", "field-combos.bin"),
    ):
      expected = (work / filename).read_bytes()
      assert image[labels[label]:labels[label] + len(expected)] == expected, label
    for label, opcode in (
        ("sms_sample_pad", "db dc 2f e6 3f"),
        ("sms_unarmed_palette", "47 e6 03 fe 03 78 c0 3d c9"),
        ("sms_wait_vblank", "db bf db bf e6 80 28 fa c9"),
        ("sms_hidden_poll_loop", "f3 e6 bf d3 bf 3e 81 d3 bf"),
        ("sms_wire_count_step", "3a 0f c0 fe 1e"),
        ("sms_scene_check", "fe 1e d8 3a 07 c0 fe 14 d8"),
        ("sms_request_budget", "3a 13 c0 fe 14"),
        ("sms_spectral_update", "3a 14 c0 fe 0a d8"),
        ("sms_field_mode", "3a 0d c0 e6 03 fe 03 c0 3e 02 c9"),
        ("sms_detector_step", "0e 00"),
        ("sms_det_transient", "af 32 8e c0 3a 20 c0 fe 18"),
        ("sms_fixture_step", "3a b0 c0 32 03 c0 3a b1 c0 e6 07 32 04 c0"),
    ):
      expected = bytes.fromhex(opcode)
      assert image[labels[label]:labels[label] + len(expected)] == expected, label
    budget = image[labels["sms_request_budget"]:labels["sms_scene_advance"]]
    assert bytes.fromhex("3a 14 c0 fe 0a") in budget
    pad_input = image[labels["sms_input_pad"]:labels["sms_input_procedural"]]
    assert bytes.fromhex("3a 01 c0 e6 30 0f 0f 0f 0f 32 2c c0") in pad_input
    loop = image[labels["sms_hidden_poll_next"]:labels["sms_pad_edge_table"]]
    offsets = []
    for label in ("sms_wait_vblank", "sms_sample_pad", "sms_make_input", "sms_begin_step",
                  "sms_detector_entry", "sms_field_scene_entry", "sms_detector_step",
                  "sms_field_step", "sms_field_make_trace", "sms_finish_step",
                  "sms_field_mode_entry", "sms_ack_mode_changed"):
      address = labels[label]
      offsets.append(loop.index(bytes((0xCD, address & 0xFF, address >> 8))))
    assert offsets == sorted(offsets)
    print("PASS: Z80 fixture links; hidden call order, key opcodes and nine embedded assets verified")


if __name__ == "__main__":
  main()
