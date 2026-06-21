#include <stdint.h>

#include "chipviz/connection.h"

#ifdef CHIPVIZ_SIMULATOR
#include <stdio.h>
#endif

static void render_n64_visualization(const ChipvizFrame *frame) {
  uint8_t rotation = frame->beat_phase;
  uint8_t particle_count = (uint8_t)(8u + (frame->energy / 12u));
  uint8_t depth_pulse = frame->bass;

#ifdef CHIPVIZ_SIMULATOR
  printf(
      "n64 frame=%u scene=%u palette=%u rotation=%u particles=%u depth=%u\n",
      frame->frame,
      frame->scene,
      frame->palette,
      rotation,
      particle_count,
      depth_pulse);
#else
  (void)rotation;
  (void)particle_count;
  (void)depth_pulse;
#endif
}

int main(void) {
  ChipvizConnection connection;
  ChipvizFrame frame;
  ChipvizConnectionSource source;
  unsigned int index;

  chipviz_connection_init(&connection, 0, 0);

  for (index = 0; index < 4u; index++) {
    if (chipviz_connection_next_frame(&connection, &frame, &source) !=
        CHIPVIZ_CONNECTION_RECEIVE_OK) {
      return 1;
    }

    (void)source;
    render_n64_visualization(&frame);
  }

  return 0;
}
