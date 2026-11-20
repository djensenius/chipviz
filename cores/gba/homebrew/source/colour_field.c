#include "colour_field.h"

#include <string.h>

#ifdef __arm__
#define GBA_FAST_CODE __attribute__((section(".iwram"), long_call))
#else
#define GBA_FAST_CODE
#endif

#define CF_SCENE 7u
#define CF_STABLE 30u
#define CF_CUT_MIN 20u
#define CF_LEVEL_GAP 10u
#define CF_KEY_BEAT_BAR 0x0003u
#define CF_KEY_ENERGY 0x0200u

static const uint8_t up[] = {0u, 48u, 104u, 168u};
static const uint8_t down[] = {0u, 32u, 88u, 152u};

static void increment(uint16_t *value) {
  if (*value != 0xffffu) {
    ++*value;
  }
}

static uint8_t regions(uint8_t layout) {
  return (layout & 2u) ? 3u : 2u;
}

static uint8_t safe_palette(uint8_t palette) {
  palette &= 7u;
  return (palette & 3u) == 3u ? (uint8_t)(palette - 1u) : palette;
}

static uint8_t energy_for_keys(uint16_t keys) {
  return (keys & CF_KEY_ENERGY) ? 200u : 64u;
}

static uint8_t initial_level(uint8_t energy) {
  uint8_t level = 0u;
  while (level < 3u && energy >= up[level + 1u]) {
    ++level;
  }
  return level;
}

static void mode_entry(gba_cf_t *cf, uint8_t energy, uint8_t power_on) {
  uint8_t level = initial_level(energy);
  cf->action = 0u;
  cf->since_cut = power_on ? CF_CUT_MIN : 0u;
  cf->level_gap = power_on ? CF_LEVEL_GAP : 0u;
  if ((cf->palette & 3u) == 0u) {
    /* All three HID bands equal energy; ties always select bass. */
    cf->combo = 0u;
  }
  cf->level[0] = level;
  cf->level[1] = level;
  cf->level[2] = regions(cf->layout) == 3u ? level : 0u;
}

static void snapshot(gba_cf_t *cf) {
  uint8_t i;
  cf->view.mode = (uint8_t)(cf->palette & 3u);
  cf->view.family = (uint8_t)(cf->palette >> 2);
  cf->view.layout = cf->layout;
  cf->view.combo = cf->combo;
  cf->view.rot = cf->rot;
  for (i = 0u; i < 3u; ++i) {
    cf->view.level[i] = i >= regions(cf->layout) ? 0u :
                        (cf->view.mode == 1u ? cf->level[i] : 3u);
  }
}

static void enter(gba_cf_t *cf, uint8_t scene, uint8_t palette, uint8_t energy, uint8_t power_on) {
  cf->palette = safe_palette(palette);
  cf->wire_value = (uint8_t)(palette & 7u);
  cf->wire_count = scene == CF_SCENE ? 1u : 0u;
  cf->wire_used = 0u;
  cf->request_valid = 0u;
  cf->energy_acc = (uint16_t)((uint16_t)energy << 3);
  cf->transient_armed = 1u;
  cf->layout = cf->combo = cf->rot = 0u;
  mode_entry(cf, energy, power_on);
  snapshot(cf);
}

static void palette_gate(gba_cf_t *cf, uint8_t scene, uint8_t palette, uint8_t energy) {
  uint8_t requested;
  palette &= 7u;
  if (palette != cf->wire_value) {
    cf->wire_value = palette;
    cf->wire_count = 0u;
    cf->wire_used = 0u;
  }
  if (scene != CF_SCENE) {
    cf->wire_count = 0u;
    cf->request_valid = 0u;
    return;
  }
  increment(&cf->wire_count);
  if (cf->wire_count >= CF_STABLE && !cf->wire_used) {
    cf->wire_used = 1u;
    cf->request = safe_palette(palette);
    cf->request_valid = 1u;
  }
  if (!cf->request_valid) {
    return;
  }
  requested = cf->request;
  if (requested == cf->palette) {
    cf->request_valid = 0u;
  } else if (cf->since_cut >= CF_CUT_MIN && cf->level_gap >= CF_LEVEL_GAP) {
    cf->palette = requested;
    cf->request_valid = 0u;
    mode_entry(cf, energy, 0u);
  }
}

static void field_step(gba_cf_t *cf, uint8_t scene, uint8_t palette, uint16_t keys, uint8_t beat) {
  uint8_t energy = energy_for_keys(keys);
  int16_t delta = (int16_t)energy - (int16_t)(cf->energy_acc >> 3);
  uint8_t transient = 0u;
  uint8_t mode = (uint8_t)(cf->palette & 3u);
  uint8_t i;

  cf->energy_acc = (uint16_t)(cf->energy_acc - (cf->energy_acc >> 3) + energy);
  if (cf->transient_armed && delta >= 40) {
    transient = 1u;
    cf->transient_armed = 0u;
  } else if (delta < 16) {
    cf->transient_armed = 1u;
  }
  /* HID energy is always 64 or 200: it cannot enter silence, its bands
   * cannot change dominance, and there are no notes, drop or spectrum. */
  if (mode == 1u && cf->level_gap >= CF_LEVEL_GAP) {
    uint8_t changed = 0u;
    for (i = 0u; i < regions(cf->layout); ++i) {
      uint8_t level = cf->level[i];
      if (level < 3u && energy >= up[level + 1u]) {
        ++cf->level[i];
        changed = 1u;
      } else if (level > 0u && energy < down[level]) {
        --cf->level[i];
        changed = 1u;
      }
    }
    if (changed) {
      cf->level_gap = 0u;
    }
  } else if (mode == 2u && (beat || transient) && cf->since_cut >= CF_CUT_MIN) {
    cf->since_cut = cf->level_gap = 0u;
    if (cf->action == 1u) {
      cf->combo = (uint8_t)((cf->combo + 1u) & 3u);
    } else if (cf->action == 3u) {
      cf->layout = (uint8_t)((cf->layout + 1u) & 3u);
      cf->rot = (uint8_t)(cf->rot % regions(cf->layout));
    } else {
      cf->rot = (uint8_t)((cf->rot + 1u) % regions(cf->layout));
    }
    cf->action = (uint8_t)((cf->action + 1u) & 3u);
  }
  increment(&cf->since_cut);
  increment(&cf->level_gap);
  snapshot(cf);
  palette_gate(cf, scene, palette, energy);
}

