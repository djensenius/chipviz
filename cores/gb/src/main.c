#include <stdint.h>

#include "chipviz/connection.h"
#include "chipviz/interface.h"

#ifdef CHIPVIZ_SIMULATOR
#include <stdio.h>
#endif

static void render_gb_visualization(const ChipvizFrame *frame) {
  uint8_t bg_scroll = (uint8_t)(frame->beat_phase / 2u);
  uint8_t window_line = (uint8_t)(32u + (frame->bass / 4u));
  uint8_t sprite_lane = (uint8_t)(frame->mid / 16u);

#ifdef CHIPVIZ_SIMULATOR
  printf(
      "gb frame=%u scene=%u palette=%u scroll=%u window=%u lane=%u\n",
      frame->frame,
      frame->scene,
      frame->palette,
      bg_scroll,
      window_line,
      sprite_lane);
#else
  (void)bg_scroll;
  (void)window_line;
  (void)sprite_lane;
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
    render_gb_visualization(&frame);
  }

  return 0;
}
