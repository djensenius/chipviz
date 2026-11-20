"""Offline structural validation of scene-catalog-v1 .cvxe files.

``open_catalog`` validates the entire input before returning a private snapshot.
It requests at most 145 bytes per input read and uses at most 145,000,000
bytes of temporary storage, rather than retaining a million Python objects.
Runtime/file buffers are separate; temporary storage may itself be RAM-backed.
The caller controls how many individual records it retains. Close the
snapshot, preferably with a ``with`` statement.
Records are retrieved individually as complete envelopes, never as stripped
legacy frames. The snapshot is not a playback or transport interface.
Snapshot access is not thread-safe.

Structural validity is not renderer availability or authorization. In
particular Starfield is unavailable: no private assets or provenance documents
are read or verified here. CRC and hashes do not authenticate input.
"""

from __future__ import annotations

import os
import stat
import struct
import tempfile
import zlib
from dataclasses import dataclass
from typing import BinaryIO


HEADER_SIZE = 20
RECORD_SIZE = 145
MAX_RECORDS = 1_000_000
MAX_FILE_SIZE = HEADER_SIZE + RECORD_SIZE * MAX_RECORDS
_MIN_EPOCH = 946_684_800_000
_MAX_EPOCH = 4_102_444_799_999
_MAX_VARIANT = (0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0)


def _crc(data: bytes) -> int:
  return zlib.crc32(data) & 0xFFFFFFFF


@dataclass(frozen=True)
class Record:
  """An intact extended envelope; renderer availability is not established."""

  envelope: bytes
  sequence: int
  tick: int
  family: int
  variant: int
  metadata: bytes

  @property
  def renderer_availability(self) -> str:
    return "unavailable" if self.family == 6 else "unassessed"


def _validate_record(wire: bytes, expected: int, star_metadata: bytes | None) -> bytes | None:
  if len(wire) != RECORD_SIZE:
    raise ValueError("truncated record")
  if wire[:6] != b"CVX1\x01\x01" or wire[7] != 0:
    raise ValueError("invalid record magic, version, catalog or reserved byte")
  if _crc(wire[:141]) != int.from_bytes(wire[141:145], "little"):
    raise ValueError("invalid record CRC")
  sequence, tick = struct.unpack_from("<QQ", wire, 8)
  if sequence != expected or tick != expected:
    raise ValueError("invalid sequence or tick")

  frame = wire[24:57]
  if frame[:2] != b"\xc7\x00":
    raise ValueError("invalid frame magic or version")
  checksum = 0
  for byte in frame:
    checksum ^= byte
  if checksum:
    raise ValueError("invalid frame XOR")
  if int.from_bytes(frame[2:4], "little") != expected % 65536:
    raise ValueError("invalid frame counter")
  if frame[22] & 0xE0:
    raise ValueError("reserved frame flags")
  if frame[23] > 4:
    raise ValueError("invalid note count")
  if any(frame[24 + 2 * index] > 127 for index in range(frame[23])):
    raise ValueError("invalid active note")
  family, variant = frame[20], wire[6]
  if family >= len(_MAX_VARIANT) or variant > _MAX_VARIANT[family]:
    raise ValueError("invalid family or variant")

  metadata = wire[57:141]
  if family != 6:
    if any(metadata):
      raise ValueError("non-Starfield metadata must be zero")
  else:
    asset, epoch, latitude, longitude, elevation, provenance = struct.unpack(
      "<32sqiii32s", metadata
    )
    if not any(asset) or not any(provenance):
      raise ValueError("missing Starfield digest")
    if not (_MIN_EPOCH <= epoch <= _MAX_EPOCH
            and -900_000_000 <= latitude <= 900_000_000
            and -1_800_000_000 <= longitude <= 1_800_000_000
            and -12_000_000 <= elevation <= 100_000_000):
      raise ValueError("invalid Starfield time or location")
    if epoch * 60 + tick * 1000 > _MAX_EPOCH * 60:
      raise ValueError("Starfield sky time out of range")
    if star_metadata is not None and metadata != star_metadata:
      raise ValueError("Starfield metadata changed within session")
    return metadata
  return star_metadata


class Catalog:
  """A validated, private temporary snapshot; no renderer is implied."""

  def __init__(self, snapshot: BinaryIO, header: bytes, count: int) -> None:
    self._snapshot = snapshot
    self.header = header
    self.count = count

  def record(self, index: int) -> Record:
    if not 0 <= index < self.count:
      raise IndexError("record index out of range")
    self._snapshot.seek(index * RECORD_SIZE)
    wire = self._snapshot.read(RECORD_SIZE)
    sequence, tick = struct.unpack_from("<QQ", wire, 8)
    return Record(wire, sequence, tick, wire[44], wire[6], wire[57:141])

  def close(self) -> None:
    self._snapshot.close()

  def __enter__(self) -> Catalog:
    return self

  def __exit__(self, *args: object) -> None:
    self.close()


def _open_regular(fd: int) -> BinaryIO:
  if isinstance(fd, bool) or not isinstance(fd, int):
    raise TypeError("input must be an already-open file descriptor")
  duplicate = os.dup(fd)
  try:
    if not stat.S_ISREG(os.fstat(duplicate).st_mode):
      raise ValueError("input must be a regular file")
    return os.fdopen(duplicate, "rb")
  except BaseException:
    os.close(duplicate)
    raise


def open_catalog(fd: int) -> Catalog:
  """Validate a .cvxe from an already-open readable regular-file descriptor.

  No input path is opened: the caller is responsible for acquiring the
  descriptor safely, without opening an untrusted device or FIFO. O_NONBLOCK
  and O_NOCTTY alone cannot make arbitrary device opens safe. The descriptor
  is duplicated before checking its type, so later path or descriptor
  replacement cannot change the checked input. Validation starts at offset
  zero; the duplicate shares and changes the caller's file position. Do not
  use buffered or concurrent access to that open file description during
  validation, or mutate the input while it is being read. The caller retains
  ownership of its descriptor. Regular-file checks do not bound I/O latency
  on remote, virtual, or faulty filesystems.

  Raises TypeError for path arguments, ValueError for invalid structure or
  non-regular input, or OSError for I/O failures. No record is exposed on
  failure, including errors in later records, at EOF, or while flushing the
  snapshot. The caller must independently establish renderer and safety
  conditions.
  """
  snapshot = None
  try:
    with _open_regular(fd) as source:
      source.seek(0)
      header = source.read(HEADER_SIZE)
      if len(header) != HEADER_SIZE:
        raise ValueError("truncated header")
      if header[:8] != b"CVXE\x01\x01\x00\x00":
        raise ValueError("invalid header magic, version, catalog or reserved bytes")
      if _crc(header[:16]) != int.from_bytes(header[16:20], "little"):
        raise ValueError("invalid header CRC")
      count = int.from_bytes(header[8:16], "little")
      if not 1 <= count <= MAX_RECORDS:
        raise ValueError("invalid record count")
      # Python integers do not overflow; the bound also limits temporary storage.
      size = HEADER_SIZE + RECORD_SIZE * count
      if size > MAX_FILE_SIZE:
        raise ValueError("file too large")

      snapshot = tempfile.TemporaryFile(mode="w+b")
      star_metadata = None
      for index in range(count):
        wire = source.read(RECORD_SIZE)
        star_metadata = _validate_record(wire, index, star_metadata)
        snapshot.write(wire)
      if source.read(1):
        raise ValueError("trailing bytes")
      snapshot.flush()
    return Catalog(snapshot, header, count)
  except BaseException:
    if snapshot is not None:
      try:
        snapshot.close()
      except OSError:
        pass
    raise
