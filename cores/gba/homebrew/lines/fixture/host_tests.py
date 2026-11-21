#!/usr/bin/env python3
"""Host-only fixture and video-protocol checks; no emulator is loaded."""

import ctypes
import pathlib
import struct
import sys
import unittest

import harness

LIBRARY = pathlib.Path(sys.argv.pop(1)).resolve()
ENCODED = [struct.pack("<H", ((i & 31) << 11) | ((i >> 5) << 6)) for i in range(256)]


class State(ctypes.Structure):
  _fields_ = [("step", ctypes.c_uint32), ("keys", ctypes.c_uint16),
              ("phase", ctypes.c_uint16), ("beat_age", ctypes.c_uint16),
              ("section_age", ctypes.c_uint16), ("initialized", ctypes.c_uint8),
              ("fold", ctypes.c_uint8), ("target", ctypes.c_uint8),
              ("energy", ctypes.c_uint8), ("phrase_beats", ctypes.c_uint8)]


class Snapshot(ctypes.Structure):
  _fields_ = [("phase", ctypes.c_uint16), ("fold", ctypes.c_uint8),
              ("energy", ctypes.c_uint8), ("palette", ctypes.c_uint16 * 256)]


class Render(ctypes.Structure):
  _fields_ = [("snapshot", Snapshot), ("next", ctypes.c_size_t), ("active", ctypes.c_uint8)]


def frame(header, payload=b""):
  pixels = (header + payload).ljust(harness.PAGE, b"\0")
  return b"".join(ENCODED[pixel] for pixel in pixels)


def stream(records):
  frames = [bytes(harness.PAGE * 2)]
  for header, page, palette in records:
    for chunk, payload in enumerate((page[:harness.PAYLOAD], page[harness.PAYLOAD:], palette)):
      h = header[:6] + bytes((chunk,)) + header[7:]
      image = frame(h, payload)
      frames.extend((image, image))
  done = frame(b"GLFX\x01" + bytes((harness.EVENTS, 3)) + bytes(25))
  frames.extend((done, done))
  return frames


