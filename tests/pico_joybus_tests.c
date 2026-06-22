#include "chipviz_pico/joybus_bridge.h"

#include <stdio.h>
#include <stdlib.h>

static void require(int condition, const char *message) {
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    exit(1);
  }
}

static void loads_four_controller_buffers(void) {
  ChipvizJoybusBridge bridge;
  uint8_t frame[CHIPVIZ_JOYBUS_FRAME_SIZE];
  const uint8_t *controller;
  size_t index;

  for (index = 0; index < CHIPVIZ_JOYBUS_FRAME_SIZE; index++) {
    frame[index] = (uint8_t)(index + 1u);
  }

  chipviz_joybus_init(&bridge);
  require(chipviz_joybus_frame_sequence(&bridge) == 0u, "sequence starts at zero");
  require(chipviz_joybus_load_frame(&bridge, frame, sizeof(frame)), "frame loads");
  require(chipviz_joybus_frame_sequence(&bridge) == 1u, "sequence increments");
  controller = chipviz_joybus_controller(&bridge, 2);
  require(controller != 0, "controller pointer returned");
  require(controller[0] == 9u, "controller 3 byte 0");
  require(controller[3] == 12u, "controller 3 byte 3");
}

static void builds_poll_and_status_responses(void) {
  ChipvizJoybusBridge bridge;
  uint8_t frame[CHIPVIZ_JOYBUS_FRAME_SIZE] = {0};
  uint8_t poll[CHIPVIZ_JOYBUS_STATE_SIZE] = {0};
  uint8_t status[CHIPVIZ_JOYBUS_STATUS_SIZE] = {0};

  frame[4] = 0xAAu;
  frame[5] = 0x55u;
  frame[6] = 127u;
  frame[7] = 128u;
  chipviz_joybus_init(&bridge);
  require(chipviz_joybus_load_frame(&bridge, frame, sizeof(frame)), "frame loads");
  require(chipviz_joybus_poll_response(&bridge, 1, poll), "poll response builds");
  require(poll[0] == 0xAAu && poll[1] == 0x55u, "poll buttons copied");
  require(poll[2] == 127u && poll[3] == 128u, "poll sticks copied");
  require(chipviz_joybus_status_response(status), "status response builds");
  require(status[0] == 0x05u && status[1] == 0x00u && status[2] == 0x02u, "status bytes");
}

static void serial_ingests_complete_frames(void) {
  ChipvizJoybusBridge bridge;
  ChipvizJoybusSerial serial;
  size_t index;

  chipviz_joybus_init(&bridge);
  chipviz_joybus_serial_init(&serial);
  for (index = 0; index < CHIPVIZ_JOYBUS_FRAME_SIZE - 1u; index++) {
    require(!chipviz_joybus_serial_push(&serial, &bridge, (uint8_t)index), "partial serial frame");
  }
  require(chipviz_joybus_frame_sequence(&bridge) == 0u, "partial does not load");
  require(chipviz_joybus_serial_push(&serial, &bridge, 0xFEu), "complete serial frame loads");
  require(chipviz_joybus_frame_sequence(&bridge) == 1u, "complete increments sequence");
}

static void rejects_bad_inputs(void) {
  ChipvizJoybusBridge bridge;
  uint8_t frame[CHIPVIZ_JOYBUS_FRAME_SIZE] = {0};

  chipviz_joybus_init(&bridge);
  require(!chipviz_joybus_load_frame(0, frame, sizeof(frame)), "null bridge rejected");
  require(!chipviz_joybus_load_frame(&bridge, 0, sizeof(frame)), "null frame rejected");
  require(!chipviz_joybus_load_frame(&bridge, frame, sizeof(frame) - 1u), "short frame rejected");
  require(chipviz_joybus_controller(&bridge, 4) == 0, "bad controller rejected");
}

int main(void) {
  loads_four_controller_buffers();
  builds_poll_and_status_responses();
  serial_ingests_complete_frames();
  rejects_bad_inputs();
  printf("pico joybus tests passed\n");
  return 0;
}
