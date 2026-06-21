#include "chipviz/control_frame.h"

#include <string.h>

static void write_u16_le(uint8_t *bytes, uint16_t value) {
  bytes[0] = (uint8_t)(value & 0xFFu);
  bytes[1] = (uint8_t)((value >> 8) & 0xFFu);
}

static uint16_t read_u16_le(const uint8_t *bytes) {
  return (uint16_t)(bytes[0] | ((uint16_t)bytes[1] << 8));
}

static ChipvizFrameStatus validate_frame(const ChipvizFrame *frame) {
  if (frame == NULL) {
    return CHIPVIZ_FRAME_INVALID_ARGUMENT;
  }

  if (frame->note_count > CHIPVIZ_FRAME_MAX_NOTES) {
    return CHIPVIZ_FRAME_BAD_NOTE_COUNT;
  }

  if ((frame->flags & CHIPVIZ_FRAME_RESERVED_FLAGS_MASK) != 0u) {
    return CHIPVIZ_FRAME_BAD_FLAGS;
  }

  return CHIPVIZ_FRAME_OK;
}

const char *chipviz_frame_status_name(ChipvizFrameStatus status) {
  switch (status) {
    case CHIPVIZ_FRAME_OK:
      return "ok";
    case CHIPVIZ_FRAME_INVALID_ARGUMENT:
      return "invalid-argument";
    case CHIPVIZ_FRAME_BAD_MAGIC:
      return "bad-magic";
    case CHIPVIZ_FRAME_BAD_VERSION:
      return "bad-version";
    case CHIPVIZ_FRAME_BAD_CHECKSUM:
      return "bad-checksum";
    case CHIPVIZ_FRAME_BAD_NOTE_COUNT:
      return "bad-note-count";
    case CHIPVIZ_FRAME_BAD_FLAGS:
      return "bad-flags";
  }

  return "unknown";
}

uint8_t chipviz_frame_checksum(const uint8_t *bytes, size_t length) {
  uint8_t checksum = 0;
  size_t index;

  if (bytes == NULL) {
    return 0;
  }

  for (index = 0; index < length; index++) {
    checksum ^= bytes[index];
  }

  return checksum;
}

ChipvizFrameStatus chipviz_frame_pack(
    const ChipvizFrame *frame,
    uint8_t wire[CHIPVIZ_FRAME_WIRE_SIZE]) {
  ChipvizFrameStatus status;
  size_t index;
  size_t note_index;

  if (wire == NULL) {
    return CHIPVIZ_FRAME_INVALID_ARGUMENT;
  }

  status = validate_frame(frame);
  if (status != CHIPVIZ_FRAME_OK) {
    return status;
  }

  memset(wire, 0, CHIPVIZ_FRAME_WIRE_SIZE);
  wire[0] = CHIPVIZ_FRAME_MAGIC;
  wire[1] = CHIPVIZ_FRAME_VERSION;
  write_u16_le(&wire[2], frame->frame);
  wire[4] = frame->bpm;
  wire[5] = frame->beat_phase;
  write_u16_le(&wire[6], frame->beat_count);
  wire[8] = frame->energy;
  wire[9] = frame->bass;
  wire[10] = frame->mid;
  wire[11] = frame->treble;

  for (index = 0; index < CHIPVIZ_FRAME_SPECTRUM_BANDS; index++) {
    wire[12 + index] = frame->spectrum[index];
  }

  wire[20] = frame->scene;
  wire[21] = frame->palette;
  wire[22] = frame->flags;
  wire[23] = frame->note_count;

  for (note_index = 0; note_index < frame->note_count; note_index++) {
    wire[24 + (note_index * 2u)] = frame->notes[note_index].note;
    wire[25 + (note_index * 2u)] = frame->notes[note_index].velocity;
  }

  wire[32] = chipviz_frame_checksum(wire, CHIPVIZ_FRAME_WIRE_SIZE - 1u);
  return CHIPVIZ_FRAME_OK;
}

ChipvizFrameStatus chipviz_frame_unpack(
    const uint8_t wire[CHIPVIZ_FRAME_WIRE_SIZE],
    ChipvizFrame *frame) {
  size_t index;
  ChipvizFrameStatus status;

  if (wire == NULL || frame == NULL) {
    return CHIPVIZ_FRAME_INVALID_ARGUMENT;
  }

  if (wire[0] != CHIPVIZ_FRAME_MAGIC) {
    return CHIPVIZ_FRAME_BAD_MAGIC;
  }

  if (wire[1] != CHIPVIZ_FRAME_VERSION) {
    return CHIPVIZ_FRAME_BAD_VERSION;
  }

  if (chipviz_frame_checksum(wire, CHIPVIZ_FRAME_WIRE_SIZE - 1u) != wire[32]) {
    return CHIPVIZ_FRAME_BAD_CHECKSUM;
  }

  memset(frame, 0, sizeof(*frame));
  frame->frame = read_u16_le(&wire[2]);
  frame->bpm = wire[4];
  frame->beat_phase = wire[5];
  frame->beat_count = read_u16_le(&wire[6]);
  frame->energy = wire[8];
  frame->bass = wire[9];
  frame->mid = wire[10];
  frame->treble = wire[11];

  for (index = 0; index < CHIPVIZ_FRAME_SPECTRUM_BANDS; index++) {
    frame->spectrum[index] = wire[12 + index];
  }

  frame->scene = wire[20];
  frame->palette = wire[21];
  frame->flags = wire[22];
  frame->note_count = wire[23];

  status = validate_frame(frame);
  if (status != CHIPVIZ_FRAME_OK) {
    return status;
  }

  for (index = 0; index < frame->note_count; index++) {
    frame->notes[index].note = wire[24 + (index * 2u)];
    frame->notes[index].velocity = wire[25 + (index * 2u)];
  }

  return CHIPVIZ_FRAME_OK;
}

void chipviz_frame_make_procedural(uint16_t frame_index, ChipvizFrame *frame) {
  uint8_t phase;
  size_t index;

  if (frame == NULL) {
    return;
  }

  memset(frame, 0, sizeof(*frame));
  phase = (uint8_t)((frame_index * 17u) & 0xFFu);

  frame->frame = frame_index;
  frame->bpm = 120;
  frame->beat_phase = phase;
  frame->beat_count = (uint16_t)(frame_index / 16u);
  frame->energy = (uint8_t)(80u + (phase / 2u));
  frame->bass = (uint8_t)(255u - phase);
  frame->mid = (uint8_t)(64u + ((frame_index * 9u) & 0x7Fu));
  frame->treble = (uint8_t)((frame_index * 23u) & 0xFFu);

  for (index = 0; index < CHIPVIZ_FRAME_SPECTRUM_BANDS; index++) {
    frame->spectrum[index] = (uint8_t)((phase + (index * 29u)) & 0xFFu);
  }

  frame->scene = (uint8_t)((frame_index / 96u) % 3u);
  frame->palette = (uint8_t)((frame_index / 32u) % 8u);
  frame->flags = phase < 17u ? 0x01u : 0x00u;

  if ((frame->flags & 0x01u) != 0u) {
    frame->note_count = 1;
    frame->notes[0].note = (uint8_t)(48u + (frame->beat_count % 24u));
    frame->notes[0].velocity = frame->energy;
  }
}
