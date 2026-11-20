#include "../cores/gba/homebrew/source/colour_field.h"
#include "../cores/c64/homebrew/src/colour_field.h"

#include <stdio.h>
#include <string.h>

/* Compare the GBA's deliberately reduced digital-only engine with the
 * existing full detector fed the same mapped HID polls. This is synthetic
 * controller input, not a claim that the GBA receives full control frames. */
int main(void) {
  gba_cf_t gba;
  uint32_t random = 1u;
  unsigned step, shown_steps = 0, mismatches = 0;
  unsigned scene_left = 0, palette_left = 0, energy_left = 0;
  uint8_t scene = 0, palette = 0;
  uint16_t keys = 0, previous_keys = 0;
  gba_cf_init(&gba);
  cf_init();
  for (step = 0; step < 30000u; ++step) {
    uint8_t shown, reference;
    uint8_t energy;
    unsigned i;
    random = random * 1664525u + 1013904223u;
    if (!scene_left--) {
      scene = ((random >> 24) & 3u) ? 7u : (uint8_t)((random >> 20) & 3u);
      scene_left = 30u + ((random >> 8) % 300u);
    }
    if (!palette_left--) {
      palette = (uint8_t)((random >> 16) & 7u);
      palette_left = 20u + ((random >> 6) % 130u);
    }
    if (!energy_left--) {
      keys ^= 0x0200u;
      energy_left = 1u + ((random >> 12) % 60u);
    }
    keys = (uint16_t)((keys & 0x0200u) | ((random >> 27) & 3u));
    energy = (keys & 0x0200u) ? 200u : 64u;
    memset(&cf_in, 0, sizeof(cf_in));
    cf_in.scene = scene;
    cf_in.palette = palette;
    cf_in.wire = 1u;
    cf_in.energy = energy;
    for (i = 0; i < 3u; ++i) {
      cf_in.band[i] = energy;
    }
    if (keys & (uint16_t)~previous_keys & 3u) {
      cf_in.events = CF_EV_B;
    }
    previous_keys = keys;
    reference = cf_tick();
    shown = gba_cf_step(&gba, scene, palette, keys);
    if (shown) {
      ++shown_steps;
    }
    if (shown != reference ||
        (shown && (gba.view.mode != cf.view.mode || gba.view.family != cf.view.family ||
                   gba.view.layout != cf.view.layout || gba.view.combo != cf.view.combo ||
                   gba.view.rot != cf.view.rot || memcmp(gba.view.level, cf.view.lvl, 3) != 0))) {
      if (mismatches < 10u) {
        fprintf(stderr, "FAIL reduced oracle at step %u\n", step);
      }
      ++mismatches;
    }
  }
  printf("gba reduced oracle: 30000 steps, %u shown, %u mismatches\n", shown_steps, mismatches);
  return mismatches || !shown_steps;
}
