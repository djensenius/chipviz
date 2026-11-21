import json
import os
import pathlib
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
BRIDGE = ROOT / "host" / "bridge"
sys.path.insert(0, str(BRIDGE))

import catalog_music_source as producer
import scene_catalog_v1 as catalog


class CatalogMusicSourceTests(unittest.TestCase):
  def setUp(self) -> None:
    self.directory = tempfile.TemporaryDirectory()
    self.addCleanup(self.directory.cleanup)
    self.root = pathlib.Path(self.directory.name)
    self.source = self.root / "source.json"
    self.output = self.root / "output.cvxe"
    self.fixture = ROOT / "host" / "fixtures" / "musical" / "scale.musicsource.json"
    self.source.write_bytes(self.fixture.read_bytes())

  def test_fixtures_round_trip_preserves_legacy_fields(self) -> None:
    for name in ("scale", "groove"):
      fixture = ROOT / "host" / "fixtures" / "musical" / f"{name}.musicsource.json"
      source = fixture.read_bytes()
      legacy = fixture.with_name(f"{name}.cvz").read_bytes()
      count = len(legacy) // 33
      for family in (0, 2, 5):
        with self.subTest(fixture=name, family=family):
          data = producer.encode_catalog(source, family)
          self.assertEqual(data, producer.encode_catalog(source, family))
          self.assertEqual(len(data), 20 + 145 * count)
          self.assertEqual(data[:8], b"CVXE\x01\x01\x00\x00")
          self.assertEqual(struct.unpack_from("<Q", data, 8)[0], count)
          self.assertEqual(struct.unpack_from("<I", data, 16)[0], zlib.crc32(data[:16]))
          self.output.write_bytes(data)
          with self.output.open("rb") as f, catalog.open_catalog(f.fileno()) as result:
            self.assertEqual(result.count, count)
            for n in range(count):
              record = result.record(n)
              wire = record.envelope
              frame = wire[24:57]
              original = legacy[n * 33:(n + 1) * 33]
              self.assertEqual(wire[:8], b"CVX1\x01\x01\x00\x00")
              self.assertEqual(struct.unpack_from("<QQ", wire, 8), (n, n))
              self.assertEqual(struct.unpack_from("<H", frame, 2)[0], n % 65536)
              self.assertEqual(frame[:20], original[:20])
              self.assertEqual(frame[20], family)
              self.assertEqual(frame[21:32], original[21:32])
              checksum = 0
              for byte in frame:
                checksum ^= byte
              self.assertEqual(checksum, 0)
              self.assertEqual(wire[57:141], bytes(84))
              self.assertEqual(struct.unpack_from("<I", wire, 141)[0], zlib.crc32(wire[:141]))

  def test_legacy_scene_is_overridden_not_interpreted(self) -> None:
    source = json.loads(self.source.read_bytes())
    source["scene"] = 7
    source["palette"] = 3
    data = producer.encode_catalog(json.dumps(source).encode(), 1)
    self.assertEqual(data[20 + 24 + 20:20 + 24 + 22], bytes((1, 3)))

  def test_rejects_pairs_and_invalid_sources_before_output(self) -> None:
    original = self.source.read_bytes()
    for family, variant in ((6, 0), (7, 0), (8, 0), (255, 0), (-1, 0),
                            (2, 1), (2, 2), (0, 1), (True, 0), (0, False)):
      with self.subTest(family=family, variant=variant):
        with self.assertRaisesRegex(ValueError, "family 0..5, variant 0"):
          producer.write_catalog(self.source, self.output, family, variant)
        self.assertFalse(self.output.exists())
    source = json.loads(original)
    invalid = [
      b"{", b"\xff", b"[]", b'{"value": NaN}',
      json.dumps({**source, "frame_rate": 59}).encode(),
      json.dumps({**source, "frame_rate": True}).encode(),
      json.dumps({k: v for k, v in source.items() if k != "frame_rate"}).encode(),
      json.dumps({**source, "frame_count": 65537}).encode(),
      json.dumps({**source, "notes": "bad"}).encode(),
      json.dumps({**source, "notes": [{"start": 0}]}).encode(),
      json.dumps({**source, "scene": 256}).encode(),
      json.dumps({**source, "notes": [source["notes"][0]] * 5}).encode(),
      b" " * (producer.MAX_SOURCE_BYTES + 1),
      json.dumps({**source, "frame_count": 65536,
                  "notes": [source["notes"][0]] * 16}).encode(),
    ]
    for data in invalid:
      with self.subTest(data=data[:100]):
        self.source.write_bytes(data)
        with self.assertRaises(ValueError):
          producer.write_catalog(self.source, self.output, 0)
        self.assertFalse(self.output.exists())
        self.assertEqual(self.source.read_bytes(), data)

  def test_work_bound_and_complete_validation_precede_creation(self) -> None:
    source = json.loads(self.source.read_bytes())
    source["frame_count"] = 1001
    source["notes"] = [source["notes"][0]] * 1000
    self.source.write_text(json.dumps(source))
    with mock.patch.object(producer.chipviz_encode, "encode_music_source") as encoder:
      with self.assertRaisesRegex(ValueError, "frame_count \\* note_count"):
        producer.write_catalog(self.source, self.output, 0)
      encoder.assert_not_called()
    self.source.write_bytes(self.fixture.read_bytes())
    with mock.patch.object(producer.scene_catalog_v1, "open_catalog",
                           side_effect=ValueError("injected invalid recording")):
      with self.assertRaisesRegex(ValueError, "injected invalid recording"):
        producer.write_catalog(self.source, self.output, 0)
    self.assertFalse(self.output.exists())
    with mock.patch.object(producer.chipviz_encode, "encode_music_source",
                           return_value=bytes(33 * 96)):
      with self.assertRaisesRegex(ValueError, "invalid frame magic"):
        producer.write_catalog(self.source, self.output, 0)
    self.assertFalse(self.output.exists())

  def test_frame_and_work_boundaries(self) -> None:
    source = {"version": 0, "bpm": 120, "frame_rate": 60,
              "frame_count": 65536, "notes": []}
    data = producer.encode_catalog(json.dumps(source).encode(), 0)
    self.assertEqual(len(data), 20 + 145 * 65536)
    last = data[-145:]
    self.assertEqual(struct.unpack_from("<QQ", last, 8), (65535, 65535))
    self.assertEqual(struct.unpack_from("<H", last, 26)[0], 65535)
    source["frame_count"] = 1000
    source["notes"] = [
      {"start": n, "duration": 1, "note": 60, "velocity": 100}
      for n in range(1000)
    ]
    data = producer.encode_catalog(json.dumps(source).encode(), 0)
    self.assertEqual(len(data), 20 + 145 * 1000)

  def test_existing_destination_and_aliases_are_never_modified(self) -> None:
    original = self.source.read_bytes()
    self.output.write_bytes(b"existing")
    hardlink = self.root / "hardlink"
    symlink = self.root / "symlink"
    os.link(self.source, hardlink)
    symlink.symlink_to(self.source)
    dangling = self.root / "dangling"
    dangling.symlink_to(self.root / "missing")
    for target in (self.output, self.source, hardlink, symlink, dangling):
      with self.subTest(target=target):
        with self.assertRaises(FileExistsError):
          producer.write_catalog(self.source, target, 3)
        self.assertEqual(self.source.read_bytes(), original)
        self.assertEqual(self.output.read_bytes(), b"existing")
    self.assertFalse((self.root / "missing").exists())

  def test_regular_input_fifo_and_permissions(self) -> None:
    fifo = self.root / "fifo"
    os.mkfifo(fifo)
    result = subprocess.run(
      [sys.executable, str(BRIDGE / "catalog_music_source.py"),
       "--music", str(fifo), "--family", "0", "--output", str(self.output)],
      capture_output=True, text=True, timeout=5, check=False,
    )
    self.assertEqual(result.returncode, 1)
    self.assertIn("source must be a regular file", result.stderr)
    with self.assertRaisesRegex(ValueError, "regular file"):
      producer.write_catalog(self.root, self.output, 0)
    self.assertFalse(self.output.exists())
    producer.write_catalog(self.source, self.output, 4)
    self.assertEqual(self.output.stat().st_mode & 0o077, 0)
    self.assertEqual(self.source.read_bytes(), self.fixture.read_bytes())

  def test_cli_success_and_error(self) -> None:
    command = [sys.executable, str(BRIDGE / "catalog_music_source.py"),
               "--music", str(self.source), "--output", str(self.output),
               "--family"]
    result = subprocess.run(command + ["5"], capture_output=True, text=True, check=False)
    self.assertEqual(result.returncode, 0, result.stderr)
    with self.output.open("rb") as f, catalog.open_catalog(f.fileno()) as result_catalog:
      self.assertEqual(result_catalog.record(0).family, 5)
    result = subprocess.run(command + ["7"], capture_output=True, text=True, check=False)
    self.assertEqual(result.returncode, 1)
    self.assertIn("family 0..5, variant 0", result.stderr)


if __name__ == "__main__":
  unittest.main()
