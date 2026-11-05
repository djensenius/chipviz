#include "input.h"

#include <stdio.h>
#include <stdlib.h>

static unsigned checks;
static unsigned failures;

static void require(int condition, const char *message) {
  checks++;
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    failures++;
  }
}

static void sample(uint16_t keys, uint16_t *prev_keys, uint8_t *scene, uint8_t *palette,
                   uint8_t expected_scene, uint8_t expected_palette, const char *message) {
  update_input(keys, prev_keys, scene, palette);
  require(*scene == expected_scene && *palette == expected_palette, message);
  require(*scene < 4u && *palette < 8u, "selection stays in range without underflow");
  require(*prev_keys == keys, "records every key sample");
}

static void directions_and_wrapping(void) {
  static const uint8_t next_scene[] = {1, 2, 3, 0};
  static const uint8_t prev_scene[] = {3, 0, 1, 2};
  static const uint8_t next_palette[] = {1, 2, 3, 4, 5, 6, 7, 0};
  static const uint8_t prev_palette[] = {7, 0, 1, 2, 3, 4, 5, 6};
  uint8_t s;
  uint8_t p;

  require(KEY_UP == 0x0040 && KEY_DOWN == 0x0080, "scene directions match GBA key bits");
  require(KEY_RIGHT == 0x0010 && KEY_LEFT == 0x0020, "palette directions match GBA key bits");
  require(GBA_SCENE_COUNT == 4u, "renderer has four scenes");
  require(GBA_PALETTE_COUNT == 8u, "renderer has eight palettes");
  for (s = 0; s < 4; s++) {
    for (p = 0; p < 8; p++) {
      uint8_t scene = s;
      uint8_t palette = p;
      uint16_t prev_keys = 0;
      sample(KEY_UP, &prev_keys, &scene, &palette, next_scene[s], p, "Up increments only scene");
      scene = s;
      palette = p;
      prev_keys = 0;
      sample(KEY_DOWN, &prev_keys, &scene, &palette, prev_scene[s], p, "Down decrements only scene");
      scene = s;
      palette = p;
      prev_keys = 0;
      sample(KEY_RIGHT, &prev_keys, &scene, &palette, s, next_palette[p], "Right increments only palette");
      scene = s;
      palette = p;
      prev_keys = 0;
      sample(KEY_LEFT, &prev_keys, &scene, &palette, s, prev_palette[p], "Left decrements only palette");
    }
  }
}

static void pressed_edges_and_opposites(void) {
  static const struct {
    uint16_t key;
    uint16_t opposite;
    uint8_t scene;
    uint8_t palette;
  } cases[] = {
    {KEY_UP, KEY_DOWN, 2, 3},
    {KEY_DOWN, KEY_UP, 0, 3},
    {KEY_RIGHT, KEY_LEFT, 1, 4},
    {KEY_LEFT, KEY_RIGHT, 1, 2},
  };
  unsigned i;

  for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
    uint8_t scene = 1;
    uint8_t palette = 3;
    uint16_t prev_keys = 0;
    unsigned repeat;
    sample(cases[i].key, &prev_keys, &scene, &palette, cases[i].scene, cases[i].palette,
           "fresh edge advances once");
    for (repeat = 0; repeat < 16; repeat++) {
      sample(cases[i].key, &prev_keys, &scene, &palette, cases[i].scene, cases[i].palette,
             "held direction does not repeat");
    }
    sample(0, &prev_keys, &scene, &palette, cases[i].scene, cases[i].palette,
           "release does not advance");
    sample(cases[i].opposite, &prev_keys, &scene, &palette, 1, 3,
           "fresh opposite edge reverses selection");
    sample(0, &prev_keys, &scene, &palette, 1, 3, "release records no input");
    sample(cases[i].key, &prev_keys, &scene, &palette, cases[i].scene, cases[i].palette,
           "repress advances again");
    sample(cases[i].key | cases[i].opposite, &prev_keys, &scene, &palette,
           cases[i].scene, cases[i].palette, "opposite added to held direction is neutral");
    sample(cases[i].opposite, &prev_keys, &scene, &palette, cases[i].scene, cases[i].palette,
           "releasing one opposite does not synthesize an edge");
    sample(0, &prev_keys, &scene, &palette, cases[i].scene, cases[i].palette,
           "release after cancellation is neutral");
    sample(cases[i].opposite, &prev_keys, &scene, &palette, 1, 3,
           "cancelled direction works after release and repress");
  }
}

