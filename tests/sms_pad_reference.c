/* Host-only SMS input stream reference using the shared C generator. */
#include "chipviz/control_frame.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

int main(void) {
  uint16_t counter = 0u;
  int digital = 0;
  int buttons;
  while ((buttons = getchar()) != EOF) {
    ChipvizFrame frame;
    uint8_t row[16];
    unsigned int band;
    if ((buttons & ~0x3f) != 0) {
      fputs("invalid SMS buttons\n", stderr);
      return 1;
    }
    digital |= (buttons & 0x30) != 0;
    if (digital) {
      memset(&frame, 0, sizeof(frame));
      frame.energy = frame.bass = frame.mid = frame.treble = 64u;
      frame.flags = (uint8_t)((buttons >> 4) & 3);
    } else {
      chipviz_frame_make_procedural(counter, &frame);
      counter = (uint16_t)(counter + 1u);
    }
    row[0] = frame.energy;
    row[1] = frame.bass;
    row[2] = frame.mid;
    row[3] = frame.treble;
    for (band = 0; band < 8u; ++band) row[4u + band] = frame.spectrum[band];
    row[12] = frame.flags;
    row[13] = frame.note_count;
    row[14] = frame.notes[0].note;
    row[15] = frame.notes[0].velocity;
    if (fwrite(row, 1, sizeof(row), stdout) != sizeof(row)) return 1;
  }
  return ferror(stdin) ? 1 : 0;
}