void gba_cf_init(gba_cf_t *cf) {
  memset(cf, 0, sizeof(*cf));
  cf->since_switch = 0xffffu;
}

uint8_t gba_cf_step(gba_cf_t *cf, uint8_t scene, uint8_t palette, uint16_t keys) {
  uint8_t shown;
  uint8_t beat = (uint8_t)((keys & (uint16_t)~cf->previous_keys & CF_KEY_BEAT_BAR) != 0u);
  cf->previous_keys = keys;
  if (!cf->started) {
    cf->started = 1u;
    cf->scene_value = scene;
    cf->scene_count = 1u;
    if (scene == CF_SCENE) {
      cf->shown = 1u;
      enter(cf, scene, palette, energy_for_keys(keys), 1u);
    }
    return cf->shown;
  }
  if (scene == cf->scene_value) {
    increment(&cf->scene_count);
  } else {
    cf->scene_value = scene;
    cf->scene_count = 1u;
  }
  shown = cf->shown;
  if (shown) {
    if (cf->entering) {
      cf->entering = 0u;
      enter(cf, scene, palette, energy_for_keys(keys), 0u);
    } else {
      field_step(cf, scene, palette, keys, beat);
    }
  }
  increment(&cf->since_switch);
  if ((uint8_t)(scene == CF_SCENE) != shown && cf->scene_count >= CF_STABLE &&
      cf->since_switch >= CF_CUT_MIN) {
    cf->since_switch = 0u;
    cf->shown = (uint8_t)!shown;
    cf->entering = cf->shown;
  }
  return shown;
}

uint8_t gba_cf_pixel_index(unsigned x, unsigned y) {
  uint8_t column = x < 80u ? 0u : (x < 120u ? 1u : (x < 160u ? 2u : 3u));
  uint8_t row = y < 56u ? 0u : (y < 80u ? 1u : (y < 112u ? 2u : 3u));
  return (uint8_t)(row * 4u + column);
}

GBA_FAST_CODE void gba_cf_render_bitmap(volatile uint16_t *target) {
  static const uint8_t pairs[4] = {40u, 20u, 20u, 40u};
  unsigned y;
  for (y = 0u; y < 160u; ++y) {
    uint8_t row = (uint8_t)(gba_cf_pixel_index(0u, y) >> 2);
    unsigned column;
    for (column = 0u; column < 4u; ++column) {
      uint8_t index = (uint8_t)(row * 4u + column);
      uint16_t pair = (uint16_t)(index | ((uint16_t)index << 8));
      unsigned count;
      for (count = 0u; count < pairs[column]; ++count) {
        *target++ = pair;
      }
    }
  }
}

uint16_t gba_cf_index_colour(const gba_cf_view_t *view, uint8_t index) {
  /* BGR555 values and the exact combo table from colour-field-v0. */
  static const uint16_t colours[11] = {
    0x105bu, 0x033fu, 0x5cc2u, 0x7fffu, 0x0000u,
    0x001fu, 0x03e0u, 0x7c00u, 0x7fe0u, 0x7c1fu, 0x03ffu
  };
  static const uint8_t combos[2][2][4][3] = {
    {{{0, 2, 0}, {1, 2, 0}, {0, 1, 0}, {2, 3, 0}},
     {{0, 1, 2}, {0, 3, 2}, {1, 4, 2}, {0, 4, 1}}},
    {{{5, 8, 0}, {6, 9, 0}, {7, 10, 0}, {8, 9, 0}},
     {{5, 6, 7}, {8, 9, 10}, {5, 4, 8}, {7, 4, 10}}}
  };
  static const uint8_t map[4][4] = {
    {0, 0, 1, 1}, {1, 1, 0, 0}, {0, 1, 1, 2}, {2, 1, 1, 0}
  };
  uint8_t layout = view->layout;
  uint8_t cls = (layout & 1u) ? (uint8_t)(index >> 2) : (uint8_t)(index & 3u);
  uint8_t region = map[layout][cls];
  uint8_t n = regions(layout);
  uint8_t entry = (uint8_t)((region + view->rot) % n);
  uint8_t level = view->level[region];
  uint16_t colour = colours[combos[view->family][n - 2u][view->combo][entry]];
  uint16_t r = (uint16_t)(((colour & 31u) * level + 1u) / 3u);
  uint16_t g = (uint16_t)((((colour >> 5) & 31u) * level + 1u) / 3u);
  uint16_t b = (uint16_t)((((colour >> 10) & 31u) * level + 1u) / 3u);
  return (uint16_t)(r | (g << 5) | (b << 10));
}
