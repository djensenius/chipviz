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
  bridge->frame_sequence++;

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

uint32_t chipviz_joybus_frame_sequence(const ChipvizJoybusBridge *bridge) {
  if (bridge == 0) {
    return 0;
  }
  return bridge->frame_sequence;
}

int chipviz_joybus_poll_response(
    const ChipvizJoybusBridge *bridge,
    size_t controller_index,
    uint8_t response[CHIPVIZ_JOYBUS_STATE_SIZE]) {
  const uint8_t *controller;

  if (response == 0) {
    return 0;
  }
  controller = chipviz_joybus_controller(bridge, controller_index);
  if (controller == 0) {
    return 0;
  }
  memcpy(response, controller, CHIPVIZ_JOYBUS_STATE_SIZE);
  return 1;
}

int chipviz_joybus_status_response(uint8_t response[CHIPVIZ_JOYBUS_STATUS_SIZE]) {
  if (response == 0) {
    return 0;
  }
  response[0] = 0x05u;
  response[1] = 0x00u;
  response[2] = 0x02u;
  return 1;
}

void chipviz_joybus_serial_init(ChipvizJoybusSerial *serial) {
  if (serial == 0) {
    return;
  }
  memset(serial, 0, sizeof(*serial));
}

int chipviz_joybus_serial_push(
    ChipvizJoybusSerial *serial,
    ChipvizJoybusBridge *bridge,
    uint8_t byte) {
  if (serial == 0 || bridge == 0) {
    return 0;
  }
  if (serial->offset >= CHIPVIZ_JOYBUS_FRAME_SIZE) {
    serial->offset = 0;
  }
  serial->buffer[serial->offset] = byte;
  serial->offset++;
  if (serial->offset != CHIPVIZ_JOYBUS_FRAME_SIZE) {
    return 0;
  }
  serial->offset = 0;
  return chipviz_joybus_load_frame(bridge, serial->buffer, sizeof(serial->buffer));
}
