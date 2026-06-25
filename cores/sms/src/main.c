#include <stdint.h>

#include "chipviz/connection.h"
#include "chipviz/interface.h"

#ifdef CHIPVIZ_SIMULATOR
#include <stdio.h>
#endif

static void render_sms_visualization(const ChipvizFrame *frame) {
  uint8_t psg_tone_a = (uint8_t)(frame->spectrum[0] / 8u);
  uint8_t psg_tone_b = (uint8_t)(frame->spectrum[2] / 8u);
  uint8_t psg_noise = (uint8_t)(frame->treble / 32u);
  uint8_t tile_phase = (uint8_t)(frame->beat_phase / 4u);

#ifdef CHIPVIZ_SIMULATOR
  printf(
      "sms frame=%u scene=%u palette=%u toneA=%u toneB=%u noise=%u tile=%u\n",
      frame->frame,
      frame->scene,
      frame->palette,
      psg_tone_a,
      psg_tone_b,
      psg_noise,
      tile_phase);
#else
  (void)psg_tone_a;
  (void)psg_tone_b;
  (void)psg_noise;
  (void)tile_phase;
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
    render_sms_visualization(&frame);
  }

  return 0;
}
