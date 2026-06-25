#include <stdint.h>

#define REG_DISPCNT (*(volatile uint16_t *)0x04000000)
#define REG_VCOUNT (*(volatile uint16_t *)0x04000006)
#define REG_KEYINPUT (*(volatile uint16_t *)0x04000130)
#define MODE4 0x0004
#define BG2_ENABLE 0x0400
#define DISPLAY_PAGE 0x0010
#define KEY_A 0x0001
#define KEY_B 0x0002
#define KEY_SELECT 0x0004
#define KEY_START 0x0008
#define KEY_RIGHT 0x0010
#define KEY_LEFT 0x0020
#define KEY_UP 0x0040
#define KEY_DOWN 0x0080
#define SCREEN_WIDTH 240
#define SCREEN_HEIGHT 160
#define HALFWORDS_PER_ROW (SCREEN_WIDTH / 2)
#define BLOCK_SIZE 4
#define BLOCK_COLUMNS (SCREEN_WIDTH / BLOCK_SIZE)
#define BLOCK_ROWS (SCREEN_HEIGHT / BLOCK_SIZE)

static volatile uint16_t *const frontbuffer = (volatile uint16_t *)0x06000000;
static volatile uint16_t *const backbuffer = (volatile uint16_t *)0x0600A000;
static volatile uint16_t *const palette_ram = (volatile uint16_t *)0x05000000;

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

