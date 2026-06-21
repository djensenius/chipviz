#include <stdint.h>

#include "chipviz/connection.h"

#ifdef CHIPVIZ_SIMULATOR
#include <stdio.h>
#endif

static void render_gba_visualization(const ChipvizFrame *frame) {
  uint8_t tile_scroll = (uint8_t)(frame->beat_phase / 4u);
  uint8_t sprite_count = (uint8_t)(4u + (frame->mid / 24u));
  uint8_t affine_scale = (uint8_t)(128u + (frame->energy / 4u));

#ifdef CHIPVIZ_SIMULATOR
  printf(
      "gba frame=%u scene=%u palette=%u scroll=%u sprites=%u affine=%u\n",
      frame->frame,
      frame->scene,
      frame->palette,
      tile_scroll,
      sprite_count,
      affine_scale);
#else
  (void)tile_scroll;
  (void)sprite_count;
  (void)affine_scale;
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
    render_gba_visualization(&frame);
  }

  return 0;
}
