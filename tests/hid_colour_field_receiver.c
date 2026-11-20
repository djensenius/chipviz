/* Host-test adapter for a gamepad-only GBA ring feeding a colour-field core.
 * The default uses the C64 full reference detector. With GBA_RECEIVER it
 * uses the GBA data-role receiver and its actual input edge handler instead. */
#include "colour_field.h"
#ifdef GBA_RECEIVER
#include "input.h"
#else
#include <stdint.h>
#include <string.h>
#endif

static uint8_t scene, palette, shown, beat_edge;
static uint16_t previous_keys;
#ifdef GBA_RECEIVER
static gba_cf_t receiver;
#else
static uint8_t fixture_options;
#endif

void receiver_init(uint8_t initial_scene, uint8_t initial_palette, uint8_t options) {
  scene = initial_scene;
  palette = initial_palette;
  previous_keys = 0;
  shown = 0;
  beat_edge = 0;
#ifdef GBA_RECEIVER
  (void)options;
  gba_cf_init(&receiver);
#else
  fixture_options = options;
  cf_init();
#endif
}

void receiver_step(uint16_t keys) {
  uint16_t pressed = (uint16_t)(keys & ~previous_keys);
  beat_edge = (uint8_t)((pressed & 3u) != 0u);
#ifdef GBA_RECEIVER
  update_input(keys, &previous_keys, &scene, &palette);
  shown = gba_cf_step(&receiver, scene, palette, keys);
#else
  previous_keys = keys;
  if ((keys & 0x00c0u) != 0x00c0u) {
    if (pressed & 0x0040u) {
      scene = scene == 7u ? 0u : scene == 3u ? 7u : (uint8_t)(scene + 1u);
    } else if (pressed & 0x0080u) {
      scene = scene == 0u ? 7u : scene == 7u ? 3u : (uint8_t)(scene - 1u);
    }
  }
  if ((keys & 0x0030u) != 0x0030u) {
    if (pressed & 0x0010u) {
      palette = (uint8_t)((palette + 1u) & 7u);
    } else if (pressed & 0x0020u) {
      palette = (uint8_t)((palette + 7u) & 7u);
    }
  }
  memset(&cf_in, 0, sizeof(cf_in));
  cf_in.scene = scene;
  /* The absolute option is only a positive-control fixture, not a gamepad. */
  cf_in.scene_abs = (uint8_t)((fixture_options & 2u) != 0u);
  cf_in.palette = palette;
  cf_in.wire = (uint8_t)((fixture_options & 1u) == 0u);
  cf_in.pre_armed = (uint8_t)((fixture_options & 1u) != 0u);
  cf_in.energy = (keys & 0x0200u) ? 200u : 64u;
  cf_in.band[0] = cf_in.band[1] = cf_in.band[2] = cf_in.energy;
  cf_in.events = beat_edge ? CF_EV_B : 0;
  shown = cf_tick();
#endif
}

/* shown, mode, family, layout, combo, rotation, levels, ring values, beat,
 * flash latch, transient, silence, pulse and post-step effective palette.
 * On the GBA build flash is
 * disabled; reference-only detector diagnostics are unavailable. */
uint8_t receiver_value(unsigned index) {
#ifdef GBA_RECEIVER
  const gba_cf_view_t *view = &receiver.view;
#else
  const cf_view_t *view = &cf.view;
#endif
  switch (index) {
    case 0: return shown;
    case 1: return view->mode;
    case 2: return view->family;
    case 3: return view->layout;
    case 4: return view->combo;
    case 5: return view->rot;
#ifdef GBA_RECEIVER
    case 6: return view->level[0];
    case 7: return view->level[1];
#else
    case 6: return view->lvl[0];
    case 7: return view->lvl[1];
#endif
    case 8: return scene;
    case 9: return palette;
    case 10: return beat_edge;
#ifdef GBA_RECEIVER
    case 11: return 0;
    case 12: return 0;
    case 13: return 0; /* Digital HID energy is never silent. */
    case 14: return 0;
    case 15: return receiver.palette;
#else
    case 11: return cf.flash;
    case 12: return (uint8_t)((cf.fx & CF_FX_T) != 0u);
    case 13: return cf.silent;
    case 14: return (uint8_t)((cf.fx & CF_FX_PULSE) != 0u);
    case 15: return cf.p_eff;
#endif
    default: return 0;
  }
}
