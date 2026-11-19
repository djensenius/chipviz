import contextlib
import io
import itertools
import json
import math
import pathlib
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "host" / "bridge"))
sys.path.insert(0, str(ROOT / "shared" / "tools"))

import chipviz_bridge
import chipviz_encode
import chipsynth_stream
import cvz_to_c
import live_bridge
import n64_joybus
import usb_hid


class _RecordingSink:
  def __init__(self, paced: bool) -> None:
    self.paced = paced
    self.received: list[bytes] = []

  def send(self, frame: bytes) -> None:
    self.received.append(frame)


class TransportTests(unittest.TestCase):
  def setUp(self) -> None:
    self.frame = chipviz_bridge.make_procedural_frame(0)
    self.wire = chipviz_bridge.pack_frame(self.frame)

  def test_n64_joybus_packet_is_16_bytes(self) -> None:
    decoded = n64_joybus.decode_control_frame(self.wire)
    packet = n64_joybus.pack_joybus_frame(decoded, 0)

    self.assertEqual(len(packet), n64_joybus.JOYBUS_SIZE)
    self.assertEqual(packet[2], self.frame.spectrum[0])
    self.assertEqual(packet[3], self.frame.spectrum[1])
    self.assertEqual(packet[14], self.frame.energy)
    self.assertEqual(packet[15], self.frame.notes[0].velocity)

  def test_n64_joybus_stream_conversion(self) -> None:
    stream = self.wire * 3
    converted = n64_joybus.convert_stream(stream)

    self.assertEqual(len(converted), 3 * n64_joybus.JOYBUS_SIZE)

  def _joybus_packet(self, frame_index: int) -> bytes:
    wire = chipviz_bridge.pack_frame(chipviz_bridge.make_procedural_frame(frame_index))
    return n64_joybus.pack_joybus_frame(n64_joybus.decode_control_frame(wire), frame_index)

  def test_n64_joybus_marker_golden_vectors_match_rom_tests(self) -> None:
    # Mirrored in tests/n64_joybus_link_tests.c so host and ROM agree.
    self.assertEqual(self._joybus_packet(0).hex(), "802d001d00003a570202749101005050")
    self.assertEqual(self._joybus_packet(63).hex(), "002d2f4c023f69860308a3c040016700")

  def test_n64_joybus_marker_fields(self) -> None:
    for frame_index in (0, 1, 62, 63, 64, 65535):
      with self.subTest(frame_index=frame_index):
        packet = self._joybus_packet(frame_index)
        self.assertTrue(n64_joybus.joybus_marker_valid(packet))
        self.assertEqual(packet[1], n64_joybus.JOYBUS_MARKER_MAGIC)
        self.assertEqual(packet[5], frame_index & 0x3F)
        for index in (1, 5, 9):
          self.assertEqual(packet[index] & 0xC0, 0, "libdragon cannot read bits 0x80/0x40")

  def test_n64_joybus_marker_keeps_payload_bytes(self) -> None:
    packet = self._joybus_packet(7)
    unmarked = bytearray(packet)
    for index in (1, 5, 9):
      unmarked[index] = 0
    decoded = n64_joybus.decode_control_frame(chipviz_bridge.pack_frame(chipviz_bridge.make_procedural_frame(7)))
    self.assertEqual(unmarked[2], decoded.spectrum[0])
    self.assertEqual(unmarked[14], decoded.energy)
    self.assertFalse(n64_joybus.joybus_marker_valid(bytes(unmarked)))

  def test_n64_joybus_stream_sequence_is_per_packet(self) -> None:
    converted = n64_joybus.convert_stream(self.wire * 3, first_sequence=62)
    packets = [converted[i:i + 16] for i in range(0, len(converted), 16)]

    self.assertEqual([packet[5] for packet in packets], [62, 63, 0])
    self.assertTrue(all(n64_joybus.joybus_marker_valid(packet) for packet in packets))

  def test_n64_serial_sink_advances_sequence_for_repeated_frames(self) -> None:
    class _Port:
      def __init__(self) -> None:
        self.written: list[bytes] = []

      def write(self, data: bytes) -> None:
        self.written.append(data)

      def flush(self) -> None:
        pass

    sink = live_bridge.N64SerialSink.__new__(live_bridge.N64SerialSink)
    sink._port = _Port()
    sink._sequence = 0
    sink._cap = live_bridge.ColourFieldRateCap("joybus")
    for _ in range(66):
      sink.send(self.wire)

    sequences = [packet[5] for packet in sink._port.written]
    self.assertEqual(sequences, [index & 0x3F for index in range(66)])
    self.assertTrue(all(n64_joybus.joybus_marker_valid(packet) for packet in sink._port.written))

  def test_n64_joybus_marker_silent_frame_is_valid(self) -> None:
    silent = bytearray(16)
    n64_joybus.apply_joybus_marker(silent, 3)
    self.assertTrue(n64_joybus.joybus_marker_valid(bytes(silent)))
    self.assertEqual(bytes(silent[i] for i in range(16) if i not in (1, 5, 9)), bytes(13))

  def test_n64_joybus_marker_rejects_idle_pad_and_corruption(self) -> None:
    self.assertFalse(n64_joybus.joybus_marker_valid(bytes(16)))
    packet = self._joybus_packet(12)
    for index in range(16):
      with self.subTest(index=index):
        corrupt = bytearray(packet)
        corrupt[index] ^= 0x01
        self.assertFalse(n64_joybus.joybus_marker_valid(bytes(corrupt)))
    high_bit = bytearray(packet)
    high_bit[5] |= 0x40
    self.assertFalse(n64_joybus.joybus_marker_valid(bytes(high_bit)))

  def test_usb_hid_mapping_outputs_buttons_and_keys(self) -> None:
    state = usb_hid.map_frame(self.wire)

    self.assertTrue(state.buttons & 0x01)
    self.assertIn("Space", state.keys)
    self.assertIn("Q", state.keys)
    self.assertEqual(state.left_x, self.frame.energy - 128)
    self.assertEqual(state.left_y, -127)

  def test_live_bridge_normalizes_input_stream(self) -> None:
    frames = live_bridge.frame_records(self.wire * 2)

    self.assertEqual(frames, [self.wire, self.wire])

  def test_live_bridge_audio_band_normalization(self) -> None:
    bands = live_bridge.normalize_bands([0.0, 1.0, 2.0, 4.0, 0.0, 8.0, 2.0, 1.0])

    self.assertEqual(len(bands), 8)
    self.assertEqual(bands[0], 0)
    self.assertEqual(bands[5], 255)

  def test_live_bridge_band_edges_rejects_invalid_band_count(self) -> None:
    for bands in (0, -1):
      with self.subTest(bands=bands):
        with self.assertRaisesRegex(ValueError, "bands must be at least 1"):
          live_bridge.band_edges(48000, bands)

  def test_live_bridge_frame_from_bands_packs_notes(self) -> None:
    wire = live_bridge.frame_from_bands(9, (255, 128, 64, 32, 16, 8, 4, 2), midi_velocity=90)
    decoded = n64_joybus.decode_control_frame(wire)

    self.assertEqual(decoded.frame, 9)
    self.assertEqual(decoded.spectrum[0], 255)
    self.assertEqual(decoded.note_velocities, (90,))

  def test_live_bridge_frame_from_bands_requires_exactly_eight_bands(self) -> None:
    with self.assertRaisesRegex(ValueError, "requires exactly 8 bands"):
      live_bridge.frame_from_bands(0, (1, 2, 3, 4))
    with self.assertRaisesRegex(ValueError, "requires exactly 8 bands"):
      live_bridge.frame_from_bands(0, (1, 2, 3, 4, 5, 6, 7, 8, 9))

  def test_live_bridge_frame_from_bands_clamps_midi_velocity(self) -> None:
    high_wire = live_bridge.frame_from_bands(0, (1, 2, 3, 4, 5, 6, 7, 8), midi_velocity=300)
    low_wire = live_bridge.frame_from_bands(1, (1, 2, 3, 4, 5, 6, 7, 8), midi_velocity=-5)
    high_decoded = n64_joybus.decode_control_frame(high_wire)
    low_decoded = n64_joybus.decode_control_frame(low_wire)

    self.assertEqual(len(high_wire), chipviz_bridge.WIRE_SIZE)
    self.assertEqual(len(low_wire), chipviz_bridge.WIRE_SIZE)
    self.assertEqual(high_decoded.note_velocities, (255,))
    self.assertEqual(low_decoded.note_velocities, ())

  def test_live_bridge_rejects_missing_optional_dependency(self) -> None:
    with self.assertRaisesRegex(RuntimeError, "definitely_missing_chipviz_module"):
      live_bridge.require_module("definitely_missing_chipviz_module", "nothing")

  def test_stream_frames_fans_out_to_every_sink_in_order(self) -> None:
    frames = [b"a", b"b", b"c"]
    first = _RecordingSink(paced=False)
    second = _RecordingSink(paced=True)
    live_bridge.stream_frames(frames, realtime=False, rate=60, sinks=[first, second])
    self.assertEqual(first.received, frames)
    self.assertEqual(second.received, frames)

  def test_stream_frames_paces_non_realtime_sources_with_paced_sinks(self) -> None:
    sleeps = self._run_stream_with_fake_clock([b"x", b"y"], realtime=False, sinks=[_RecordingSink(paced=True)])
    self.assertEqual(len(sleeps), 2)
    self.assertTrue(all(delay > 0 for delay in sleeps))

  def test_stream_frames_skips_pacing_for_realtime_sources(self) -> None:
    sleeps = self._run_stream_with_fake_clock([b"x", b"y"], realtime=True, sinks=[_RecordingSink(paced=True)])
    self.assertEqual(sleeps, [])

  def test_stream_frames_skips_pacing_for_unpaced_sinks_only(self) -> None:
    sleeps = self._run_stream_with_fake_clock([b"x", b"y"], realtime=False, sinks=[_RecordingSink(paced=False)])
    self.assertEqual(sleeps, [])

  def _run_stream_with_fake_clock(self, frames, realtime, sinks):
    sleeps: list[float] = []
    clock = [0.0]
    original_monotonic = live_bridge.time.monotonic
    original_sleep = live_bridge.time.sleep
    live_bridge.time.monotonic = lambda: clock[0]
    live_bridge.time.sleep = lambda delay: sleeps.append(delay)
    try:
      live_bridge.stream_frames(frames, realtime=realtime, rate=60, sinks=sinks)
    finally:
      live_bridge.time.monotonic = original_monotonic
      live_bridge.time.sleep = original_sleep
    return sleeps

  def test_cvz_to_c_renders_frame_array(self) -> None:
    header = cvz_to_c.render_header(self.wire, "chipviz_test_frames")

    self.assertIn("chipviz_test_frames_count = 1u", header)
    self.assertIn("0xC7", header)

  def test_cli_outputs_are_json_or_files(self) -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
      root = pathlib.Path(temp_dir)
      cvz = root / "input.cvz"
      cvz.write_bytes(self.wire)
      hid = root / "hid.json"
      joybus = root / "joybus.bin"
      header = root / "frames.h"

      self.assertEqual(usb_hid.main_from_args(["--input", str(cvz), "--output", str(hid)]), 0)
      self.assertEqual(n64_joybus.main_from_args(["--input", str(cvz), "--output", str(joybus)]), 0)
      self.assertEqual(cvz_to_c.main_from_args(["--input", str(cvz), "--symbol", "frames", "--output", str(header)]), 0)

      self.assertEqual(len(joybus.read_bytes()), n64_joybus.JOYBUS_SIZE)
      self.assertEqual(json.loads(hid.read_text(encoding="utf-8"))[0]["keys"][0], "1")
      self.assertIn("static const uint8_t frames[]", header.read_text(encoding="utf-8"))

  def test_cvz_to_c_rejects_empty_symbol(self) -> None:
    with self.assertRaisesRegex(ValueError, "symbol"):
      cvz_to_c.sanitize_symbol("")

  def test_cvz_to_c_header_guard_uses_symbol(self) -> None:
    header = cvz_to_c.render_header(self.wire, "custom_frames")

    self.assertIn("CHIPVIZ_GENERATED_CUSTOM_FRAMES_H", header)


