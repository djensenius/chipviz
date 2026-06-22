#include <stdint.h>

#include "chipviz/connection.h"
#include "chipviz/interface.h"

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
    render_gba_visualization(&frame);
  }

  return 0;
}
