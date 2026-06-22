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
  require(chipviz_joybus_load_frame(&bridge, frame, sizeof(frame)), "frame loads");
  controller = chipviz_joybus_controller(&bridge, 2);
  require(controller != 0, "controller pointer returned");
  require(controller[0] == 9u, "controller 3 byte 0");
  require(controller[3] == 12u, "controller 3 byte 3");
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
  rejects_bad_inputs();
  printf("pico joybus tests passed\n");
  return 0;
}
