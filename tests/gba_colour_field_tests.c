#include "colour_field.h"
#include "input.h"

#include <stdio.h>
#include <stdlib.h>

static unsigned checks, failures;

static void require(int condition, const char *message) {
  ++checks;
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    ++failures;
  }
}

static void hid_fixture(void) {
  static const unsigned cuts[] = {50, 110, 170, 230, 290, 320, 350, 410, 470, 530, 590};
  gba_cf_t cf;
  unsigned step, count = 0;
  gba_cf_view_t previous;
  gba_cf_init(&cf);
  for (step = 0; step < 600; ++step) {
    uint16_t keys = (step >= 320 && step < 380) ? 0x0200u : 0u;
    if (step >= 50 && (step - 50) % 60 == 0) {
      keys |= 1u;
    }
    require(gba_cf_step(&cf, 7, 1, keys), "F19 spectral stays shown");
    if (step == 0 || step == 319 || step == 390) {
      require(cf.view.level[0] == 1 && cf.view.level[1] == 1, "F19 spectral rests at 1");
    }
    if (step == 320 || step == 380) {
      require(cf.view.level[0] == 2 && cf.view.level[1] == 2, "F19 spectral steps to 2");
    }
    if (step == 330 || step == 379) {
      require(cf.view.level[0] == 3 && cf.view.level[1] == 3, "F19 spectral reaches 3");
    }
  }
  gba_cf_init(&cf);
  previous = cf.view;
  for (step = 0; step < 600; ++step) {
    uint16_t keys = (step >= 320 && step < 380) ? 0x0200u : 0u;
    if (step >= 50 && (step - 50) % 60 == 0) {
      keys |= 1u;
    }
    require(gba_cf_step(&cf, 7, 2, keys), "F19 cuts stay shown");
    if (step && (cf.view.rot != previous.rot || cf.view.combo != previous.combo ||
                 cf.view.layout != previous.layout)) {
      require(count < sizeof(cuts) / sizeof(cuts[0]) && cuts[count] == step, "F19 exact cut step");
      ++count;
    }
    previous = cf.view;
  }
  require(count == sizeof(cuts) / sizeof(cuts[0]), "F19 eleven cuts including shoulder transient");
}

static void scene_gate(void) {
  gba_cf_t cf;
  unsigned step;
  gba_cf_init(&cf);
  for (step = 0; step < 600; ++step) {
    uint8_t scene = step < 100 ? 6 : step < 200 ? (step & 1u ? 6 : 7) :
                    step < 300 ? 7 : step < 320 ? 6 : step < 450 ? 7 : 6;
    uint8_t shown = gba_cf_step(&cf, scene, 0, 0);
    require(shown == (step >= 230 && step < 480), "F21 scene debounce and delayed entry");
  }
}

static void relative_palette(void) {
  gba_cf_t cf;
  unsigned step;
  gba_cf_init(&cf);
  for (step = 0; step < 600; ++step) {
    uint8_t palette = step < 10 ? 0 : step < 20 ? 1 : step < 30 ? 2 : step < 130 ? 3 : 2;
    require(gba_cf_step(&cf, 7, palette, 0), "F24 field stays shown");
    require(cf.view.mode == (step < 60 ? 0 : 2), "F24 relative palette stability and no flash");
  }
}

static void timing_and_entry(void) {
  gba_cf_t cf;
  unsigned step, last = 0, changes = 0;
  gba_cf_view_t previous;
  gba_cf_init(&cf);
  for (step = 0; step < 200; ++step) {
    uint16_t keys = step < 10 ? 1u : (step & 1u);
    require(gba_cf_step(&cf, 7, 2, keys), "entry fixture shown");
    if (step == 0) {
      previous = cf.view;
    } else if (cf.view.rot != previous.rot || cf.view.combo != previous.combo ||
               cf.view.layout != previous.layout) {
      require(step >= 11 && (!changes || step - last >= 20), "held entry beat and cut budget");
      last = step;
      ++changes;
      previous = cf.view;
    }
  }
  require(changes > 0, "fresh beats produce cuts");
  for (step = 0; step < 8; ++step) {
    gba_cf_init(&cf);
    require(gba_cf_step(&cf, 7, (uint8_t)step, 0), "power-on field entry");
    require(cf.view.mode != 3, "no palette can enable experimental flash");
    require(cf.view.family == step / 4, "power-on family");
  }
}

