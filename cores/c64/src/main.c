#include <stdint.h>

#include "chipviz/connection.h"
#include "chipviz/interface.h"

#ifdef CHIPVIZ_SIMULATOR
#include <stdio.h>
#endif

static void render_c64_visualization(const ChipvizFrame *frame) {
  uint8_t raster_line = (uint8_t)(50u + (frame->beat_phase / 3u));
  uint8_t bar_color = (uint8_t)((frame->palette + frame->scene) & 0x0Fu);
  uint8_t petscii_cell = (uint8_t)((frame->energy / 16u) & 0x0Fu);

#ifdef CHIPVIZ_SIMULATOR
  printf(
      "c64 frame=%u scene=%u palette=%u raster=%u color=%u petscii=%u\n",
      frame->frame,
      frame->scene,
      frame->palette,
      raster_line,
      bar_color,
      petscii_cell);
#else
  (void)raster_line;
  (void)bar_color;
  (void)petscii_cell;
#endif
}

int main(void) {
  ChipvizConnection connection;
  ChipvizInterfaceState interface;
  ChipvizFrame frame;
  ChipvizConnectionSource source;
  unsigned int index;

  chipviz_connection_init(&connection, 0, 0);
  chipviz_interface_init(&interface, CHIPVIZ_INTERFACE_DEFAULT_SCENE_COUNT);
  chipviz_interface_apply_input(&interface, CHIPVIZ_INTERFACE_INPUT_DEMO);

  for (index = 0; index < 4u; index++) {
    if (chipviz_connection_next_frame(&connection, &frame, &source) !=
        CHIPVIZ_CONNECTION_RECEIVE_OK) {
      return 1;
    }

    chipviz_interface_apply_to_frame(&interface, &frame);
    chipviz_interface_tick(&interface);
    (void)source;
    render_c64_visualization(&frame);
  }

  return 0;
}
