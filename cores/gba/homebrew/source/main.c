#include <stdint.h>

#define REG_DISPCNT (*(volatile uint16_t *)0x04000000)
#define REG_VCOUNT (*(volatile uint16_t *)0x04000006)
#define MODE3 0x0003
#define BG2_ENABLE 0x0400

static volatile uint16_t *const framebuffer = (volatile uint16_t *)0x06000000;

static uint16_t rgb15(uint8_t r, uint8_t g, uint8_t b) {
  return (uint16_t)((r & 31u) | ((g & 31u) << 5) | ((b & 31u) << 10));
}

static void wait_vblank(void) {
  while (REG_VCOUNT >= 160) {
  }
  while (REG_VCOUNT < 160) {
  }
}

int main(void) {
  uint16_t frame = 0;
  int x;
  int y;

  REG_DISPCNT = MODE3 | BG2_ENABLE;
  for (;;) {
    wait_vblank();
    for (y = 0; y < 160; y++) {
      for (x = 0; x < 240; x++) {
        const uint8_t tile = (uint8_t)(((x + frame) / 16 + (y / 12)) & 7);
        const uint8_t pulse = (uint8_t)((frame + x + y) & 31);
        uint16_t pixel = rgb15((uint8_t)((tile * 3 + pulse) & 31), (uint8_t)((y >> 3) + tile), (uint8_t)((x >> 3) ^ tile));
        if (((x + frame) & 31) < 4 || ((y + frame / 2) & 31) < 3) {
          pixel = rgb15(31, (uint8_t)(tile * 4), pulse);
        }
        framebuffer[y * 240 + x] = pixel;
      }
    }
    for (x = 0; x < 8; x++) {
      int bar_height = 12 + ((frame + x * 23) & 63);
      int bx;
      for (y = 0; y < bar_height; y++) {
        for (bx = 0; bx < 16; bx++) {
          framebuffer[(150 - y) * 240 + 16 + x * 26 + bx] = rgb15(31, (uint8_t)(x * 3), (uint8_t)(31 - x * 3));
        }
      }
    }
    frame++;
  }
}
