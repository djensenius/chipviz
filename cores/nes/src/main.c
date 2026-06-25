#include <stdint.h>

#include "chipviz/connection.h"
#include "chipviz/interface.h"

#ifdef CHIPVIZ_SIMULATOR
#include <stdio.h>
#endif

static void render_nes_visualization(const ChipvizFrame *frame) {
  uint8_t nametable_scroll = (uint8_t)(frame->beat_phase / 3u);
  uint8_t pulse_height = (uint8_t)(frame->bass / 8u);
  uint8_t triangle_step = (uint8_t)(frame->mid / 12u);
  uint8_t noise_gate = (uint8_t)(frame->treble > 96u ? 1u : 0u);

#ifdef CHIPVIZ_SIMULATOR
  printf(
      "nes frame=%u scene=%u palette=%u scroll=%u pulse=%u triangle=%u noise=%u\n",
      frame->frame,
      frame->scene,
      frame->palette,
      nametable_scroll,
      pulse_height,
      triangle_step,
      noise_gate);
#else
  (void)nametable_scroll;
  (void)pulse_height;
  (void)triangle_step;
  (void)noise_gate;
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
    render_nes_visualization(&frame);
  }

  return 0;
}
