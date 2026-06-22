#include "chipviz_pico/joybus_bridge.h"

#include <stdint.h>

#ifdef PICO_SDK_VERSION_STRING
#include "pico/stdlib.h"
#endif

int main(void) {
  ChipvizJoybusBridge bridge;
  ChipvizJoybusSerial serial;

  chipviz_joybus_init(&bridge);
  chipviz_joybus_serial_init(&serial);

#ifdef PICO_SDK_VERSION_STRING
  stdio_init_all();
  for (;;) {
    int byte = getchar_timeout_us(0);
    if (byte >= 0) {
      (void)chipviz_joybus_serial_push(&serial, &bridge, (uint8_t)byte);
    }
    tight_loop_contents();
  }
#else
  return 0;
#endif
}
