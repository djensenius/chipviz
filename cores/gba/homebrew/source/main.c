#include <stdint.h>

#define REG_DISPCNT (*(volatile uint16_t *)0x04000000)
#define REG_VCOUNT (*(volatile uint16_t *)0x04000006)
#define REG_KEYINPUT (*(volatile uint16_t *)0x04000130)
#define MODE3 0x0003
#define BG2_ENABLE 0x0400
#define KEY_A 0x0001
#define KEY_B 0x0002
#define KEY_SELECT 0x0004
#define KEY_START 0x0008
#define KEY_RIGHT 0x0010
#define KEY_LEFT 0x0020
#define KEY_UP 0x0040
#define KEY_DOWN 0x0080

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
  uint8_t scene = 0;
  uint8_t palette = 0;

  REG_DISPCNT = MODE3 | BG2_ENABLE;
  for (;;) {
    uint16_t keys = (uint16_t)(~REG_KEYINPUT);
    if (keys & KEY_RIGHT) {
      scene = (uint8_t)((scene + 1u) & 7u);
    }
    if (keys & KEY_LEFT) {
      palette = (uint8_t)((palette + 1u) & 7u);
    }
    wait_vblank();
    for (y = 0; y < 160; y++) {
      for (x = 0; x < 240; x++) {
        const uint8_t tile = (uint8_t)(((x + frame + scene * 11u) / 16 + (y / 12)) & 7);
        const uint8_t pulse = (uint8_t)((frame + x + y + palette * 7u) & 31);
        uint16_t pixel = rgb15((uint8_t)((tile * 3 + pulse + palette * 2u) & 31), (uint8_t)(((y >> 3) + tile + scene) & 31), (uint8_t)(((x >> 3) ^ tile ^ palette) & 31));
        if (((x + frame + scene * 5u) & 31) < 4 || ((y + frame / 2 + palette * 3u) & 31) < 3) {
          pixel = rgb15(31, (uint8_t)((tile * 4u + palette * 3u) & 31), pulse);
        }
        if ((keys & KEY_A) && ((x + y + frame) & 15) < 4) {
          pixel = rgb15(31, 31, pulse);
        }
        if ((keys & KEY_B) && (((x - 120) * (x - 120) + (y - 80) * (y - 80) + frame * 8) & 1023) < 128) {
          pixel = rgb15(pulse, 31, 31);
        }
        framebuffer[y * 240 + x] = pixel;
      }
    }
    for (x = 0; x < 8; x++) {
      int bar_height = 12 + ((frame + x * 23 + scene * 13 + palette * 5) & 63);
      int bx;
      for (y = 0; y < bar_height; y++) {
        for (bx = 0; bx < 16; bx++) {
          framebuffer[(150 - y) * 240 + 16 + x * 26 + bx] = rgb15(31, (uint8_t)(x * 3), (uint8_t)(31 - x * 3));
        }
      }
    }
    if (keys & KEY_START) {
      for (x = 0; x < 240; x++) {
        framebuffer[80 * 240 + x] = rgb15(31, 31, 31);
      }
    }
    if (keys & KEY_SELECT) {
      for (y = 0; y < 160; y++) {
        framebuffer[y * 240 + 120] = rgb15(31, 31, 31);
      }
    }
    frame++;
  }
}
