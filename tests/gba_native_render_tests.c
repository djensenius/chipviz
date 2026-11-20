/* Exercise the production renderer and its incremental prefetch schedule. */
#define main gba_production_main
#include "../cores/gba/homebrew/source/main.c"
#undef main

#include <stdio.h>
#include <string.h>

static uint16_t visible[SCREEN_HEIGHT * HALFWORDS_PER_ROW];
static uint16_t hidden[SCREEN_HEIGHT * HALFWORDS_PER_ROW];
static uint16_t expected[SCREEN_HEIGHT * HALFWORDS_PER_ROW];

int main(void) {
  unsigned scene, palette, keys, cases = 0;
  for (scene = 0; scene < 4; ++scene) {
    for (palette = 0; palette < 8; ++palette) {
      for (keys = 0; keys < 16; ++keys) {
        gba_native_prefetch_t p;
        demo_state_t state;
        uint8_t first, end, work;
        uint16_t frame = (uint16_t)(scene * 97u + palette * 33u + keys);
        memset(visible, 0x5a, sizeof(visible));
        memset(hidden, 0xa5, sizeof(hidden));
        memset(expected, 0xa5, sizeof(expected));
        make_demo_state(frame, (uint8_t)scene, (uint8_t)palette, &state);
        render_background_rows(expected, frame, &state, 0, BLOCK_ROWS);
        render_overlay(expected, frame, &state, (uint16_t)keys);

        gba_native_prefetch_init(&p);
        gba_native_prefetch_observe(&p, (uint8_t)scene, (uint8_t)palette, (uint16_t)keys);
        while (!p.ready) {
          work = gba_native_prefetch_next(&p, &first, &end);
          if (work == GBA_PREFETCH_BACKGROUND) {
            render_background_rows(hidden, frame, &state, first, end);
          } else if (work == GBA_PREFETCH_OVERLAY) {
            render_overlay_items(hidden, frame, &state, p.keys, first, end);
          } else {
            puts("gba native render tests: stalled");
            return 1;
          }
        }
        if (memcmp(hidden, expected, sizeof(hidden)) != 0) {
          printf("gba native render tests: mismatch scene %u palette %u keys %u\n", scene, palette, keys);
          return 1;
        }
        for (size_t i = 0; i < sizeof(visible) / sizeof(visible[0]); ++i) {
          if (visible[i] != 0x5a5au) {
            puts("gba native render tests: visible page modified");
            return 1;
          }
        }
        ++cases;
      }
    }
  }
  printf("gba native render tests: %u page cases passed\n", cases);
  return 0;
}
