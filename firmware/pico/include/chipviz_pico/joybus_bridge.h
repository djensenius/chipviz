#ifndef CHIPVIZ_PICO_JOYBUS_BRIDGE_H
#define CHIPVIZ_PICO_JOYBUS_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#define CHIPVIZ_JOYBUS_CONTROLLER_COUNT 4u
#define CHIPVIZ_JOYBUS_STATE_SIZE 4u
#define CHIPVIZ_JOYBUS_FRAME_SIZE 16u
#define CHIPVIZ_JOYBUS_STATUS_SIZE 3u

typedef struct {
  uint8_t controllers[CHIPVIZ_JOYBUS_CONTROLLER_COUNT][CHIPVIZ_JOYBUS_STATE_SIZE];
  uint32_t frame_sequence;
} ChipvizJoybusBridge;

typedef struct {
  uint8_t buffer[CHIPVIZ_JOYBUS_FRAME_SIZE];
  size_t offset;
} ChipvizJoybusSerial;

void chipviz_joybus_init(ChipvizJoybusBridge *bridge);
int chipviz_joybus_load_frame(
    ChipvizJoybusBridge *bridge,
    const uint8_t frame[CHIPVIZ_JOYBUS_FRAME_SIZE],
    size_t size);
const uint8_t *chipviz_joybus_controller(
    const ChipvizJoybusBridge *bridge,
    size_t controller_index);
uint32_t chipviz_joybus_frame_sequence(const ChipvizJoybusBridge *bridge);
int chipviz_joybus_poll_response(
    const ChipvizJoybusBridge *bridge,
    size_t controller_index,
    uint8_t response[CHIPVIZ_JOYBUS_STATE_SIZE]);
int chipviz_joybus_status_response(uint8_t response[CHIPVIZ_JOYBUS_STATUS_SIZE]);
void chipviz_joybus_serial_init(ChipvizJoybusSerial *serial);
int chipviz_joybus_serial_push(
    ChipvizJoybusSerial *serial,
    ChipvizJoybusBridge *bridge,
    uint8_t byte);

#endif
