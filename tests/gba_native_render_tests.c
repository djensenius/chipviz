/* Exercise the production renderer and its incremental prefetch schedule. */
#define main gba_production_main
#include "../cores/gba/homebrew/source/main.c"
#undef main

#include <stdio.h>
#include <string.h>

static uint16_t visible[SCREEN_HEIGHT * HALFWORDS_PER_ROW];
static uint16_t hidden[SCREEN_HEIGHT * HALFWORDS_PER_ROW];
static uint16_t expected[SCREEN_HEIGHT * HALFWORDS_PER_ROW];

static int cancelled_page_is_replaced(void) {
  gba_native_prefetch_t p;
  demo_state_t state;
  uint8_t first, end;
  unsigned poll;
  memset(hidden, 0xa5, sizeof(hidden));
  make_demo_state(37, 3, 1, &state);
  gba_native_prefetch_init(&p);
  gba_native_prefetch_observe(&p, 3, 1, KEY_A, 27);
  for (poll = 0; poll < 3; ++poll) {
    if (gba_native_prefetch_next(&p, &first, &end) != GBA_PREFETCH_BACKGROUND) {
      return 0;
    }
    render_background_rows(hidden, 37, &state, first, end);
  }
  gba_native_prefetch_observe(&p, 7, 1, 0, 1);
  if (p.active || p.ready) {
    return 0;
  }
  gba_native_prefetch_observe(&p, 2, 6, KEY_A | KEY_B, 27);
  make_demo_state(37, p.scene, p.palette, &state);
  render_background_rows(expected, 37, &state, 0, BLOCK_ROWS);
  render_overlay(expected, 37, &state, p.keys);
  while (!p.ready) {
    if (gba_native_prefetch_next(&p, &first, &end) != GBA_PREFETCH_BACKGROUND) {
      return 0;
    }
    render_background_rows(hidden, 37, &state, first, end);
    if (end == GBA_PREFETCH_ROWS) {
      render_overlay(hidden, 37, &state, p.keys);
      gba_native_prefetch_complete(&p);
    }
  }
  return memcmp(hidden, expected, sizeof(hidden)) == 0;
}

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
        gba_native_prefetch_observe(&p, (uint8_t)scene, (uint8_t)palette, (uint16_t)keys, 27);
        while (!p.ready) {
          work = gba_native_prefetch_next(&p, &first, &end);
          if (work == GBA_PREFETCH_BACKGROUND) {
            render_background_rows(hidden, frame, &state, first, end);
            if (end == GBA_PREFETCH_ROWS) {
              render_overlay(hidden, frame, &state, p.keys);
              gba_native_prefetch_complete(&p);
            }
          } else {
            puts("gba native render tests: stalled");
            return 1;
          }
        }
        if (memcmp(hidden, expected, sizeof(hidden)) != 0) {
          printf("gba native render tests: mismatch scene %u palette %u keys %u\n", scene, palette, keys);
          return 1;
        }
        /* Candidate late-snapshot schedule: four 10-row batches followed by
         * overlays. This proves pixels only, not the GBA per-poll time bound. */
        memset(hidden, 0xa5, sizeof(hidden));
        for (int batch = 0; batch < 4; ++batch) {
          render_background_rows(hidden, frame, &state, batch * 10, (batch + 1) * 10);
        }
        render_overlay(hidden, frame, &state, (uint16_t)keys);
        if (memcmp(hidden, expected, sizeof(hidden)) != 0) {
          printf("gba native render tests: late schedule mismatch scene %u palette %u keys %u\n",
                 scene, palette, keys);
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
  printf("gba native render tests: %u page cases and four-batch candidate passed\n", cases);
  if (!cancelled_page_is_replaced()) {
    puts("gba native render tests: cancelled page was not completely replaced");
    return 1;
  }
  puts("gba native render tests: cancelled hidden page replaced completely");
  {
    demo_state_t state;
    unsigned scene_differences = 0, palette_differences = 0;
    make_demo_state(37, 3, 0, &state);
    render_background_rows(expected, 37, &state, 0, BLOCK_ROWS);
    make_demo_state(37, 2, 0, &state);
    render_background_rows(hidden, 37, &state, 0, BLOCK_ROWS);
    for (size_t i = 0; i < sizeof(hidden) / sizeof(hidden[0]); ++i) {
      scene_differences += hidden[i] != expected[i];
    }
    make_demo_state(37, 3, 1, &state);
    render_background_rows(hidden, 37, &state, 0, BLOCK_ROWS);
    for (size_t i = 0; i < sizeof(hidden) / sizeof(hidden[0]); ++i) {
      palette_differences += hidden[i] != expected[i];
    }
    if (!scene_differences || !palette_differences) {
      puts("gba native render tests: expected native scene and palette differences");
      return 1;
    }
    printf("gba native render tests: late scene differs in %u halfwords; palette in %u\n",
           scene_differences, palette_differences);
  }
  return 0;
}
