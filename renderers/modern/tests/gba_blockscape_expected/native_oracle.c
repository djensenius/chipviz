#include <stdio.h>
#include <stdint.h>

#define main unused_gba_entry
#include "../../../../cores/gba/homebrew/source/main.c"
#undef main

int main(void) {
  unsigned frame, scene, palette, energy, beat, note, keys, bins[8];
  uint16_t screen[SCREEN_WIDTH * SCREEN_HEIGHT / 2];
  uint16_t colours[256];
  demo_state_t state;
  unsigned i;

  if (scanf("%u %u %u %u %u %u %u", &frame, &scene, &palette, &energy, &beat,
            &note, &keys) != 7) {
    return 1;
  }
  for (i = 0; i < 8; ++i) {
    if (scanf("%u", &bins[i]) != 1) {
      return 1;
    }
    state.spectrum[i] = (uint8_t)bins[i];
  }
  state.scene = (uint8_t)scene;
  state.palette = (uint8_t)palette;
  state.energy = (uint8_t)energy;
  state.beat = (uint8_t)beat;
  state.note = (uint8_t)note;
  render_background_rows(screen, (uint16_t)frame, &state, 0, BLOCK_ROWS);
  render_overlay(screen, (uint16_t)frame, &state, (uint16_t)keys);
  make_palette(colours, (uint16_t)frame, &state);

  for (i = 0; i < SCREEN_WIDTH * SCREEN_HEIGHT; ++i) {
    unsigned pair = screen[i / 2];
    if (putchar((int)((pair >> ((i & 1u) * 8)) & 255u)) == EOF) {
      return 1;
    }
  }
  for (i = 0; i < 256; ++i) {
    if (putchar(colours[i] & 255u) == EOF || putchar(colours[i] >> 8) == EOF) {
      return 1;
    }
  }
  return ferror(stdout) ? 1 : 0;
}
