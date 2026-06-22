#include <stdint.h>

#include "chipviz/connection.h"
#include "chipviz/interface.h"

#ifdef CHIPVIZ_SIMULATOR
#include <stdio.h>
#endif

static void render_snes_visualization(const ChipvizFrame *frame) {
  uint8_t mode7_angle = frame->beat_phase;
  uint8_t sprite_count = (uint8_t)(6u + (frame->energy / 18u));
  uint8_t cgram_bank = (uint8_t)(frame->palette & 0x07u);
  uint8_t hdma_pulse = (uint8_t)((frame->flags & 0x01u) ? 255u : frame->treble);

#ifdef CHIPVIZ_SIMULATOR
  printf(
      "snes frame=%u scene=%u palette=%u mode7=%u sprites=%u cgram=%u hdma=%u\n",
      frame->frame,
      frame->scene,
      frame->palette,
      mode7_angle,
      sprite_count,
      cgram_bank,
      hdma_pulse);
#else
  (void)mode7_angle;
  (void)sprite_count;
  (void)cgram_bank;
  (void)hdma_pulse;
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
    render_snes_visualization(&frame);
  }

  return 0;
}
