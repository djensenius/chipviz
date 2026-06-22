#include "chipviz_pico/joybus_bridge.h"

#include <string.h>

void chipviz_joybus_init(ChipvizJoybusBridge *bridge) {
  if (bridge == 0) {
    return;
  }
  memset(bridge, 0, sizeof(*bridge));
}

int chipviz_joybus_load_frame(
    ChipvizJoybusBridge *bridge,
    const uint8_t frame[CHIPVIZ_JOYBUS_FRAME_SIZE],
    size_t size) {
  size_t controller;

  if (bridge == 0 || frame == 0 || size != CHIPVIZ_JOYBUS_FRAME_SIZE) {
    return 0;
  }

  for (controller = 0; controller < CHIPVIZ_JOYBUS_CONTROLLER_COUNT; controller++) {
    memcpy(
        bridge->controllers[controller],
        &frame[controller * CHIPVIZ_JOYBUS_STATE_SIZE],
        CHIPVIZ_JOYBUS_STATE_SIZE);
  }

  return 1;
}

const uint8_t *chipviz_joybus_controller(
    const ChipvizJoybusBridge *bridge,
    size_t controller_index) {
  if (bridge == 0 || controller_index >= CHIPVIZ_JOYBUS_CONTROLLER_COUNT) {
    return 0;
  }
  return bridge->controllers[controller_index];
}
