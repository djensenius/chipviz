#ifndef CHIPVIZ_CONTROL_FRAME_H
#define CHIPVIZ_CONTROL_FRAME_H

#include <stddef.h>
#include <stdint.h>

#define CHIPVIZ_FRAME_MAGIC 0xC7u
#define CHIPVIZ_FRAME_VERSION 0u
#define CHIPVIZ_FRAME_SPECTRUM_BANDS 8u
#define CHIPVIZ_FRAME_MAX_NOTES 4u
#define CHIPVIZ_FRAME_WIRE_SIZE 33u
#define CHIPVIZ_FRAME_RESERVED_FLAGS_MASK 0xE0u

typedef struct {
  uint8_t note;
  uint8_t velocity;
} ChipvizNote;

typedef struct {
  uint16_t frame;
  uint8_t bpm;
  uint8_t beat_phase;
  uint16_t beat_count;
  uint8_t energy;
  uint8_t bass;
  uint8_t mid;
  uint8_t treble;
  uint8_t spectrum[CHIPVIZ_FRAME_SPECTRUM_BANDS];
  uint8_t scene;
  uint8_t palette;
  uint8_t flags;
  uint8_t note_count;
  ChipvizNote notes[CHIPVIZ_FRAME_MAX_NOTES];
} ChipvizFrame;

typedef enum {
  CHIPVIZ_FRAME_OK = 0,
  CHIPVIZ_FRAME_INVALID_ARGUMENT,
  CHIPVIZ_FRAME_BAD_MAGIC,
  CHIPVIZ_FRAME_BAD_VERSION,
  CHIPVIZ_FRAME_BAD_CHECKSUM,
  CHIPVIZ_FRAME_BAD_NOTE_COUNT,
  CHIPVIZ_FRAME_BAD_FLAGS
} ChipvizFrameStatus;

const char *chipviz_frame_status_name(ChipvizFrameStatus status);
uint8_t chipviz_frame_checksum(const uint8_t *bytes, size_t length);
ChipvizFrameStatus chipviz_frame_pack(
    const ChipvizFrame *frame,
    uint8_t wire[CHIPVIZ_FRAME_WIRE_SIZE]);
ChipvizFrameStatus chipviz_frame_unpack(
    const uint8_t wire[CHIPVIZ_FRAME_WIRE_SIZE],
    ChipvizFrame *frame);
void chipviz_frame_make_procedural(uint16_t frame_index, ChipvizFrame *frame);

#endif
