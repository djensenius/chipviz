#ifndef CHIPVIZ_PICO_JOYBUS_BRIDGE_H
#define CHIPVIZ_PICO_JOYBUS_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#define CHIPVIZ_JOYBUS_CONTROLLER_COUNT 4u
#define CHIPVIZ_JOYBUS_STATE_SIZE 4u
#define CHIPVIZ_JOYBUS_FRAME_SIZE 16u

typedef struct {
  uint8_t controllers[CHIPVIZ_JOYBUS_CONTROLLER_COUNT][CHIPVIZ_JOYBUS_STATE_SIZE];
} ChipvizJoybusBridge;

void chipviz_joybus_init(ChipvizJoybusBridge *bridge);
int chipviz_joybus_load_frame(
    ChipvizJoybusBridge *bridge,
    const uint8_t frame[CHIPVIZ_JOYBUS_FRAME_SIZE],
    size_t size);
const uint8_t *chipviz_joybus_controller(
    const ChipvizJoybusBridge *bridge,
    size_t controller_index);

#endif