static uint8_t color_index(uint8_t r, uint8_t g, uint8_t b) {
  return (uint8_t)((r >> 2) | ((g >> 2) << 3) | ((b >> 3) << 6));
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

static void write_palette(uint16_t frame, const demo_state_t *state) {
  uint16_t i;
  uint8_t warm = (uint8_t)((state->palette * 3u + (frame >> 3)) & 7u);
  for (i = 0; i < 256; i++) {
    uint8_t r = (uint8_t)((((i & 7u) * 4u) + warm) & 31u);
    uint8_t g = (uint8_t)(((((i >> 3) & 7u) * 4u) + state->scene * 3u) & 31u);
    uint8_t b = (uint8_t)(((((i >> 6) & 3u) * 10u) + (state->energy >> 4)) & 31u);
    palette_ram[i] = rgb15(r, g, b);
  }
  palette_ram[color_index(31, 31, 31)] = rgb15(31, 31, 31);
}

static void fill_rect(volatile uint16_t *target, int x, int y, int width, int height, uint8_t color) {
  uint16_t pair;
  int pairs;
  int row;
  int column;

  if (x < 0) {
    width += x;
    x = 0;
  }
  if (y < 0) {
    height += y;
    y = 0;
  }
  if (x + width > SCREEN_WIDTH) {
    width = SCREEN_WIDTH - x;
  }
  if (y + height > SCREEN_HEIGHT) {
    height = SCREEN_HEIGHT - y;
  }
  if (width <= 0 || height <= 0) {
    return;
  }
  if (x & 1) {
    x--;
    width++;
  }
  if (width & 1) {
    width++;
  }

  pair = (uint16_t)(color | ((uint16_t)color << 8));
  pairs = width >> 1;
  for (row = 0; row < height; row++) {
    volatile uint16_t *dest = target + (y + row) * HALFWORDS_PER_ROW + (x >> 1);
    for (column = 0; column < pairs; column++) {
      dest[column] = pair;
    }
  }
}

static void fill_block4(volatile uint16_t *target, int block_x, int block_y, uint8_t color) {
  uint16_t pair = (uint16_t)(color | ((uint16_t)color << 8));
  int offset = block_y * BLOCK_SIZE * HALFWORDS_PER_ROW + block_x * 2;
  volatile uint16_t *row = target + offset;

  row[0] = pair;
  row[1] = pair;
  row += HALFWORDS_PER_ROW;
  row[0] = pair;
  row[1] = pair;
  row += HALFWORDS_PER_ROW;
  row[0] = pair;
  row[1] = pair;
  row += HALFWORDS_PER_ROW;
  row[0] = pair;
  row[1] = pair;
}

static uint8_t block_color(int bx, int by, uint16_t frame, const demo_state_t *state) {
  uint8_t band = (uint8_t)(state->spectrum[((bx >> 3) + (by >> 2)) & 7] >> 5);
  uint8_t phase = (uint8_t)(((frame >> 1) + bx * 3 + by * 5 + state->palette * 7u) & 31u);
  uint8_t r = (uint8_t)((phase + band + (bx >> 1)) & 31u);
  uint8_t g = (uint8_t)((by + band * 3u + state->scene * 4u) & 31u);
  uint8_t b = (uint8_t)(((bx ^ by) + state->palette * 2u) & 31u);

  if (state->scene == 1u) {
    int cx = bx - (BLOCK_COLUMNS / 2);
    int cy = by - (BLOCK_ROWS / 2);
    uint8_t tunnel = (uint8_t)(((cx * cx + cy * cy) >> 1) + (frame >> 2) + state->beat * 5u);
    r = (uint8_t)((tunnel + (state->energy >> 3)) & 31u);
    g = (uint8_t)(((tunnel >> 1) + state->palette * 3u) & 31u);
    b = (uint8_t)(31u - ((tunnel + band) & 31u));
  } else if (state->scene == 2u) {
    uint8_t stripe = (uint8_t)((bx + (frame >> 2) + (state->spectrum[by & 7] >> 4)) & 31u);
    r = (uint8_t)((stripe + state->palette * 2u) & 31u);
    g = (uint8_t)((31u - stripe + band * 2u) & 31u);
    b = (uint8_t)((phase + (stripe >> 1)) & 31u);
  } else if (state->scene == 3u) {
    uint8_t star = (uint8_t)((bx * 11 + by * 17 + frame * 3u + state->note) & 63u);
    r = star < 3u ? 31u : (uint8_t)((phase + band) & 15u);
    g = star < 5u ? (uint8_t)(16u + band * 2u) : (uint8_t)((state->palette * 4u + by) & 31u);
    b = star < 7u ? 31u : (uint8_t)((bx + (state->energy >> 3)) & 31u);
  }

  if ((bx & 7) == 0 || (by & 7) == 0) {
    r = (uint8_t)(24u + state->beat * 7u);
    g = (uint8_t)((band * 4u + state->palette * 2u) & 31u);
    b = phase;
  }

  return color_index(r, g, b);
}

static void render_background(volatile uint16_t *target, uint16_t frame, const demo_state_t *state) {
  int by;
  int bx;

  for (by = 0; by < BLOCK_ROWS; by++) {
    for (bx = 0; bx < BLOCK_COLUMNS; bx++) {
      fill_block4(target, bx, by, block_color(bx, by, frame, state));
    }
  }
}

static void render_overlay(volatile uint16_t *target, uint16_t frame, const demo_state_t *state, uint16_t keys) {
  int x;
  int y;

  for (x = 0; x < 8; x++) {
    int bar_height = 10 + (state->spectrum[x] >> 2);
    uint8_t bar_color = color_index(31, (uint8_t)((x * 3 + state->palette) & 31), (uint8_t)(31 - x * 3));
    uint8_t tick_color = color_index((uint8_t)(state->energy >> 3), 31, 31);
    fill_rect(target, 16 + x * 26, 150 - bar_height, 16, bar_height, bar_color);
    fill_rect(target, 20 + x * 25, 32 + ((frame + x * 13) & 63), 12, 4, tick_color);
  }

  if (state->beat) {
    fill_rect(target, 0, 0, SCREEN_WIDTH, 4, color_index(31, 31, 31));
    fill_rect(target, 0, SCREEN_HEIGHT - 4, SCREEN_WIDTH, 4, color_index(31, 31, 31));
  }
  if (keys & KEY_START) {
    fill_rect(target, 0, 78, SCREEN_WIDTH, 4, color_index(31, 31, 31));
  }
  if (keys & KEY_SELECT) {
    fill_rect(target, 118, 0, 4, SCREEN_HEIGHT, color_index(31, 31, 31));
  }
  if (keys & KEY_A) {
    for (y = 0; y < SCREEN_HEIGHT; y += 12) {
      fill_rect(target, 0, y, SCREEN_WIDTH, 2, color_index(31, 31, 0));
    }
  }
  if (keys & KEY_B) {
    for (x = 0; x < SCREEN_WIDTH; x += 12) {
      fill_rect(target, x, 0, 2, SCREEN_HEIGHT, color_index(0, 31, 31));
    }
  }
}

static void render_frame(volatile uint16_t *target, uint16_t frame, const demo_state_t *state, uint16_t keys) {
  render_background(target, frame, state);
  render_overlay(target, frame, state, keys);
}

int main(void) {
  uint16_t frame = 0;
  uint8_t scene = 0;
  uint8_t palette = 0;
  uint8_t display_page = 0;
  uint16_t prev_keys = 0;

  REG_DISPCNT = MODE4 | BG2_ENABLE;
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
    {
      demo_state_t state;
      volatile uint16_t *draw_target;
      make_demo_state(frame, scene, palette, &state);
      draw_target = display_page ? frontbuffer : backbuffer;
      render_frame(draw_target, frame, &state, keys);
      wait_vblank();
      write_palette(frame, &state);
      display_page ^= 1u;
      REG_DISPCNT = (uint16_t)(MODE4 | BG2_ENABLE | (display_page ? DISPLAY_PAGE : 0u));
    }
    frame++;
  }
}
