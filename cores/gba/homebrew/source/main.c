#include <stdint.h>

#define REG_DISPCNT (*(volatile uint16_t *)0x04000000)
#define MODE3 0x0003
#define BG2_ENABLE 0x0400

static volatile uint16_t *const framebuffer = (volatile uint16_t *)0x06000000;

static uint16_t rgb15(uint8_t r, uint8_t g, uint8_t b) {
  return (uint16_t)((r & 31u) | ((g & 31u) << 5) | ((b & 31u) << 10));
}

int main(void) {
  int x;
  int y;

  REG_DISPCNT = MODE3 | BG2_ENABLE;
  for (;;) {
    for (y = 0; y < 160; y++) {
      for (x = 0; x < 240; x++) {
        const uint8_t band = (uint8_t)((x / 30 + y / 20) & 7);
        framebuffer[y * 240 + x] = rgb15((uint8_t)(x >> 3), (uint8_t)(y >> 3), (uint8_t)(band * 4));
      }
    }
  }
}
