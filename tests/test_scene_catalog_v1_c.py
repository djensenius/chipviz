"""Synthetic single-record C/Python conformance; no file ordering is implied."""

import ctypes
import os
import pathlib
import random
import shlex
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "host" / "bridge"))
import scene_catalog_v1 as reference


GOLDEN = (
  bytes.fromhex("43 56 58 31 01 01 00 00") + bytes(16)
  + bytes.fromhex(
    "c7 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 "
    "00 00 00 00 07 02 00 00 00 00 00 00 00 00 00 00 c2"
  ) + bytes(84) + bytes.fromhex("c3 cc bb 8b")
)


class CRecord(ctypes.Structure):
  _fields_ = [
    ("envelope", ctypes.c_uint8 * 145),
    ("sequence", ctypes.c_uint64),
    ("tick", ctypes.c_uint64),
    ("family", ctypes.c_uint8),
    ("variant", ctypes.c_uint8),
    ("metadata", ctypes.c_uint8 * 84),
  ]


def repair(wire: bytearray, inner: bool = True) -> bytes:
  if inner:
    wire[56] = 0
    for byte in wire[24:56]:
      wire[56] ^= byte
  struct.pack_into("<I", wire, 141, zlib.crc32(wire[:141]))
  return bytes(wire)


def make_record(sequence: int = 0, family: int = 7, variant: int = 0,
                metadata: bytes = bytes(84)) -> bytes:
  wire = bytearray(GOLDEN)
  struct.pack_into("<QQ", wire, 8, sequence, sequence)
  struct.pack_into("<H", wire, 26, sequence & 65535)
  wire[44] = family
  wire[6] = variant
  wire[57:141] = metadata
  return repair(wire)


def star(epoch: int = 946_684_800_000, lat: int = 0, lon: int = 0,
         elev: int = 0) -> bytes:
  return struct.pack("<32sqiii32s", b"a", epoch, lat, lon, elev, b"p")


def python_status(wire: bytes) -> int:
  try:
    metadata = reference._validate_record(wire, int.from_bytes(wire[8:16], "little"), None)
    return 2 if metadata is not None else 0
  except ValueError:
    return 1


