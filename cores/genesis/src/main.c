#include <stdint.h>

#include "chipviz/connection.h"
#include "chipviz/interface.h"

#ifdef CHIPVIZ_SIMULATOR
#include <stdio.h>
#endif

static void render_genesis_visualization(const ChipvizFrame *frame) {
  uint8_t fm_voice_mask = (uint8_t)((frame->spectrum[0] >> 5u) | ((frame->spectrum[3] >> 4u) << 2u));
  uint8_t plane_scroll = (uint8_t)(frame->beat_phase + frame->bass / 8u);
  uint8_t psg_flash = (uint8_t)(frame->treble / 24u);

#ifdef CHIPVIZ_SIMULATOR
  printf(
      "genesis frame=%u scene=%u palette=%u fm=%u scroll=%u psg=%u\n",
      frame->frame,
      frame->scene,
      frame->palette,
      fm_voice_mask,
      plane_scroll,
      psg_flash);
#else
  (void)fm_voice_mask;
  (void)plane_scroll;
  (void)psg_flash;
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
    render_genesis_visualization(&frame);
  }

  return 0;
}