def _scene_frame(scene: int, palette: int) -> bytes:
  return chipviz_bridge.pack_frame(chipviz_bridge.ControlFrame(
    frame=0, bpm=120, beat_phase=0, beat_count=0, energy=100, bass=100, mid=100, treble=100,
    spectrum=(100,) * 8, scene=scene, palette=palette, flags=0, notes=(),
  ))


class FlashGuardTests(unittest.TestCase):
  """colour-field-v0 host flash guard (F23) and the CC1 scene debounce."""

  def test_f23_hid_alias_scene_15_palette_3(self) -> None:
    states = usb_hid.convert_stream(_scene_frame(15, 3) * 600)
    self.assertTrue(all(state.keys[:2] == ("8", "E") for state in states))
    self.assertEqual(usb_hid.map_frame(_scene_frame(15, 3), allow_flash=True).keys[:2], ("8", "R"))

  def test_f23_joybus_alias_scene_23_palette_3(self) -> None:
    converted = n64_joybus.convert_stream(_scene_frame(23, 3) * 600)
    packets = [converted[i:i + 16] for i in range(0, len(converted), 16)]
    for packet in packets:
      scene_nibble = ((packet[13] >> 2) & 0x0C) | ((packet[12] >> 4) & 0x03)
      self.assertEqual(scene_nibble, 7)
      self.assertEqual(packet[13] & 0x0F, 2)
      self.assertEqual(packet[12] >> 6, 2)
    allowed = n64_joybus.convert_stream(_scene_frame(23, 3), allow_flash=True)
    self.assertEqual(allowed[13] & 0x0F, 3)

  def test_f23_full_path_scene_15_is_not_rewritten(self) -> None:
    wire = _scene_frame(15, 3)
    self.assertEqual(chipviz_bridge.sanitise_flash(wire, "full"), wire)

  def test_full_path_scene_7_palette_3_and_7_sanitised_without_flag(self) -> None:
    for palette, expected in ((3, 2), (7, 6), (0x0B, 0x0A), (1, 1), (2, 2), (6, 6)):
      with self.subTest(palette=palette):
        wire = _scene_frame(7, palette)
        safe = chipviz_bridge.sanitise_flash(wire, "full")
        self.assertEqual(safe, _scene_frame(7, expected))
        self.assertEqual(safe[-1], chipviz_bridge.checksum(safe[:-1]))
        self.assertEqual(chipviz_bridge.sanitise_flash(wire, "full", allow_flash=True), wire)

  def test_sanitise_keeps_corrupt_frame_corrupt(self) -> None:
    corrupt = bytearray(_scene_frame(7, 3))
    corrupt[-1] ^= 0x40
    safe = chipviz_bridge.sanitise_flash(bytes(corrupt), "full")
    self.assertEqual(safe[21], 2)
    self.assertNotEqual(safe[-1], chipviz_bridge.checksum(safe[:-1]))
    with self.assertRaisesRegex(ValueError, "checksum"):
      n64_joybus.decode_control_frame(safe)

  def test_live_bridge_sinks_sanitise_per_transport(self) -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
      path = pathlib.Path(temp_dir) / "out.cvz"
      for allow_flash, expected in ((False, 2), (True, 3)):
        sink = live_bridge.FileSink(path, allow_flash)
        live_bridge.stream_frames([_scene_frame(7, 3), _scene_frame(15, 3)], True, 60, [sink])
        sink.close()
        written = path.read_bytes()
        self.assertEqual((written[21], written[33 + 21]), (expected, 3))

  def test_live_bridge_audio_never_emits_scene_7(self) -> None:
    scenes = {live_bridge.frame_from_bands(index, (0,) * 8)[20] for index in range(0, 240 * 16, 240)}
    self.assertNotIn(7, scenes)

  def test_live_bridge_one_frame_sanitised_per_sink_transport(self) -> None:
    class _Port:
      def __init__(self) -> None:
        self.written: list[bytes] = []

      def write(self, data: bytes) -> None:
        self.written.append(data)

      def flush(self) -> None:
        pass

    # Scene 23 decodes as 23 on the full path and as 7 on Joybus.
    with tempfile.TemporaryDirectory() as temp_dir:
      path = pathlib.Path(temp_dir) / "out.cvz"
      file_sink = live_bridge.FileSink(path)
      serial_sink = live_bridge.N64SerialSink.__new__(live_bridge.N64SerialSink)
      serial_sink._port = _Port()
      serial_sink._sequence = 0
      serial_sink._cap = live_bridge.ColourFieldRateCap("joybus", clock=itertools.count(0, 20_000_000).__next__)
      live_bridge.stream_frames([_scene_frame(23, 3), _scene_frame(7, 3)], True, 60, [file_sink, serial_sink])
      file_sink.close()
      written = path.read_bytes()
    self.assertEqual((written[21], written[33 + 21]), (3, 2))
    self.assertEqual([packet[13] & 0x0F for packet in serial_sink._port.written], [2, 2])

  def test_midi_path_debounces_cc1_into_scene_7(self) -> None:
    class _Message:
      def __init__(self, control: int, value: int) -> None:
        self.type = "control_change"
        self.control = control
        self.value = value

    class _Port:
      def __init__(self) -> None:
        self.polls = 0

      def __enter__(self):
        return self

      def __exit__(self, *exc) -> None:
        return None

      def iter_pending(self):
        self.polls += 1
        if self.polls == 1:
          return [_Message(1, 127), _Message(2, 48)]
        if 40 <= self.polls < 50:
          return [_Message(1, 112 if self.polls % 2 else 111)]
        return []

    fake_mido = type(sys)("mido")
    fake_mido.open_input = lambda name: _Port()
    original_sleep = live_bridge.time.sleep
    original_mido = sys.modules.get("mido")
    sys.modules["mido"] = fake_mido
    live_bridge.time.sleep = lambda delay: None
    try:
      frames = list(live_bridge.iter_midi_frames("fake", 80, 60))
    finally:
      live_bridge.time.sleep = original_sleep
      if original_mido is None:
        del sys.modules["mido"]
      else:
        sys.modules["mido"] = original_mido
    scenes = [frame[20] for frame in frames]
    self.assertEqual(scenes[:30], [0] * 30)
    self.assertEqual(scenes[30:80], [7] * 50, "CC1 jitter at 111/112 must not leave scene 7")
    self.assertTrue(all(frame[21] == 3 for frame in frames))
    file_frame = chipviz_bridge.sanitise_flash(frames[30], "full")
    self.assertEqual(file_frame[21], 2)

  def test_cc1_debounce_enters_scene_7_after_30_stable_frames(self) -> None:
    debounce = live_bridge.SceneDebounce()
    sent = [debounce.step(7) for _ in range(31)]
    self.assertEqual(sent[:30], [0] * 30)
    self.assertEqual(sent[30], 7)

  def test_cc1_debounce_ignores_jitter_across_7_boundary(self) -> None:
    debounce = live_bridge.SceneDebounce(6)
    sent = [debounce.step(7 if index % 20 < 10 else 6) for index in range(600)]
    self.assertEqual(set(sent), {6})
    debounce = live_bridge.SceneDebounce(7)
    sent = [debounce.step(6 if index % 2 else 7) for index in range(600)]
    self.assertEqual(set(sent), {7})

  def test_cc1_debounce_limits_scene_7_changes(self) -> None:
    debounce = live_bridge.SceneDebounce()
    sent = [debounce.step(7 if (index // 30) % 2 else 0) for index in range(600)]
    changes = [index for index in range(1, 600) if (sent[index] == 7) != (sent[index - 1] == 7)]
    self.assertTrue(changes)
    self.assertTrue(all(b - a >= 30 for a, b in zip(changes, changes[1:])))
    for start in range(600 - 60):
      self.assertLessEqual(sum(1 for index in changes if start <= index < start + 60), 2)

  def test_cc1_debounce_passes_native_scene_changes_at_once(self) -> None:
    debounce = live_bridge.SceneDebounce()
    self.assertEqual([debounce.step(scene) for scene in (1, 2, 5, 0)], [1, 2, 5, 0])

  def test_cc1_debounce_lasts_at_least_500_ms_at_any_rate(self) -> None:
    for rate, frames in ((30, 30), (60, 30), (90, 45), (120, 60), (240, 120)):
      with self.subTest(rate=rate):
        debounce = live_bridge.SceneDebounce(rate=rate)
        sent = [debounce.step(7) for _ in range(frames + 1)]
        self.assertEqual(sent[:frames], [0] * frames)
        self.assertEqual(sent[frames], 7)
        self.assertGreaterEqual(frames / rate, 0.5)
        exits = [debounce.step(0) for _ in range(frames + 1)]
        self.assertEqual(exits.index(0), frames)

  def test_allow_flash_cli_warning_only_with_flag(self) -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
      root = pathlib.Path(temp_dir)
      cvz = root / "input.cvz"
      cvz.write_bytes(_scene_frame(7, 3))
      music = ROOT / "host" / "fixtures" / "musical" / "scale.musicsource.json"
      out = str(root / "out")

      def via_argv(main, argv):
        def run(extra):
          original = sys.argv
          sys.argv = ["prog", *argv, *extra]
          try:
            return main()
          finally:
            sys.argv = original
        return run

      clis = {
        "usb_hid": lambda extra: usb_hid.main_from_args(["--input", str(cvz), "--output", out, *extra]),
        "n64_joybus": lambda extra: n64_joybus.main_from_args(["--input", str(cvz), "--output", out, *extra]),
        "chipviz_bridge": via_argv(chipviz_bridge.main, ["--frames", "1", "--output", out]),
        "chipsynth_stream": via_argv(chipsynth_stream.main, ["--output", out]),
        "chipviz_encode": via_argv(chipviz_encode.main, ["--music", str(music), "--output", out]),
        "live_bridge": via_argv(live_bridge.main, ["--input", str(cvz), "--output", out]),
      }
      for name, run in clis.items():
        for extra, expect_warning in (([], False), (["--allow-flash"], True)):
          with self.subTest(cli=name, allow_flash=expect_warning):
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
              self.assertEqual(run(extra), 0)
            self.assertEqual(chipviz_bridge.FLASH_WARNING in stderr.getvalue(), expect_warning)


def _cf_frame(index: int, scene: int = 7, flags: int = 0, notes: tuple = ()) -> bytes:
  return chipviz_bridge.pack_frame(chipviz_bridge.ControlFrame(
    frame=index & 0xFFFF, bpm=120, beat_phase=0, beat_count=0, energy=100, bass=100, mid=100, treble=100,
    spectrum=(100,) * 8, scene=scene, palette=0, flags=flags, notes=notes,
  ))


class _FakeClock:
  """Integer-nanosecond clock that advances only when the code sleeps."""

  def __init__(self) -> None:
    self.ns = 0

  def monotonic(self) -> float:
    return self.ns / 1e9

  def monotonic_ns(self) -> int:
    return self.ns

  def sleep(self, delay: float) -> None:
    self.ns += math.ceil(delay * 1e9)


def _counter(wire: bytes) -> int:
  return int.from_bytes(wire[2:4], "little")


class ColourFieldRateCapTests(unittest.TestCase):
  """colour-field-v0 "Step rate": at most 60 colour-field frames per second."""

  def _stream(self, frames: list[bytes], rate: int):
    class _Socket:
      def __init__(self, clock: _FakeClock) -> None:
        self.sent: list[tuple[int, bytes]] = []
        self._clock = clock

      def sendto(self, data: bytes, endpoint) -> None:
        self.sent.append((self._clock.ns, data))

      def write(self, data: bytes) -> None:
        self.sent.append((self._clock.ns, data))

      def flush(self) -> None:
        pass

      def close(self) -> None:
        pass

    clock = _FakeClock()
    with contextlib.ExitStack() as stack:
      for name in ("monotonic", "monotonic_ns", "sleep"):
        stack.enter_context(mock.patch.object(live_bridge.time, name, getattr(clock, name)))
      udp = live_bridge.UdpSink(("127.0.0.1", 9))
      udp._socket.close()
      udp._socket = _Socket(clock)
      serial = live_bridge.N64SerialSink.__new__(live_bridge.N64SerialSink)
      serial._port = _Socket(clock)
      serial._sequence = 0
      serial._cap = live_bridge.ColourFieldRateCap("joybus")
      live_bridge.stream_frames(frames, realtime=False, rate=rate, sinks=[udp, serial])
    return udp._socket.sent, serial._port.sent

  def _cap(self, transport: str, frames: list[bytes], step_ns: int = 8_333_334) -> list[bytes | None]:
    cap = live_bridge.ColourFieldRateCap(transport, clock=itertools.count(0, step_ns).__next__)
    return [cap.filter(frame) for frame in frames]

  def _assert_at_most_60_per_second(self, times: list[int]) -> None:
    # Half-open reading: any window [t, t + 1 s) holds at most 60 sends.
    for first, later in zip(times, times[60:]):
      self.assertGreaterEqual(later - first, 1_000_000_000)

  def test_rate_120_scene_7_drops_to_60_on_every_output(self) -> None:
    udp, serial = self._stream([_cf_frame(index) for index in range(240)], rate=120)
    for sent in (udp, serial):
      self.assertEqual(len(sent), 120)
      self._assert_at_most_60_per_second([at for at, _ in sent])
    self.assertEqual([_counter(data) for _, data in udp], list(range(0, 240, 2)))

  def test_rate_60_scene_7_is_not_decimated(self) -> None:
    udp, serial = self._stream([_cf_frame(index) for index in range(120)], rate=60)
    self.assertEqual((len(udp), len(serial)), (120, 120))

  def test_jitter_keeps_60_without_exceeding_it(self) -> None:
    # 120 Hz with up to 3 ms of late jitter: the slot grid absorbs it.
    times = [index * 8_333_334 + (index * 7919 % 3) * 1_000_000 for index in range(1200)]
    cap = live_bridge.ColourFieldRateCap("full", clock=iter(times).__next__)
    sent = [at for at, index in zip(times, range(1200)) if cap.filter(_cf_frame(index)) is not None]
    self.assertGreaterEqual(len(sent), 590)
    self._assert_at_most_60_per_second(sent)

  def test_rates_between_61_and_119_never_exceed_60(self) -> None:
    for rate, expected in ((90, 45), (100, 50), (110, 55)):
      with self.subTest(rate=rate):
        times = [math.ceil(index * 1e9 / rate) for index in range(rate * 5)]
        cap = live_bridge.ColourFieldRateCap("full", clock=iter(times).__next__)
        sent = [at for at, index in zip(times, range(len(times))) if cap.filter(_cf_frame(index)) is not None]
        self.assertEqual(len(sent), expected * 5)
        self._assert_at_most_60_per_second(sent)

  def test_leaving_and_entering_scene_7_at_rate_120(self) -> None:
    scenes = [7] * 60 + [6] * 60 + [7] * 60
    udp, _ = self._stream([_cf_frame(index, scene) for index, scene in enumerate(scenes)], rate=120)
    counters = [_counter(data) for _, data in udp]
    # Native-scene frames are not limited; scene-7 frames keep a 60 Hz grid.
    self.assertEqual(counters, list(range(0, 60, 2)) + list(range(60, 120)) + list(range(120, 180, 2)))
    sent_7 = [at for at, data in udp if data[20] == 7]
    self.assertTrue(all((b - a) * 60 >= 1_000_000_000 for a, b in zip(sent_7, sent_7[1:])))

  def test_cap_follows_each_transport_decoded_scene(self) -> None:
    # Scene 23 is native on the full path but scene 7 on Joybus (23 & 0x0F).
    udp, serial = self._stream([_cf_frame(index, 23) for index in range(240)], rate=120)
    self.assertEqual((len(udp), len(serial)), (240, 120))

  def test_dropped_edges_and_onsets_carry_to_next_sent_frame(self) -> None:
    note = chipviz_bridge.Note(60, 100)
    frames = []
    for index in range(40):
      flags = 0x01 if index % 2 else 0x00  # F15: beat on odd 120 Hz frames only.
      if index == 5:
        flags |= 0x02 | 0x04  # One-off bar and drop on a dropped frame.
      frames.append(_cf_frame(index, flags=flags, notes=(note,) if index == 9 else ()))
    sent = [out for out in self._cap("full", frames) if out is not None]
    self.assertEqual([_counter(out) for out in sent], list(range(0, 40, 2)))
    for out in sent:
      n64_joybus.decode_control_frame(out)
    self.assertEqual([out[22] & 0x01 for out in sent], [0, 1] * 10, "beat must not be held high")
    self.assertEqual([index for index, out in enumerate(sent) if out[22] & 0x06], [3])
    self.assertEqual([index for index, out in enumerate(sent) if out[23]], [5])
    self.assertEqual(sent[5][24:26], bytes((60, 100)))

  def test_edge_shown_by_previous_sent_frame_is_held_not_lost(self) -> None:
    # Reviewer repro: drop 1,0,1,0,0 at 120 Hz; the second rise is frame 2.
    drops = [1, 0, 1, 0, 0, 0, 0, 0]
    sent = [out[22] for out in self._cap("full", [_cf_frame(i, flags=f * 4) for i, f in enumerate(drops)]) if out]
    self.assertEqual(sent, [4, 4, 0, 4])
    # Beats on frames 1 and 3: the second is held one send, not lost.
    beats = [0, 1, 0, 1, 0, 0, 0, 0]
    sent = [out[22] for out in self._cap("full", [_cf_frame(i, flags=f) for i, f in enumerate(beats)]) if out]
    self.assertEqual(sent, [0, 1, 0, 1])

  def test_held_event_expires_after_carry_hold_sends(self) -> None:
    # A rise on dropped frame 1 while beat stays high on every sent frame can
    # never become an edge, so it must not surface after CARRY_HOLD sends.
    beats = [1, 0, 1, 1] + [1] * 8 + [0] * 8
    outs = self._cap("full", [_cf_frame(i, flags=0x01 if f else 0) for i, f in enumerate(beats)])
    sent = [out[22] & 0x01 for out in outs if out]
    self.assertEqual(sent, [1] * 6 + [0] * 4)

  def test_pending_note_also_on_next_frame_is_not_doubled(self) -> None:
    note = chipviz_bridge.Note(60, 100)
    frames = [_cf_frame(0), _cf_frame(1, notes=(note,)), _cf_frame(2, notes=(note,))]
    sent = [out for out in self._cap("full", frames) if out]
    self.assertEqual([out[23] for out in sent], [0, 1])
    self.assertEqual(sent[1], frames[2])

  def test_joybus_note_carry_uses_byte_15_edges(self) -> None:
    # Reviewer repro: A(60,100), B(64,100) dropped, C none, D dropped, E(60,100).
    notes = [((60, 100),), ((64, 100),), (), (), ((60, 100),)]
    frames = [_cf_frame(i, notes=tuple(chipviz_bridge.Note(*n) for n in pairs)) for i, pairs in enumerate(notes)]
    udp_like = [out for out in self._cap("joybus", frames) if out]
    packets = n64_joybus.convert_stream(b"".join(udp_like))
    self.assertEqual([packets[i * 16 + 15] for i in range(len(udp_like))], [100, 0, 100])
    # A Joybus onset on a dropped frame is carried as byte 15 rising.
    notes = [(), ((60, 100),), (), ()]
    frames = [_cf_frame(i, notes=tuple(chipviz_bridge.Note(*n) for n in pairs)) for i, pairs in enumerate(notes)]
    sent = [out for out in self._cap("joybus", frames) if out]
    packets = n64_joybus.convert_stream(b"".join(sent))
    self.assertEqual([packets[i * 16 + 15] for i in range(len(sent))], [0, 100])

  def test_corrupt_frames_are_not_laundered(self) -> None:
    corrupt = bytearray(_cf_frame(1, flags=0x04))
    corrupt[-1] ^= 0x40
    frames = [_cf_frame(0), bytes(corrupt), _cf_frame(2), _cf_frame(3), _cf_frame(4)]
    sent = [out for out in self._cap("full", frames) if out]
    self.assertEqual([out[22] for out in sent], [0, 0, 0], "a dropped corrupt drop flag must not be carried")
    # A corrupt frame that is sent goes out unchanged, and is not the edge reference.
    corrupt_low = bytearray(_cf_frame(2))
    corrupt_low[-1] ^= 0x40
    frames = [_cf_frame(0), _cf_frame(1, flags=0x04), bytes(corrupt_low), _cf_frame(3, flags=0x04), _cf_frame(4), _cf_frame(5)]
    outs = self._cap("full", frames, step_ns=16_666_667)
    self.assertEqual(outs[2], bytes(corrupt_low))
    self.assertEqual([out[22] for out in outs], [0, 4, 0, 4, 0, 0])

  def test_reserved_flag_bits_make_a_frame_invalid(self) -> None:
    # Checksum is correct, but bit 5 is reserved: targets reject the frame.
    reserved = bytearray(_cf_frame(1))
    reserved[22] = 0x21
    reserved[-1] = chipviz_bridge.checksum(reserved[:-1])
    frames = [_cf_frame(0), bytes(reserved), _cf_frame(2), _cf_frame(3), _cf_frame(4)]
    sent = [out for out in self._cap("full", frames) if out]
    self.assertEqual([out[22] for out in sent], [0, 0, 0], "a reserved-bit frame must not inject a beat")
    # Sent, it goes out unchanged and is not the edge reference: frame 3's
    # beat is not an edge against frame 1, so nothing is held or carried.
    frames = [_cf_frame(0), _cf_frame(1, flags=0x01), bytes(reserved), _cf_frame(3, flags=0x01), _cf_frame(4), _cf_frame(5)]
    outs = self._cap("full", frames, step_ns=16_666_667)
    self.assertEqual(outs[2], bytes(reserved))
    self.assertEqual([out[22] for out in outs], [0, 1, 0x21, 1, 0, 0])

  def test_carried_onset_replaces_quietest_sub_threshold_note(self) -> None:
    # Reviewer repro: loud note on dropped frame 1, four quiet notes after.
    quiet = tuple(chipviz_bridge.Note(70 + index, velocity) for index, velocity in enumerate((40, 10, 30, 20)))
    loud = chipviz_bridge.Note(60, 100)
    frames = [_cf_frame(0), _cf_frame(1, notes=(loud,))] + [_cf_frame(i, notes=quiet) for i in range(2, 8)]
    full = [out for out in self._cap("full", frames) if out]
    self.assertEqual(full[1][23], 4)
    self.assertEqual(full[1][24:32], bytes((70, 40, 60, 100, 72, 30, 73, 20)))
    self.assertTrue(all((60, 100) not in live_bridge._wire_notes(out) for out in full[2:]))
    joybus = [out for out in self._cap("joybus", frames) if out]
    packets = n64_joybus.convert_stream(b"".join(joybus))
    self.assertEqual([packets[i * 16 + 15] for i in range(len(joybus))], [0, 100, 40, 40])

  def test_carried_note_waits_for_room_then_is_bounded(self) -> None:
    def loud(*numbers: int) -> tuple:
      return tuple(chipviz_bridge.Note(number, 100) for number in numbers)

    held = loud(70, 71, 72)
    frames = [_cf_frame(0, notes=held), _cf_frame(1, notes=held + loud(60))]
    frames += [_cf_frame(i, notes=held + loud(80)) for i in range(2, 10)]
    frames += [_cf_frame(i, notes=held) for i in range(10, 14)]
    sent = [out for out in self._cap("full", frames) if out]
    shown = [index for index, out in enumerate(sent) if (60, 100) in live_bridge._wire_notes(out)]
    self.assertEqual(shown, [5], "the onset waits through 4 full sends, then takes the free slot")
    # Full for longer than CARRY_FULL_HOLD sends: the onset is dropped.
    limit = live_bridge.ColourFieldRateCap.CARRY_FULL_HOLD
    frames = [_cf_frame(0, notes=held), _cf_frame(1, notes=held + loud(60))]
    frames += [_cf_frame(i, notes=held + loud(80)) for i in range(2, 2 * limit + 4)]
    frames += [_cf_frame(i, notes=held) for i in range(2 * limit + 4, 2 * limit + 8)]
    sent = [out for out in self._cap("full", frames) if out]
    self.assertFalse(any((60, 100) in live_bridge._wire_notes(out) for out in sent))


class MidiRateTests(unittest.TestCase):
  def test_iter_midi_frames_scales_cc1_debounce_with_rate(self) -> None:
    class _Message:
      type = "control_change"
      control = 1
      value = 127

    class _Port:
      def __enter__(self):
        return self

      def __exit__(self, *exc) -> None:
        return None

      def iter_pending(self):
        return [_Message()]

    fake_mido = type(sys)("mido")
    fake_mido.open_input = lambda name: _Port()
    for fps, frames in ((60, 30), (120, 60)):
      with self.subTest(fps=fps), mock.patch.dict(sys.modules, {"mido": fake_mido}), \
          mock.patch.object(live_bridge.time, "sleep"):
        scenes = [frame[20] for frame in live_bridge.iter_midi_frames("fake", frames + 1, fps)]
      self.assertEqual(scenes, [0] * frames + [7])

if __name__ == "__main__":
  unittest.main()