class FixtureTests(unittest.TestCase):
  @classmethod
  def setUpClass(cls):
    lib = ctypes.CDLL(str(LIBRARY))
    cls.lib = lib
    lib.lines_init.argtypes = [ctypes.POINTER(State)]
    lib.lines_init.restype = None
    lib.lines_observe.argtypes = [ctypes.POINTER(State), ctypes.c_uint32, ctypes.c_uint16]
    lib.lines_observe.restype = ctypes.c_int
    lib.lines_snapshot.argtypes = [ctypes.POINTER(State), ctypes.POINTER(Snapshot)]
    lib.lines_snapshot.restype = None
    lib.lines_render_begin.argtypes = [ctypes.POINTER(Render), ctypes.POINTER(Snapshot)]
    lib.lines_render_begin.restype = None
    lib.lines_render_cancel.argtypes = [ctypes.POINTER(Render)]
    lib.lines_render_cancel.restype = None
    lib.lines_render_batch.argtypes = [ctypes.POINTER(Render), ctypes.POINTER(ctypes.c_uint16),
                                       ctypes.c_size_t, ctypes.c_size_t]
    lib.lines_render_batch.restype = ctypes.c_int
    callback_type = ctypes.CFUNCTYPE(None, ctypes.POINTER(ctypes.c_uint8),
                                     ctypes.POINTER(ctypes.c_uint16), ctypes.POINTER(ctypes.c_uint16))
    lib.fixture_run.argtypes = [ctypes.POINTER(ctypes.c_uint16), callback_type]
    lib.fixture_run.restype = None
    guarded = (ctypes.c_uint16 * (harness.PAGE // 2 + 2))()
    cls.records = []

    @callback_type
    def collect(header, page, palette):
      cls.records.append((ctypes.string_at(header, harness.HEADER), ctypes.string_at(page, harness.PAGE),
                          ctypes.string_at(palette, 512)))

    lib.fixture_run(guarded, collect)

  def check_frames(self, frames):
    collector = harness.Collector()
    for image in frames:
      collector.feed(image)
    return collector

  def test_fixture_matches_scalar_oracle_and_protocol(self):
    self.assertEqual(len(self.records), harness.EVENTS)
    self.assertTrue(self.check_frames(stream(self.records)).done)

  def test_phase_boundaries_match_scalar_state_palette_and_page(self):
    state, scalar = State(), harness.ScalarState(0)
    self.lib.lines_init(ctypes.byref(state))
    self.assertEqual(self.lib.lines_observe(ctypes.byref(state), 0, 0), 1)
    for step in range(1, 65537):
      keys = harness.keys_at(0, step)
      scalar.advance(keys)
      self.assertEqual(self.lib.lines_observe(ctypes.byref(state), step, keys), 1)
      if step not in (47, 48, 65535, 65536):
        continue
      with self.subTest(step=step):
        self.assertEqual(scalar.phase, {47: 47, 48: 0, 65535: 15, 65536: 16}[step])
        self.assertEqual((state.step, state.keys, state.phase, state.fold, state.target, state.energy,
                          state.phrase_beats, state.beat_age, state.section_age),
                         (step, keys, scalar.phase, scalar.fold, scalar.target, scalar.energy,
                          scalar.phrase, scalar.beat_age, scalar.section_age))
        snapshot, render = Snapshot(), Render()
        page = (ctypes.c_uint16 * (harness.PAGE // 2))()
        self.lib.lines_snapshot(ctypes.byref(state), ctypes.byref(snapshot))
        self.assertEqual(struct.pack("<256H", *snapshot.palette), scalar.palette())
        self.lib.lines_render_begin(ctypes.byref(render), ctypes.byref(snapshot))
        for batch in range(80):
          result = self.lib.lines_render_batch(ctypes.byref(render), page, len(page), 240)
          self.assertEqual(result, int(batch == 79))
        page_bytes = struct.pack(f"<{len(page)}H", *page)
        self.assertEqual(page_bytes, scalar.pixels())
        if step == 65536:
          scalar.phase = 0  # Old 16-bit phase wrapping changes the rendered image here.
          self.assertNotEqual(page_bytes, scalar.pixels())

  def test_cancelled_and_completed_render_polling(self):
    state, snapshot, render = State(), Snapshot(), Render()
    page = (ctypes.c_uint16 * (harness.PAGE // 2))(*([0xA5A5] * (harness.PAGE // 2)))
    self.lib.lines_init(ctypes.byref(state))
    self.lib.lines_snapshot(ctypes.byref(state), ctypes.byref(snapshot))
    self.lib.lines_render_begin(ctypes.byref(render), ctypes.byref(snapshot))
    self.assertEqual(self.lib.lines_render_batch(ctypes.byref(render), page, len(page), 37), 0)
    self.lib.lines_render_cancel(ctypes.byref(render))
    partial = bytes(page)
    self.assertEqual((render.next, render.active), (37, 0))
    self.assertEqual(self.lib.lines_render_batch(ctypes.byref(render), page, len(page), 240), 0)
    self.assertEqual(bytes(page), partial)
    self.lib.lines_render_begin(ctypes.byref(render), ctypes.byref(snapshot))
    for batch in range(80):
      self.assertEqual(self.lib.lines_render_batch(ctypes.byref(render), page, len(page), 240),
                       int(batch == 79))
    complete = bytes(page)
    self.assertEqual((render.next, render.active), (len(page), 0))
    self.assertEqual(self.lib.lines_render_batch(ctypes.byref(render), page, len(page), 240), 1)
    self.lib.lines_render_cancel(ctypes.byref(render))
    self.assertEqual(self.lib.lines_render_batch(ctypes.byref(render), page, len(page), 240), 1)
    self.assertEqual(self.lib.lines_render_batch(ctypes.byref(render), page, len(page) - 1, 240), -1)
    self.assertEqual(bytes(page), complete)

  def test_rejects_bad_page_palette_and_canary(self):
    for field in (0, 1, 2):
      records = list(self.records)
      record = list(records[3])
      damaged = bytearray(record[field])
      damaged[24 if field == 0 else 0] ^= 1
      record[field] = bytes(damaged)
      records[3] = tuple(record)
      with self.subTest(field=field), self.assertRaises(AssertionError):
        self.check_frames(stream(records))

  def test_rejects_missing_chunk(self):
    frames = stream(self.records)
    del frames[3:5]
    with self.assertRaisesRegex(AssertionError, "out-of-order"):
      self.check_frames(frames)

  def test_rejects_truncated_frame_and_corrupt_transport(self):
    frames = stream(self.records)
    with self.assertRaisesRegex(AssertionError, "240x160"):
      self.check_frames(frames[:1] + [frames[1][:-2]])
    damaged = bytearray(frames[1])
    damaged[harness.HEADER * 2] ^= 1  # Not a valid transport palette value.
    frames[1:3] = [bytes(damaged)] * 2
    with self.assertRaisesRegex(AssertionError, "transport palette"):
      self.check_frames(frames)

  def test_rejects_incomplete_trace_and_malformed_completion(self):
    frames = stream(self.records)
    del frames[-4:-2]  # Omit the final palette chunk but send completion.
    with self.assertRaisesRegex(AssertionError, "out-of-order"):
      self.check_frames(frames)
    frames = stream(self.records)
    del frames[2]  # A single copy is not enough to accept the first chunk.
    with self.assertRaisesRegex(AssertionError, "out-of-order"):
      self.check_frames(frames)
    frames = stream(self.records)
    malformed = frame(b"GLFX\x01" + bytes((harness.EVENTS, 3, 1)) + bytes(24))
    frames[-2:] = [malformed, malformed]
    with self.assertRaisesRegex(AssertionError, "malformed completion"):
      self.check_frames(frames)

  def test_incomplete_trace_and_missing_completion_time_out(self):
    all_frames = stream(self.records)

    class FakeCore:
      def __init__(self, frames):
        self.frames = frames
        self.i = -1

      def run_frame(self):
        self.i += 1

      def pixels(self):
        return self.frames[self.i]

    bad_header = bytearray(all_frames[1])
    bad_header[0] ^= 1  # Invalid transport header must not become a record.
    for frames in (all_frames[:3], all_frames[:-2], [bytes(bad_header)] * 2):
      with self.subTest(accepted_frames=len(frames)):
        with self.assertRaisesRegex(AssertionError, "did not complete"):
          harness.consume(FakeCore(frames), len(frames))


if __name__ == "__main__":
  unittest.main()
