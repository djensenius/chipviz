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

typedef struct {
  uint8_t spectrum[8];
  uint8_t scene;
  uint8_t palette;
  uint8_t energy;
  uint8_t beat;
  uint8_t note;
} demo_state_t;

static uint16_t rgb15(uint8_t r, uint8_t g, uint8_t b) {
  return (uint16_t)((r & 31u) | ((g & 31u) << 5) | ((b & 31u) << 10));
}

static void wait_vblank(void) {
  while (REG_VCOUNT >= 160) {
  }
  while (REG_VCOUNT < 160) {
  }
}

static void make_demo_state(uint16_t frame, uint8_t scene_override, uint8_t palette_override, demo_state_t *state) {
  int band;
  uint8_t phrase = (uint8_t)((frame / 96u) & 3u);
  uint8_t phase = (uint8_t)(frame & 255u);

  state->scene = (uint8_t)((phrase + scene_override) & 3u);
  state->palette = (uint8_t)(((frame / 32u) + palette_override) & 7u);
  state->energy = (uint8_t)(80u + ((frame * 5u + phrase * 23u) & 95u));
  state->beat = (uint8_t)((phase & 31u) < 4u);
  state->note = (uint8_t)(48u + ((frame / 16u) & 23u));
  for (band = 0; band < 8; band++) {
    uint8_t wave = (uint8_t)((frame * (uint16_t)(band + 3) + band * 29u) & 127u);
    state->spectrum[band] = (uint8_t)(48u + (wave < 64u ? wave : 127u - wave) + phrase * 16u);
  }
}

static uint16_t scene_pixel(int x, int y, uint16_t frame, const demo_state_t *state, uint16_t keys) {
  uint8_t pulse = (uint8_t)((frame + x + y + state->palette * 9u) & 31u);
  uint8_t band = state->spectrum[((x / 30) + (y / 20)) & 7] >> 3;
  uint8_t r = (uint8_t)((pulse + band + state->palette * 2u) & 31u);
  uint8_t g = (uint8_t)(((y >> 3) + state->scene * 4u + band) & 31u);
  uint8_t b = (uint8_t)(((x >> 3) ^ band ^ state->palette) & 31u);

  if (state->scene == 1u) {
    int cx = x - 120;
    int cy = y - 80;
    uint16_t tunnel = (uint16_t)((cx * cx + cy * cy + frame * (6u + state->beat * 6u)) >> 7);
    r = (uint8_t)((tunnel + state->energy / 8u) & 31u);
    g = (uint8_t)((tunnel / 2u + state->palette * 3u) & 31u);
    b = (uint8_t)(31u - ((tunnel + band) & 31u));
  } else if (state->scene == 2u) {
    uint8_t stripe = (uint8_t)(((x + frame + state->spectrum[(y >> 4) & 7]) >> 3) & 31u);
    r = (uint8_t)((stripe + state->palette * 2u) & 31u);
    g = (uint8_t)((31u - stripe + band) & 31u);
    b = (uint8_t)((pulse + stripe / 2u) & 31u);
  } else if (state->scene == 3u) {
    uint8_t star = (uint8_t)((x * 7 + y * 13 + frame * 5 + state->note) & 63);
    r = star < 3 ? 31u : (uint8_t)((pulse + band) & 15u);
    g = star < 5 ? (uint8_t)(16u + band) : (uint8_t)((state->palette * 4u + y / 12u) & 31u);
    b = star < 7 ? 31u : (uint8_t)((x / 9u + state->energy / 8u) & 31u);
  }

  if (((x + frame + state->scene * 5u) & 31) < (3 + state->beat * 4) ||
      ((y + frame / 2 + state->palette * 3u) & 31) < 2) {
    r = 31;
    g = (uint8_t)((band + state->palette * 3u) & 31u);
    b = pulse;
  }
  if ((keys & KEY_A) && ((x + y + frame) & 15) < 4) {
    r = 31;
    g = 31;
  }
  if ((keys & KEY_B) && (((x - 120) * (x - 120) + (y - 80) * (y - 80) + frame * 8) & 1023) < 128) {
    g = 31;
    b = 31;
  }
  return rgb15(r, g, b);
}

int main(void) {
  uint16_t frame = 0;
  int x;
  int y;
  uint8_t scene = 0;
  uint8_t palette = 0;
  uint16_t prev_keys = 0;

  REG_DISPCNT = MODE3 | BG2_ENABLE;
  for (;;) {
    uint16_t keys = (uint16_t)(~REG_KEYINPUT & 0x03FF);
    uint16_t pressed = (uint16_t)(keys & ~prev_keys);
    prev_keys = keys;
    if (pressed & KEY_RIGHT) {
      scene = (uint8_t)((scene + 1u) & 7u);
    }
    if (pressed & KEY_LEFT) {
      palette = (uint8_t)((palette + 1u) & 7u);
    }
    wait_vblank();
    {
      demo_state_t state;
      make_demo_state(frame, scene, palette, &state);
    for (y = 0; y < 160; y++) {
      for (x = 0; x < 240; x++) {
        framebuffer[y * 240 + x] = scene_pixel(x, y, frame, &state, keys);
      }
    }
    for (x = 0; x < 8; x++) {
      int bar_height = 10 + (state.spectrum[x] >> 2);
      int bx;
      for (y = 0; y < bar_height; y++) {
        for (bx = 0; bx < 16; bx++) {
          framebuffer[(150 - y) * 240 + 16 + x * 26 + bx] = rgb15(31, (uint8_t)((x * 3 + state.palette) & 31), (uint8_t)(31 - x * 3));
        }
      }
      for (bx = 0; bx < 12; bx++) {
        framebuffer[(32 + ((frame + x * 13) & 63)) * 240 + 20 + x * 25 + bx] = rgb15((uint8_t)(state.energy >> 3), 31, 31);
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
    }
    frame++;
  }
}
