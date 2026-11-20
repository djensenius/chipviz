import io
import pathlib
import struct
import sys
import tempfile
import unittest
import zlib
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "host" / "bridge"))
sys.path.insert(0, str(ROOT / "shared" / "tools"))

import chipviz_encode
import cvz_to_c
import scene_catalog_v1 as catalog


PAYLOAD = bytes.fromhex(
  "c7 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 "
  "00 00 00 00 07 02 00 00 00 00 00 00 00 00 00 00 c2"
)
GOLDEN_RECORD = (
  bytes.fromhex("43 56 58 31 01 01 00 00") + bytes(16) + PAYLOAD
  + bytes(84) + bytes.fromhex("c3 cc bb 8b")
)
GOLDEN_HEADER = bytes.fromhex(
  "43 56 58 45 01 01 00 00 01 00 00 00 00 00 00 00 bf 5d 1f 65"
)


def crc(data: bytes | bytearray) -> bytes:
  return struct.pack("<I", zlib.crc32(data))


def header(count: int) -> bytes:
  data = b"CVXE\x01\x01\x00\x00" + struct.pack("<Q", count)
  return data + crc(data)


def record(sequence: int = 0, family: int = 7, variant: int = 0,
           metadata: bytes = bytes(84)) -> bytes:
  data = bytearray(GOLDEN_RECORD)
  data[6] = variant
  struct.pack_into("<QQ", data, 8, sequence, sequence)
  struct.pack_into("<H", data, 26, sequence % 65536)
  data[44] = family
  data[57:141] = metadata
  fix_frame(data)
  return fix_crc(data)


def fix_frame(data: bytearray) -> None:
  checksum = 0
  for byte in data[24:56]:
    checksum ^= byte
  data[56] = checksum


def fix_crc(data: bytearray) -> bytes:
  data[141:145] = crc(data[:141])
  return bytes(data)


def star_metadata(epoch: int = 946_684_800_000, latitude: int = 0,
                  longitude: int = 0, elevation: int = 0) -> bytes:
  return struct.pack("<32sqiii32s", b"a" * 32, epoch, latitude, longitude,
                     elevation, b"p" * 32)


