import pathlib
import sys
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "host" / "bridge"))

import chipviz_bridge
import chipsynth_stream


class ControlFrameTests(unittest.TestCase):
  def test_procedural_note_boundaries_and_wrap(self) -> None:
    # Independently expected shared-domain values are also checked in C and
    # Rust; the out-of-range cases check Python's existing wrap behavior.
    # index, wire frame, beat, phase, scene, palette, optional (note, velocity)
    cases = (
      (0, 0, 0, 0, 0, 0, (48, 80)),
      (1, 1, 0, 17, 0, 0, None),
      (4081, 4081, 255, 1, 0, 7, (63, 80)),
      (4095, 4095, 255, 239, 0, 7, None),
      (4096, 4096, 256, 0, 0, 0, (64, 80)),
      (4097, 4097, 256, 17, 0, 0, None),
      (65521, 65521, 4095, 1, 1, 7, (63, 80)),
      (65535, 65535, 4095, 239, 1, 7, None),
      (65536, 0, 4096, 0, 1, 0, (64, 80)),
      # Python calculates the note from the unbounded beat, before wire truncation.
      (1048576, 0, 0, 0, 2, 0, (64, 80)),
    )
    for index, wire_frame, beat, phase, scene, palette, note in cases:
      with self.subTest(index=index):
        frame = chipviz_bridge.make_procedural_frame(index)
        self.assertEqual((frame.frame, frame.beat_count, frame.beat_phase),
                         (wire_frame, beat, phase))
        self.assertEqual((frame.scene, frame.palette), (scene, palette))
        self.assertEqual(frame.flags, int(note is not None))
        self.assertEqual(frame.notes, (chipviz_bridge.Note(*note),) if note else ())

  def test_procedural_frame_packs_to_expected_wire_shape(self) -> None:
    frame = chipviz_bridge.make_procedural_frame(17)
    wire = chipviz_bridge.pack_frame(frame)

    self.assertEqual(len(wire), chipviz_bridge.WIRE_SIZE)
    self.assertEqual(wire[0], chipviz_bridge.MAGIC)
    self.assertEqual(wire[1], chipviz_bridge.VERSION)
    self.assertEqual(wire[-1], chipviz_bridge.checksum(wire[:-1]))
    self.assertEqual(wire[12:20], bytes(frame.spectrum))

  def test_pack_rejects_reserved_flags(self) -> None:
    frame = chipviz_bridge.make_procedural_frame(0)
    bad = chipviz_bridge.ControlFrame(
      frame=frame.frame,
      bpm=frame.bpm,
      beat_phase=frame.beat_phase,
      beat_count=frame.beat_count,
      energy=frame.energy,
      bass=frame.bass,
      mid=frame.mid,
      treble=frame.treble,
      spectrum=frame.spectrum,
      scene=frame.scene,
      palette=frame.palette,
      flags=0xE0,
      notes=frame.notes,
    )

    with self.assertRaisesRegex(ValueError, "reserved flag"):
      chipviz_bridge.pack_frame(bad)


class ChipSynthStreamTests(unittest.TestCase):
  def test_demo_packet_maps_to_control_frame(self) -> None:
    packet = chipsynth_stream.make_demo_packet()
    parsed = chipsynth_stream.parse_packet(packet)
    frame = chipsynth_stream.to_control_frame(parsed)
    wire = chipviz_bridge.pack_frame(frame)

    self.assertEqual(len(packet), chipsynth_stream.PACKET_SIZE)
    self.assertEqual(packet[:4], chipsynth_stream.MAGIC)
    self.assertEqual(packet[-1], chipsynth_stream.checksum(packet[:-1]))
    self.assertEqual(frame.frame, 42)
    self.assertEqual(frame.bpm, 128)
    self.assertEqual(frame.flags, 0x01)
    self.assertEqual(frame.energy, 136)
    self.assertEqual(frame.scene, 1)
    self.assertEqual(frame.palette, 0)
    self.assertEqual(frame.notes[0], chipviz_bridge.Note(60, 110))
    self.assertEqual(wire[-1], chipviz_bridge.checksum(wire[:-1]))

  def test_dominant_voice_uses_level_then_velocity_then_lowest_voice_index(self) -> None:
    voices = [
      chipsynth_stream.Voice(2, 2, 1, 60, 100, 90, chipsynth_stream.VOICE_ACTIVE),
      chipsynth_stream.Voice(1, 3, 2, 62, 100, 90, chipsynth_stream.VOICE_ACTIVE),
      chipsynth_stream.Voice(0, 4, 3, 64, 80, 90, chipsynth_stream.VOICE_ACTIVE),
    ]

    self.assertEqual(chipsynth_stream.dominant_active_voice(voices), voices[1])

  def test_parse_rejects_bad_checksum(self) -> None:
    packet = bytearray(chipsynth_stream.make_demo_packet())
    packet[-1] ^= 0x01

    with self.assertRaisesRegex(ValueError, "checksum"):
      chipsynth_stream.parse_packet(bytes(packet))

  def test_parse_rejects_invalid_voice(self) -> None:
    packet = bytearray(chipsynth_stream.make_demo_packet())
    packet[49] = chipsynth_stream.CHANNEL_COUNT
    packet[-1] = chipsynth_stream.checksum(packet[:-1])

    with self.assertRaisesRegex(ValueError, "voice channel"):
      chipsynth_stream.parse_packet(bytes(packet))


if __name__ == "__main__":
  unittest.main()
