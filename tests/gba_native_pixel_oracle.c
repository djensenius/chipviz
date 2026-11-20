/* Host-only full native-page oracle for mGBA captures. */
#define main gba_production_main
#include "../cores/gba/homebrew/source/main.c"
#undef main

void gba_test_native_pixels(uint16_t frame, uint8_t scene, uint8_t palette, uint16_t keys, uint16_t *out) {
  uint16_t bitmap[SCREEN_HEIGHT * HALFWORDS_PER_ROW];
  uint16_t colours[256];
  demo_state_t state;
  unsigned i;
  make_demo_state(frame, scene, palette, &state);
  render_background_rows(bitmap, frame, &state, 0, BLOCK_ROWS);
  render_overlay(bitmap, frame, &state, keys);
  make_palette(colours, frame, &state);
  for (i = 0; i < SCREEN_HEIGHT * SCREEN_WIDTH; ++i) {
    uint16_t pair = bitmap[i >> 1];
    uint8_t index = (uint8_t)((i & 1u) ? pair >> 8 : pair);
    uint16_t colour = colours[index];
    out[i] = (uint16_t)(((colour & 31u) << 11) |
                        (((colour >> 5) & 31u) << 6) |
                        ((colour >> 10) & 31u));
  }
}