class CConformanceTests(unittest.TestCase):
  @classmethod
  def setUpClass(cls) -> None:
    cls.directory = tempfile.TemporaryDirectory()
    library = pathlib.Path(cls.directory.name) / "catalog.so"
    command = shlex.split(os.environ.get("CC", "cc")) + [
      "-std=c99", "-Wall", "-Wextra", "-Werror", "-pedantic", "-shared", "-fPIC",
      str(ROOT / "host" / "scene_catalog_v1.c"), "-o", str(library),
    ]
    subprocess.run(command, check=True, capture_output=True, text=True)
    cls.lib = ctypes.CDLL(str(library))
    cls.validate = cls.lib.chipviz_catalog_v1_validate
    cls.validate.argtypes = [ctypes.POINTER(ctypes.c_uint8), ctypes.c_size_t,
                             ctypes.POINTER(CRecord)]
    cls.validate.restype = ctypes.c_int

  @classmethod
  def tearDownClass(cls) -> None:
    cls.directory.cleanup()

  def check_record(self, wire: bytes, expected: int) -> None:
    self.assertEqual(python_status(wire), expected, "Python disagrees with specified expectation")
    buf = (ctypes.c_uint8 * len(wire)).from_buffer_copy(wire)
    output = CRecord()
    ctypes.memset(ctypes.byref(output), 0xa5, ctypes.sizeof(output))
    before = bytes(output)
    status = self.validate(buf, len(wire), ctypes.byref(output))
    self.assertEqual(status, expected)
    if status:
      self.assertEqual(bytes(output), before)
    else:
      self.assertEqual(bytes(output.envelope), wire)
      self.assertEqual(output.sequence, int.from_bytes(wire[8:16], "little"))
      self.assertEqual(output.tick, int.from_bytes(wire[16:24], "little"))
      self.assertEqual((output.family, output.variant), (wire[44], wire[6]))
      self.assertEqual(bytes(output.metadata), wire[57:141])

  def test_published_literal_vectors(self) -> None:
    self.check_record(GOLDEN, 0)
    for family, variant, checksum, crc, expected in (
      (2, 0, 0xc7, "52 55 2f d6", 0), (2, 2, 0xc7, "5f 1c 03 e5", 0),
      (2, 3, 0xc7, "f9 bb 2d 11", 1), (14, 0, 0xcb, "51 75 ce b2", 1),
    ):
      wire = bytearray(GOLDEN)
      wire[44], wire[6], wire[56] = family, variant, checksum
      wire[141:] = bytes.fromhex(crc)
      self.check_record(bytes(wire), expected)

  def test_independent_boundaries(self) -> None:
    for sequence in (0, 65535, 65536, 2**32, 2**63, 2**64 - 1):
      self.check_record(make_record(sequence), 0)
    for family in range(14):
      limit = {2: 2, 11: 3}.get(family, 0)
      for variant in range(limit + 2):
        metadata = star() if family == 6 else bytes(84)
        expected = 1 if variant > limit else 2 if family == 6 else 0
        self.check_record(make_record(family=family, variant=variant, metadata=metadata), expected)
    for metadata, expected in (
      (star(946_684_800_000, -900_000_000, -1_800_000_000, -12_000_000), 2),
      (star(4_102_444_799_999, 900_000_000, 1_800_000_000, 100_000_000), 2),
      (star(946_684_799_999), 1), (star(4_102_444_800_000), 1),
      (star(-(2**63)), 1), (star(2**63 - 1), 1),
      (star(lat=-900_000_001), 1), (star(lat=900_000_001), 1),
      (star(lon=-1_800_000_001), 1), (star(lon=1_800_000_001), 1),
      (star(elev=-12_000_001), 1), (star(elev=100_000_001), 1),
      (star(lat=-(2**31)), 1), (star(lon=2**31 - 1), 1),
      (star(elev=-(2**31)), 1), (star(elev=2**31 - 1), 1),
      (bytes(32) + star()[32:], 1), (star()[:52] + bytes(32), 1),
    ):
      self.check_record(make_record(family=6, metadata=metadata), expected)
    # One millisecond below the upper epoch allows exactly zero whole ticks;
    # fifty milliseconds allows three, but not four.
    for epoch, tick, expected in (
      (4_102_444_799_999, 0, 2), (4_102_444_799_999, 1, 1),
      (4_102_444_799_949, 3, 2), (4_102_444_799_949, 4, 1),
      (946_684_800_000, 189_345_599_999, 2),
      (946_684_800_000, 189_345_600_000, 1),
      (946_684_800_000, 2**64 - 1, 1),
    ):
      self.check_record(make_record(tick, 6, metadata=star(epoch)), expected)
    for count, note, expected in ((0, 255, 0), (1, 127, 0), (1, 128, 1), (5, 0, 1)):
      wire = bytearray(GOLDEN)
      wire[47], wire[48] = count, note
      wire[55] = 255
      self.check_record(repair(wire), expected)
    for index in range(4):
      wire = bytearray(GOLDEN)
      wire[47] = index + 1
      wire[48 + 2 * index] = 128
      self.check_record(repair(wire), 1)

  def test_repaired_corruptions(self) -> None:
    for offset, value, inner in (
      (0, 0, False), (4, 2, False), (5, 2, False), (7, 1, False),
      (16, 1, False), (24, 0, True), (25, 1, True), (26, 1, True),
      (46, 0x80, True), (47, 5, True), (56, 0xc3, False), (57, 1, False),
    ):
      wire = bytearray(GOLDEN)
      wire[offset] = value
      self.check_record(repair(wire, inner=inner), 1)
    wire = bytearray(GOLDEN)
    wire[141] ^= 1
    self.check_record(bytes(wire), 1)

  def test_synthetic_mutation_corpus(self) -> None:
    randomizer = random.Random(163)
    for _ in range(500):
      sequence = randomizer.getrandbits(64)
      family = randomizer.choice([0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12, 13])
      variant = randomizer.randrange({2: 3, 11: 4}.get(family, 1))
      wire = bytearray(make_record(sequence, family, variant))
      wire[28:44] = randomizer.randbytes(16)
      wire[45] = randomizer.randrange(256)
      wire[46] = randomizer.randrange(32)
      wire[47] = count = randomizer.randrange(5)
      for i in range(4):
        wire[48 + 2 * i] = randomizer.randrange(128 if i < count else 256)
        wire[49 + 2 * i] = randomizer.randrange(256)
      self.check_record(repair(wire), 0)
    for _ in range(1000):
      family = randomizer.randrange(16)
      sequence = randomizer.getrandbits(64)
      metadata = star() if family == 6 else bytes(84)
      wire = bytearray(make_record(sequence, family, randomizer.randrange(5), metadata))
      for _ in range(randomizer.randrange(1, 5)):
        offset = randomizer.randrange(141)
        wire[offset] = randomizer.randrange(256)
      wire = repair(wire, inner=bool(randomizer.randrange(2)))
      self.check_record(wire, python_status(wire))


if __name__ == "__main__":
  unittest.main()