static void palette_window_and_entry_edges(void) {
  gba_cf_t cf;
  unsigned step;
  gba_cf_init(&cf);
  for (step = 0; step < 100; ++step) {
    uint8_t scene = step == 30 ? 6 : 7;
    uint8_t palette = step == 0 ? 0 : 1;
    require(gba_cf_step(&cf, scene, palette, 0), "short scene departure does not exit");
    require(cf.view.mode == (step < 61 ? 0 : 1), "scene window resets palette stability");
  }
  gba_cf_init(&cf);
  for (step = 0; step < 80; ++step) {
    uint8_t palette = step == 0 ? 0 : step < 32 ? 1 : 4;
    require(gba_cf_step(&cf, 7, palette, 0), "mode and family fixture stays shown");
    require(cf.view.mode == (step < 31 ? 0 : step < 62 ? 1 : 0), "mode requests apply after stability");
    require(cf.view.family == (step < 62 ? 0 : 1), "family request applies with mode");
  }
  gba_cf_init(&cf);
  gba_cf_step(&cf, 7, 2, 1);
  gba_cf_step(&cf, 7, 2, 1);
  require(cf.view.rot == 0, "beat held at entry raises no event");
  gba_cf_step(&cf, 7, 2, 3);
  require(cf.view.rot == 1, "bar edge is independent of a held beat");
}

static void pad_ring_integration(void) {
  gba_cf_t cf;
  uint16_t previous_keys = 0;
  uint8_t scene = 0, palette = 0;
  unsigned step;
  gba_cf_init(&cf);
  for (step = 0; step < 100; ++step) {
    uint16_t keys = (step == 1 || step == 3 || step == 5 || step == 7) ? KEY_UP :
                    (step == 40 || step == 42) ? KEY_RIGHT : 0u;
    uint8_t shown;
    update_input(keys, &previous_keys, &scene, &palette);
    shown = gba_cf_step(&cf, scene, palette, keys);
    require(shown == (step >= 37), "real scene ring pulses enter after 30 stable polls");
    if (shown) {
      require(cf.view.mode == (step < 72 ? 0 : 2), "real palette pulses pass the stability gate");
    }
  }
}

static uint16_t scale(uint16_t colour, uint8_t level) {
  unsigned r = ((colour & 31u) * level + 1u) / 3u;
  unsigned g = (((colour >> 5) & 31u) * level + 1u) / 3u;
  unsigned b = (((colour >> 10) & 31u) * level + 1u) / 3u;
  return (uint16_t)(r | (g << 5) | (b << 10));
}

static void colours_and_geometry(void) {
  static const uint16_t combos[2][2][4][3] = {
    {{{0x105b, 0x5cc2, 0}, {0x033f, 0x5cc2, 0}, {0x105b, 0x033f, 0}, {0x5cc2, 0x7fff, 0}},
     {{0x105b, 0x033f, 0x5cc2}, {0x105b, 0x7fff, 0x5cc2}, {0x033f, 0, 0x5cc2}, {0x105b, 0, 0x033f}}},
    {{{0x001f, 0x7fe0, 0}, {0x03e0, 0x7c1f, 0}, {0x7c00, 0x03ff, 0}, {0x7fe0, 0x7c1f, 0}},
     {{0x001f, 0x03e0, 0x7c00}, {0x7fe0, 0x7c1f, 0x03ff}, {0x001f, 0, 0x7fe0}, {0x7c00, 0, 0x03ff}}}
  };
  static uint16_t bitmap[240u * 160u / 2u];
  gba_cf_view_t view = {0};
  unsigned layout, family, combo, rot, x, y;
  gba_cf_render_bitmap(bitmap);
  for (y = 0; y < 160; ++y) {
    for (x = 0; x < 240; ++x) {
      uint16_t pair = bitmap[y * 120u + x / 2u];
      require(((pair >> ((x & 1u) * 8u)) & 0xffu) == gba_cf_pixel_index(x, y),
              "prepared framebuffer index matches every pixel");
    }
  }
  for (layout = 0; layout < 4; ++layout) {
    unsigned n = layout >= 2 ? 3 : 2;
    unsigned size = layout & 1u ? 160u : 240u;
    view.layout = (uint8_t)layout;
    for (family = 0; family < 2; ++family) {
      view.family = (uint8_t)family;
      for (combo = 0; combo < 4; ++combo) {
        view.combo = (uint8_t)combo;
        for (rot = 0; rot < n; ++rot) {
          view.rot = (uint8_t)rot;
          view.level[0] = 1;
          view.level[1] = 2;
          view.level[2] = 3;
          for (y = 0; y < 160; ++y) {
            for (x = 0; x < 240; ++x) {
              unsigned position = layout & 1u ? 159u - y : x;
              unsigned region = 0, boundary;
              for (boundary = 1; boundary < n; ++boundary) {
                if (position >= 8u * ((boundary * (size / 8u)) / n)) {
                  ++region;
                }
              }
              require(gba_cf_index_colour(&view, gba_cf_pixel_index(x, y)) ==
                      scale(combos[family][n - 2u][combo][(region + rot) % n], view.level[region]),
                      "every pixel matches spec boundary, combo and channel scaling");
            }
          }
        }
      }
    }
  }
}

int main(void) {
  hid_fixture();
  scene_gate();
  relative_palette();
  timing_and_entry();
  palette_window_and_entry_edges();
  pad_ring_integration();
  colours_and_geometry();
  printf("gba colour field tests: %u checks, %u failures\n", checks, failures);
  return failures ? EXIT_FAILURE : EXIT_SUCCESS;
}