static void chords(void) {
  static const struct {
    uint16_t keys;
    uint8_t scene;
    uint8_t palette;
  } cases[] = {
    {KEY_UP | KEY_RIGHT, 1, 1},
    {KEY_UP | KEY_LEFT, 1, 7},
    {KEY_DOWN | KEY_RIGHT, 3, 1},
    {KEY_DOWN | KEY_LEFT, 3, 7},
    {KEY_UP | KEY_DOWN, 0, 0},
    {KEY_LEFT | KEY_RIGHT, 0, 0},
    {KEY_UP | KEY_DOWN | KEY_LEFT | KEY_RIGHT, 0, 0},
    {KEY_UP | KEY_DOWN | KEY_RIGHT, 0, 1},
    {KEY_UP | KEY_DOWN | KEY_LEFT, 0, 7},
    {KEY_LEFT | KEY_RIGHT | KEY_UP, 1, 0},
    {KEY_LEFT | KEY_RIGHT | KEY_DOWN, 3, 0},
  };
  unsigned i;

  for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
    uint8_t scene = 0;
    uint8_t palette = 0;
    uint16_t prev_keys = 0;
    uint16_t keys = cases[i].keys;
    sample(keys, &prev_keys, &scene, &palette, cases[i].scene, cases[i].palette,
           "diagonals act independently and opposite axes cancel");
    sample(keys, &prev_keys, &scene, &palette, cases[i].scene, cases[i].palette,
           "held chord does not repeat");
    sample((uint16_t)(keys & (KEY_UP | KEY_RIGHT)), &prev_keys, &scene, &palette,
           cases[i].scene, cases[i].palette, "partial chord release does not advance");
    sample(0, &prev_keys, &scene, &palette, cases[i].scene, cases[i].palette,
           "chord release does not advance");
  }
}

static void no_input_and_unrelated_buttons(void) {
  static const uint16_t buttons[] = {
    0,       /* No input. */
    0x0001,  /* A. */
    0x0002,  /* B. */
    0x0004,  /* Select. */
    0x0008,  /* Start. */
    0x0100,  /* R. */
    0x0200,  /* L. */
    0x0009,  /* Start + A overlay. */
    0x030F,  /* All unrelated buttons together. */
  };
  unsigned i;

  for (i = 0; i < sizeof(buttons) / sizeof(buttons[0]); i++) {
    uint8_t scene = 2;
    uint8_t palette = 5;
    uint16_t prev_keys = 0;
    sample(buttons[i], &prev_keys, &scene, &palette, 2, 5, "unrelated buttons do not select");
    sample(buttons[i], &prev_keys, &scene, &palette, 2, 5, "unrelated held buttons do not select");
    sample(0, &prev_keys, &scene, &palette, 2, 5, "unrelated release does not select");
  }
  {
    uint8_t scene = 0;
    uint8_t palette = 0;
    uint16_t prev_keys = 0;
    sample(0x030F | KEY_UP | KEY_RIGHT, &prev_keys, &scene, &palette, 1, 1,
           "unrelated buttons do not block direction edges");
    sample(KEY_UP | KEY_RIGHT, &prev_keys, &scene, &palette, 1, 1,
           "unrelated releases do not repeat held directions");
    sample(0x030F | KEY_UP | KEY_RIGHT, &prev_keys, &scene, &palette, 1, 1,
           "unrelated edges do not repeat held directions");
  }
}

int main(void) {
  directions_and_wrapping();
  pressed_edges_and_opposites();
  chords();
  no_input_and_unrelated_buttons();
  printf("gba input tests: %u checks, %u failures\n", checks, failures);
  return failures ? EXIT_FAILURE : EXIT_SUCCESS;
}
