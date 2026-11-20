/* Compare the SMS lookup against the shared procedural generator. This checks
 * the lookup and a host model of the note counter, not Z80 execution. */
#include "chipviz/control_frame.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#define ROW_SIZE 13u
#define ROW_COUNT 256u

int main(int argc, char **argv) {
  uint8_t rows[ROW_SIZE * ROW_COUNT];
  uint8_t low = 0u;
  uint8_t high = 0u;
  uint8_t note_index = 0u;
  unsigned int index;
  FILE *file;

  if (argc != 2 || (file = fopen(argv[1], "rb")) == NULL) {
    fprintf(stderr, "usage: sms_procedural_oracle field-procedural.bin\n");
    return 1;
  }
  if (fread(rows, 1, sizeof(rows), file) != sizeof(rows) || fgetc(file) != EOF) {
    fprintf(stderr, "invalid procedural table length\n");
    fclose(file);
    return 1;
  }
  fclose(file);

  for (index = 0; index <= 65535u; ++index) {
    ChipvizFrame frame;
    const uint8_t *row = &rows[(unsigned int)low * ROW_SIZE];
    unsigned int band;
    chipviz_frame_make_procedural((uint16_t)index, &frame);
    if (row[0] != frame.energy || row[1] != frame.bass || row[2] != frame.mid ||
        row[3] != frame.treble || row[12] != frame.flags) {
      fprintf(stderr, "procedural field mismatch at frame %u\n", index);
      return 1;
    }
    for (band = 0; band < 8u; ++band) {
      if (row[4u + band] != frame.spectrum[band]) {
        fprintf(stderr, "procedural spectrum mismatch at frame %u band %u\n", index, band);
        return 1;
      }
    }
    if (frame.note_count != (uint8_t)(row[12] != 0u) ||
        (frame.note_count && (frame.notes[0].note != (uint8_t)(48u + note_index) ||
                              frame.notes[0].velocity != row[0]))) {
      fprintf(stderr, "procedural note mismatch at frame %u\n", index);
      return 1;
    }

    ++low;
    if (low == 0u && ++high == 0u) {
      note_index = 0u;
    } else if ((low & 15u) == 0u) {
      note_index = (uint8_t)((note_index + 1u) % 24u);
    }
  }
  if (low != 0u || high != 0u || note_index != 0u) {
    fprintf(stderr, "procedural frame wrap mismatch\n");
    return 1;
  }
  puts("sms procedural oracle: 65536 frames passed");
  return 0;
}
