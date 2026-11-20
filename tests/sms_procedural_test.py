"""Generate and compare candidate SMS procedural data in a private directory."""

from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]


def main():
  with tempfile.TemporaryDirectory(prefix="chipviz-sms-oracle-") as tmp:
    directory = Path(tmp)
    executable = directory / "sms_procedural_oracle"
    subprocess.run([sys.executable, "cores/sms/homebrew/field_assets.py", str(directory)],
                   cwd=ROOT, check=True)
    subprocess.run([*sys.argv[1:], "tests/sms_procedural_oracle.c", "shared/src/control_frame.c",
                    "-o", str(executable)], cwd=ROOT, check=True)
    subprocess.run([str(executable), str(directory / "field-procedural.bin")], cwd=ROOT, check=True)


if __name__ == "__main__":
  main()
