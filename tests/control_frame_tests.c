#include "chipviz/control_frame.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void require(int condition, const char *message) {
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    exit(1);
  }
}

static void round_trips_procedural_frame(void) {
  ChipvizFrame input;
  ChipvizFrame output;
  uint8_t wire[CHIPVIZ_FRAME_WIRE_SIZE];

  chipviz_frame_make_procedural(0, &input);
  require(chipviz_frame_pack(&input, wire) == CHIPVIZ_FRAME_OK, "procedural frame packs");
  require(wire[0] == CHIPVIZ_FRAME_MAGIC, "magic is written");
  require(wire[1] == CHIPVIZ_FRAME_VERSION, "version is written");
  require(wire[32] == chipviz_frame_checksum(wire, CHIPVIZ_FRAME_WIRE_SIZE - 1u), "checksum is written");
  require(chipviz_frame_unpack(wire, &output) == CHIPVIZ_FRAME_OK, "packed frame unpacks");
  require(output.frame == input.frame, "frame preserved");
  require(output.beat_phase == input.beat_phase, "beat phase preserved");
  require(output.energy == input.energy, "energy preserved");
  require(output.spectrum[7] == input.spectrum[7], "spectrum preserved");
  require(output.note_count == input.note_count, "note count preserved");
  require(output.notes[0].note == input.notes[0].note, "note preserved");
}

static void rejects_invalid_frames(void) {
  ChipvizFrame frame;
  uint8_t wire[CHIPVIZ_FRAME_WIRE_SIZE];

  chipviz_frame_make_procedural(0, &frame);
  require(chipviz_frame_pack(NULL, wire) == CHIPVIZ_FRAME_INVALID_ARGUMENT, "null frame rejected");
  require(chipviz_frame_pack(&frame, NULL) == CHIPVIZ_FRAME_INVALID_ARGUMENT, "null wire rejected");

  frame.note_count = CHIPVIZ_FRAME_MAX_NOTES + 1u;
  require(chipviz_frame_pack(&frame, wire) == CHIPVIZ_FRAME_BAD_NOTE_COUNT, "too many notes rejected");

  chipviz_frame_make_procedural(0, &frame);
  frame.flags = CHIPVIZ_FRAME_RESERVED_FLAGS_MASK;
  require(chipviz_frame_pack(&frame, wire) == CHIPVIZ_FRAME_BAD_FLAGS, "reserved flags rejected");
}

static void rejects_invalid_wire_data(void) {
  ChipvizFrame frame;
  uint8_t wire[CHIPVIZ_FRAME_WIRE_SIZE];

  chipviz_frame_make_procedural(0, &frame);
  require(chipviz_frame_pack(&frame, wire) == CHIPVIZ_FRAME_OK, "frame packs for corruption tests");

  wire[0] = 0;
  require(chipviz_frame_unpack(wire, &frame) == CHIPVIZ_FRAME_BAD_MAGIC, "bad magic rejected");

  chipviz_frame_make_procedural(0, &frame);
  require(chipviz_frame_pack(&frame, wire) == CHIPVIZ_FRAME_OK, "frame repacks for bad version test");
  wire[1] = 99;
  require(chipviz_frame_unpack(wire, &frame) == CHIPVIZ_FRAME_BAD_VERSION, "bad version rejected");

  chipviz_frame_make_procedural(0, &frame);
  require(chipviz_frame_pack(&frame, wire) == CHIPVIZ_FRAME_OK, "frame repacks for bad checksum test");
  wire[32] ^= 1u;
  require(chipviz_frame_unpack(wire, &frame) == CHIPVIZ_FRAME_BAD_CHECKSUM, "bad checksum rejected");

  chipviz_frame_make_procedural(0, &frame);
  require(chipviz_frame_pack(&frame, wire) == CHIPVIZ_FRAME_OK, "frame repacks for bad note count test");
  wire[23] = CHIPVIZ_FRAME_MAX_NOTES + 1u;
  wire[32] = chipviz_frame_checksum(wire, CHIPVIZ_FRAME_WIRE_SIZE - 1u);
  require(chipviz_frame_unpack(wire, &frame) == CHIPVIZ_FRAME_BAD_NOTE_COUNT, "bad note count rejected");

  chipviz_frame_make_procedural(0, &frame);
  require(chipviz_frame_pack(&frame, wire) == CHIPVIZ_FRAME_OK, "frame repacks for bad flags test");
  wire[22] = CHIPVIZ_FRAME_RESERVED_FLAGS_MASK;
  wire[32] = chipviz_frame_checksum(wire, CHIPVIZ_FRAME_WIRE_SIZE - 1u);
  require(chipviz_frame_unpack(wire, &frame) == CHIPVIZ_FRAME_BAD_FLAGS, "bad wire flags rejected");
}

int main(void) {
  round_trips_procedural_frame();
  rejects_invalid_frames();
  rejects_invalid_wire_data();
  printf("control frame tests passed\n");
  return 0;
}