class CatalogTests(unittest.TestCase):
  def setUp(self) -> None:
    self.directory = tempfile.TemporaryDirectory()
    self.addCleanup(self.directory.cleanup)
    self.path = pathlib.Path(self.directory.name) / "input.cvxe"

  def open_bytes(self, data: bytes) -> catalog.Catalog:
    self.path.write_bytes(data)
    return catalog.open_catalog(self.path)

  def assert_invalid(self, data: bytes, message: str = "") -> None:
    with self.assertRaises(ValueError, msg=message):
      self.open_bytes(data)

  def test_normative_golden_and_crc(self) -> None:
    self.assertEqual(zlib.crc32(b"123456789"), 0xCBF43926)
    self.assertEqual(record(), GOLDEN_RECORD)
    self.assertEqual(header(1), GOLDEN_HEADER)
    with self.open_bytes(GOLDEN_HEADER + GOLDEN_RECORD) as result:
      self.assertEqual(result.header, GOLDEN_HEADER)
      self.assertEqual(result.count, 1)
      item = result.record(0)
      self.assertEqual(item.envelope, GOLDEN_RECORD)
      self.assertEqual((item.sequence, item.tick, item.family, item.variant), (0, 0, 7, 0))
      self.assertEqual(item.metadata, bytes(84))
      self.assertEqual(item.renderer_availability, "unassessed")
      with self.assertRaises(IndexError):
        result.record(1)
      with self.assertRaises(IndexError):
        result.record(-1)
    with self.assertRaises(ValueError):
      result.record(0)

  def test_normative_record_vectors(self) -> None:
    vectors = [
      (2, 0, "52 55 2f d6", True),
      (2, 2, "5f 1c 03 e5", True),
      (2, 3, "f9 bb 2d 11", False),
      (14, 0, "51 75 ce b2", False),
    ]
    for family, variant, expected_crc, valid in vectors:
      wire = record(family=family, variant=variant)
      self.assertEqual(wire[-4:], bytes.fromhex(expected_crc))
      if valid:
        with self.open_bytes(header(1) + wire) as result:
          self.assertEqual((result.record(0).family, result.record(0).variant),
                           (family, variant))
      else:
        self.assert_invalid(header(1) + wire)

    for offset, value, expected_crc in (
      (46, 0x20, "e9 57 aa 5a"),
      (47, 5, "b5 99 9f 81"),
    ):
      wire = bytearray(GOLDEN_RECORD)
      wire[offset] = value
      fix_frame(wire)
      wire = fix_crc(wire)
      self.assertEqual(wire[-4:], bytes.fromhex(expected_crc))
      self.assert_invalid(header(1) + wire)
    wire = bytearray(GOLDEN_RECORD)
    wire[56] ^= 1
    wire = fix_crc(wire)
    self.assertEqual(wire[-4:], bytes.fromhex("42 10 3a 80"))
    self.assert_invalid(header(1) + wire)

  def test_all_family_variant_pairs(self) -> None:
    for family in range(14):
      limit = 2 if family == 2 else 3 if family == 11 else 0
      metadata = star_metadata() if family == 6 else bytes(84)
      for variant in range(limit + 1):
        with self.subTest(family=family, variant=variant):
          with self.open_bytes(header(1) + record(family=family, variant=variant,
                                                  metadata=metadata)) as result:
            item = result.record(0)
            self.assertEqual((item.family, item.variant), (family, variant))
            self.assertEqual(item.metadata, metadata)
            self.assertEqual(item.renderer_availability,
                             "unavailable" if family == 6 else "unassessed")
      self.assert_invalid(header(1) + record(family=family, variant=limit + 1,
                                              metadata=metadata))

  def test_header_and_length_errors(self) -> None:
    file = GOLDEN_HEADER + GOLDEN_RECORD
    for size in (0, 1, 19, 20, 33, 164):
      self.assert_invalid(file[:size], str(size))
    for size in (0, 33, 144):
      self.assert_invalid(GOLDEN_HEADER + GOLDEN_RECORD[:size], str(size))
    for suffix in (b"\x00", bytes(145)):
      self.assert_invalid(file + suffix)
    for offset in (0, 4, 5, 6, 7, 16):
      bad = bytearray(file)
      bad[offset] ^= 1
      if offset < 8:
        bad[16:20] = crc(bad[:16])
      self.assert_invalid(bytes(bad), str(offset))
    for count in (0, 2, 1_000_001, 2**64 - 1):
      self.assert_invalid(header(count) + GOLDEN_RECORD, str(count))
    # A maximum-count header is accepted as a bound, not an allocation request.
    self.assert_invalid(header(1_000_000))

  def test_inner_outer_and_metadata_validation(self) -> None:
    for offset, value, repair_inner in (
      (0, 0, False), (4, 2, False), (5, 2, False), (7, 1, False),
      (24, 0, True), (25, 1, True), (26, 1, True), (46, 0x80, True),
      (47, 5, True), (56, 0, False), (57, 1, False), (141, 0, False),
    ):
      bad = bytearray(GOLDEN_RECORD)
      bad[offset] = value
      if repair_inner:
        fix_frame(bad)
      if offset != 141:
        bad = bytearray(fix_crc(bad))
      self.assert_invalid(header(1) + bad, str(offset))

    for note in (0, 3):
      bad = bytearray(GOLDEN_RECORD)
      bad[47] = note + 1
      bad[48 + 2 * note] = 128
      fix_frame(bad)
      self.assert_invalid(header(1) + fix_crc(bad))
    # Inactive note slots are not constrained.
    inactive = bytearray(GOLDEN_RECORD)
    inactive[54] = 255
    fix_frame(inactive)
    with self.open_bytes(header(1) + fix_crc(inactive)):
      pass

  def test_order_and_later_errors_are_atomic(self) -> None:
    good = record()
    corrupt = bytearray(record(1))
    corrupt[-1] ^= 1
    for later in (record(0), record(2), record(2**64 - 1), record(1)[:-1],
                  record(1) + b"x", corrupt):
      self.assert_invalid(header(2) + good + later)
    bad_tick = bytearray(record(1))
    struct.pack_into("<Q", bad_tick, 16, 0)
    self.assert_invalid(header(2) + good + fix_crc(bad_tick))
    with self.open_bytes(header(2) + good + record(1, family=11, variant=3)) as result:
      self.path.write_bytes(b"changed after validation")
      self.assertEqual(result.record(1).sequence, 1)
      self.assertEqual(result.record(1).variant, 3)

  def test_frame_counter_wrap(self) -> None:
    with self.path.open("wb") as stream:
      stream.write(header(65_537))
      for index in range(65_537):
        stream.write(record(index))
    with catalog.open_catalog(self.path) as result:
      self.assertEqual(result.record(65_536).envelope[26:28], b"\x00\x00")
      self.assertEqual(result.record(65_536).tick, 65_536)

  def test_starfield_structure_without_claiming_availability(self) -> None:
    metadata = star_metadata(latitude=900_000_000, longitude=-1_800_000_000,
                             elevation=100_000_000)
    sequence = record(0, family=6, metadata=metadata) + record(1) + record(
      2, family=6, metadata=metadata
    )
    with self.open_bytes(header(3) + sequence) as result:
      self.assertEqual(result.record(2).metadata, metadata)
      self.assertEqual(result.record(2).renderer_availability, "unavailable")
    self.assert_invalid(header(1) + record(family=6))
    for metadata in (
      bytes(32) + star_metadata()[32:],
      star_metadata()[:52] + bytes(32),
      star_metadata(epoch=946_684_799_999),
      star_metadata(epoch=4_102_444_800_000),
      star_metadata(latitude=900_000_001),
      star_metadata(longitude=-1_800_000_001),
      star_metadata(elevation=-12_000_001),
      star_metadata(elevation=100_000_001),
    ):
      self.assert_invalid(header(1) + record(family=6, metadata=metadata))
    self.assert_invalid(header(2) + record(family=6, metadata=star_metadata()) +
                        record(1, family=6, metadata=star_metadata(latitude=1)))
    self.assert_invalid(header(2) + record() +
                        record(1, family=6, metadata=star_metadata(epoch=4_102_444_799_999)))

  def test_reads_are_bounded(self) -> None:
    class GuardedInput(io.BytesIO):
      def read(self, size: int = -1) -> bytes:
        if not 0 <= size <= catalog.RECORD_SIZE:
          raise AssertionError(f"unbounded read: {size}")
        return super().read(size)

    source = GuardedInput(header(2) + record() + record(1))
    with mock.patch("builtins.open", return_value=source):
      with catalog.open_catalog("ignored") as result:
        self.assertEqual(result.count, 2)
    self.assertEqual(catalog.MAX_FILE_SIZE, 145_000_020)

  def test_foreign_aligned_files_and_legacy_fixtures(self) -> None:
    file = GOLDEN_HEADER + GOLDEN_RECORD
    # The legacy converter is alignment-only; neither aligned foreign input
    # is thereby safe for legacy playback.
    for foreign in (file, file[:132]):
      self.assertEqual(len(foreign) % 33, 0)
      self.assertIn("foreign_count", cvz_to_c.render_header(foreign, "foreign"))
    self.assert_invalid(file[:132])
    for _, _, name in chipviz_encode.FIXTURES:
      fixture = (chipviz_encode.FIXTURES_ROOT / name).read_bytes()
      self.assert_invalid(fixture)
    self.assertEqual(chipviz_encode.verify_fixtures(), 0)


if __name__ == "__main__":
  unittest.main()
