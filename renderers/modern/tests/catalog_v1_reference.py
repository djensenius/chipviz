"""Development-only oracle for the Rust catalog-v1 differential test."""

import pathlib
import struct
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "host" / "bridge"))
import scene_catalog_v1 as catalog


def main() -> None:
  source = sys.stdin.buffer
  while size_bytes := source.read(8):
    if len(size_bytes) != 8:
      raise ValueError("truncated test vector length")
    size = struct.unpack("<Q", size_bytes)[0]
    data = source.read(size)
    if len(data) != size:
      raise ValueError("truncated test vector")
    with tempfile.TemporaryFile() as file:
      file.write(data)
      file.flush()
      try:
        with catalog.open_catalog(file.fileno()):
          result = "valid"
      except catalog.UnavailableSceneError:
        result = "unavailable"
      except ValueError:
        result = "invalid"
    print(result)


if __name__ == "__main__":
  main()
