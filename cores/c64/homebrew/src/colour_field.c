#include "colour_field.h"

#include <string.h>

#define CF_NONE 0xFFu

cf_state_t cf;
cf_input_t cf_in;
uint8_t cf_keys[8];

static uint8_t cf_latched_scene = 0xFFu;
static uint8_t cf_latched_palette;
static uint8_t cf_previous_beat_keys;

#define SAT_INC(x) do { if ((x) != 0xFFFFu) { ++(x); } } while (0)

static const uint8_t cf_up[4] = {0u, 48u, 104u, 168u};
static const uint8_t cf_down[4] = {0u, 32u, 88u, 152u};

/* C64 (Pepto) indices per the spec table: A.R 10 light red, A.Y 7 yellow,
 * A.B 6 blue, W 1 white, K 0 black, B.R 2 red, B.G 5 green, B.B 6 blue,
 * B.C 3 cyan, B.M 4 purple, B.Y 7 yellow. Indexed by family << 5 |
 * (regions - 2) << 4 | combo << 2 | entry; entries are padded to 4 so the
 * index is all shifts (6502 multiplies are slow). */
static const uint8_t cf_palette[64] = {
  10u, 6u, 0u, 0u, 7u, 6u, 0u, 0u, 10u, 7u, 0u, 0u, 6u, 1u, 0u, 0u,
  10u, 7u, 6u, 0u, 10u, 1u, 6u, 0u, 7u, 0u, 6u, 0u, 10u, 0u, 7u, 0u,
  2u, 3u, 0u, 0u, 5u, 4u, 0u, 0u, 6u, 7u, 0u, 0u, 3u, 4u, 0u, 0u,
  2u, 5u, 6u, 0u, 3u, 4u, 7u, 0u, 2u, 0u, 3u, 0u, 6u, 0u, 7u, 0u,
};

static uint8_t cf_argmax(uint16_t a, uint16_t b, uint16_t c) {
  if (a >= b && a >= c) {
    return 0u;
  }
  return b >= c ? 1u : 2u;
}

/* Macros rather than calls: cc65 function calls cost more than the bodies. */
#define CF_AVG(index) ((uint16_t)(cf.acc[index] >> 3))
#define CF_REGIONS ((cf.layout & 2u) ? 3u : 2u)

static uint8_t cf_count_up(uint8_t v) {
  uint8_t level = 0u;
  while (level < 3u && v >= cf_up[level + 1u]) {
    ++level;
  }
  return level;
}

uint8_t cf_mode(void) {
  uint8_t mode = (uint8_t)(cf.p_eff & 3u);
  return (mode == 3u && !cf.flash) ? 2u : mode;
}

uint8_t cf_regions(void) {
  return CF_REGIONS;
}

/* Spectral band value for a region: 3 regions follow bass/mid/treble,
 * 2 regions follow bass and max(mid, treble). */
static uint8_t cf_region_value(uint8_t region) {
  if (region == 0u) {
    return cf_in.band[0];
  }
  if (CF_REGIONS == 3u) {
    return cf_in.band[region];
  }
  return cf_in.band[1] > cf_in.band[2] ? cf_in.band[1] : cf_in.band[2];
}

static void cf_init_levels(void) {
  uint8_t i;
  for (i = 0u; i < 3u; ++i) {
    cf.lvl[i] = cf_count_up(cf_region_value(i));
  }
  if (CF_REGIONS == 2u) {
    cf.lvl[2] = 0u;
  }
}

static void cf_rotate(void) {
  cf.rot = (uint8_t)((cf.rot + 1u) % CF_REGIONS);
}

static void cf_next_combo(void) {
  cf.combo = (uint8_t)((cf.combo + 1u) & 3u);
}

/* Not called in armed flash/gate, so it always uses the change budget. */
static void cf_next_layout(void) {
  cf.layout = (uint8_t)((cf.layout + 1u) & 3u);
  cf.rot = (uint8_t)(cf.rot % CF_REGIONS);
  cf.since_layout = 0u;
  cf.lg = 0u;
  if (cf_mode() == 1u) {
    cf_init_levels();
  }
  cf.fx |= CF_FX_LAYOUT;
}

static void cf_mode_entry(uint8_t power_on) {
  uint8_t mode = cf_mode();
  cf.act = 0u;
  if (power_on) {
    cf.since_cut = CF_CUT_MIN;
    cf.lg = CF_LEVEL_OPP;
  } else {
    cf.since_cut = 0u;
    cf.lg = 0u;
  }
  if (mode == 0u) {
    cf.combo = cf.dom;
  }
  cf_init_levels();
  cf.pulse = 0u;
  cf.gate_off = 4u;
  cf.hyst = (uint8_t)(CF_AVG(0u) >= 64u);
  cf.dim = cf.hyst;
  if (cf.silent && (mode == 1u || mode == 3u)) {
    cf.lvl[0] = cf.lvl[1] = cf.lvl[2] = 0u;
    cf.hyst = cf.dim = 0u;
  }
}

static void cf_enter(uint8_t power_on) {
  uint8_t i;
  uint8_t p = (uint8_t)(cf_in.palette & 7u);

  cf.acc[0] = (uint16_t)((uint16_t)cf_in.energy << 3);
  for (i = 0u; i < 3u; ++i) {
    cf.acc[i + 1u] = (uint16_t)((uint16_t)cf_in.band[i] << 3);
  }
  for (i = 0u; i < 8u; ++i) {
    cf.prev_spec[i] = cf_in.spectrum[i];
  }
  cf.armed = 1u;
  cf.silent = 0u;
  cf.quiet = 0u;
  cf.dom = cf_argmax(cf_in.band[0], cf_in.band[1], cf_in.band[2]);
  cf.cand = CF_NONE;
  cf.cand_n = 0u;
  cf.layout = 0u;
  cf.rot = 0u;
  cf.combo = 0u;
  cf.flash = (uint8_t)(!cf_in.wire && cf_in.pre_armed && (p & 3u) == 3u);
  if ((p & 3u) == 3u && !cf.flash) {
    --p;
  }
  cf.p_eff = p;
  cf.req_valid = 0u;
  cf.since_pulse = 8u;
  /* The wire stability count starts here; the entry value is step 1. */
  cf.w_val = (uint8_t)(cf_in.palette & 7u);
  cf.w_n = cf_in.scene == CF_SCENE ? 1u : 0u;
  cf.w_ok = cf_in.scene_abs;
  cf_mode_entry(power_on);
  cf.since_layout = 0u;
}

static void cf_mode_rule(uint8_t mode, uint8_t e_ev, uint8_t d_ev) {
  uint8_t x_ev = (uint8_t)((cf_in.events & CF_EV_X) != 0u);
  uint8_t i;
  uint8_t changed;

  if (mode == 0u) {
    if (d_ev && cf.combo != cf.dom) {
      cf.combo = cf.dom;
      cf.lg = 0u;
    }
    if ((x_ev || (d_ev && cf.since_layout >= 600u)) && cf.since_cut >= CF_CUT_MIN) {
      cf_next_layout();
      cf.since_cut = 0u;
    }
  } else if (mode == 1u) {
    if (x_ev && cf.since_cut >= CF_CUT_MIN && cf.lg >= CF_LEVEL_OPP) {
      cf_next_layout();
      cf_next_combo();
      cf.since_cut = 0u;
    } else if (d_ev && cf.since_layout >= 600u && cf.lg >= CF_LEVEL_OPP) {
      cf_next_layout();
    }
    if (cf.lg >= CF_LEVEL_OPP) {
      changed = 0u;
      for (i = 0u; i < CF_REGIONS; ++i) {
        uint8_t v = cf_region_value(i);
        if (cf.lvl[i] < 3u && v >= cf_up[cf.lvl[i] + 1u]) {
          ++cf.lvl[i];
          changed = 1u;
        } else if (cf.lvl[i] > 0u && v < cf_down[cf.lvl[i]]) {
          --cf.lvl[i];
          changed = 1u;
        }
      }
      if (changed) {
        cf.lg = 0u;
      }
    }
  } else if (mode == 2u) {
    if (e_ev && cf.since_cut >= CF_CUT_MIN) {
      cf.since_cut = 0u;
      cf.lg = 0u;
      cf.fx |= CF_FX_CUT;
      if (x_ev) {
        cf_next_layout();
        cf_next_combo();
      } else {
        if (cf.act == 1u) {
          cf_next_combo();
        } else if (cf.act == 3u) {
          cf_next_layout();
        } else {
          cf_rotate();
        }
        cf.act = (uint8_t)((cf.act + 1u) & 3u);
      }
    }
  } else {
    uint16_t avg_e = CF_AVG(0u);
    if (avg_e >= 64u) {
      cf.hyst = 1u;
    } else if (avg_e < 48u) {
      cf.hyst = 0u;
    }
    if (cf.pulse > 0u) {
      if (--cf.pulse == 0u) {
        cf.gate_off = 0u;
        cf.dim = cf.hyst;
      }
    } else if (e_ev && cf.gate_off >= 4u && cf.since_pulse >= 8u) {
      cf.pulse = 4u;
      cf.since_pulse = 0u;
      cf.fx |= CF_FX_PULSE;
      cf_rotate();
      if (x_ev) {
        cf_next_combo();
      }
    }
  }
}

static void cf_snapshot(void) {
  cf_view_t *v = &cf.view;
  uint8_t mode = cf_mode();
  uint8_t i;
  v->mode = mode;
  v->family = (uint8_t)(cf.p_eff >> 2);
  v->layout = cf.layout;
  v->combo = cf.combo;
  v->rot = cf.rot;
  for (i = 0u; i < 3u; ++i) {
    if (i >= CF_REGIONS) {
      v->lvl[i] = 0u;
    } else if (mode == 1u) {
      v->lvl[i] = cf.lvl[i];
    } else if (mode == 3u) {
      v->lvl[i] = cf.pulse > 0u ? 3u : (cf.dim ? 1u : 0u);
    } else {
      v->lvl[i] = 3u;
    }
  }
}

static void cf_wire_gate(void) {
  uint8_t p = (uint8_t)(cf_in.palette & 7u);

  if (cf_in.scene != CF_SCENE) {
    /* Outside the scene-7 window: restart the count, drop pending requests. */
    cf.w_n = 0u;
    cf.req_valid = 0u;
    return;
  }
  if (cf.w_n == 0u || p != cf.w_val) {
    cf.w_val = p;
    cf.w_n = 1u;
    cf.w_ok = cf_in.scene_abs;
  } else {
    SAT_INC(cf.w_n);
    if (!cf_in.scene_abs) {
      cf.w_ok = 0u;
    }
  }
  if (cf.w_n == CF_P_STABLE) {
    cf.req_p = p;
    cf.req_armed = (uint8_t)(cf.w_ok && (p & 3u) == 3u);
    cf.req_valid = 1u;
  }
}

static void cf_apply_request(void) {
  uint8_t mode;
  uint8_t p;

  if (!cf.req_valid) {
    return;
  }
  p = cf.req_p;
  mode = (uint8_t)(p & 3u);
  if (mode == 3u && !cf.req_armed) {
    mode = 2u;
    --p;
  }
  if (mode == cf_mode() && (p >> 2) == (cf.p_eff >> 2)) {
    cf.req_valid = 0u;
    return;
  }
  if (cf.lg < CF_LEVEL_OPP || cf.since_cut < CF_CUT_MIN) {
    return;
  }
  cf.p_eff = p;
  cf.flash = (uint8_t)(mode == 3u);
  cf.req_valid = 0u;
  cf.fx |= CF_FX_APPLY;
  cf_mode_entry(0u);
}

static void cf_step(void) {
  int16_t de;
  int16_t db[3];
  uint16_t flux = 0u;
  uint16_t avg_e;
  uint8_t i;
  uint8_t mode;
  uint8_t hit;
  uint8_t calm;
  uint8_t t_ev = 0u;
  uint8_t d_ev = 0u;
  uint8_t e_ev;
  uint8_t c;
  uint16_t avg_b[3];

  /* 1. deltas, then averages and previous spectrum. */
  de = (int16_t)((int16_t)cf_in.energy - (int16_t)CF_AVG(0u));
  cf.acc[0] = (uint16_t)(cf.acc[0] - (cf.acc[0] >> 3) + cf_in.energy);
  for (i = 0u; i < 3u; ++i) {
    db[i] = (int16_t)((int16_t)cf_in.band[i] - (int16_t)CF_AVG(i + 1u));
    cf.acc[i + 1u] = (uint16_t)(cf.acc[i + 1u] - (cf.acc[i + 1u] >> 3) + cf_in.band[i]);
    avg_b[i] = CF_AVG(i + 1u);
  }
  for (i = 0u; i < 8u; ++i) {
    if (cf_in.spectrum[i] > cf.prev_spec[i]) {
      flux = (uint16_t)(flux + (uint8_t)(cf_in.spectrum[i] - cf.prev_spec[i]));
    }
    cf.prev_spec[i] = cf_in.spectrum[i];
  }
  avg_e = CF_AVG(0u);
  mode = cf_mode();

  /* 2. silence. */
  if (cf.silent) {
    if (cf_in.energy >= 24u) {
      cf.silent = 0u;
      cf.quiet = 0u;
    }
  } else {
    if (avg_e <= 12u) {
      SAT_INC(cf.quiet);
    } else {
      cf.quiet = 0u;
    }
    if (cf.quiet >= 30u) {
      cf.silent = 1u;
      cf.dim = 0u;
      if (cf.pulse > 0u) {
        cf.pulse = 0u;
        cf.gate_off = 0u;
      }
    }
  }
  if (mode == 1u && cf.silent && (cf.lvl[0] | cf.lvl[1] | cf.lvl[2]) && cf.lg >= CF_LEVEL_OPP) {
    cf.lvl[0] = cf.lvl[1] = cf.lvl[2] = 0u;
    cf.lg = 0u;
  }

  /* 3. transients. */
  hit = (uint8_t)(cf_in.energy >= 24u &&
                  (de >= 40 || db[0] >= 56 || db[1] >= 56 || db[2] >= 56 || flux >= 200u));
  calm = (uint8_t)(de < 16 && db[0] < 24 && db[1] < 24 && db[2] < 24 && flux < 64u);
  if (cf.armed && hit) {
    t_ev = 1u;
    cf.armed = 0u;
    cf.fx |= CF_FX_T;
  } else if (calm) {
    cf.armed = 1u;
  }

  /* 4. events. */
  e_ev = (uint8_t)(!cf.silent && (t_ev || (cf_in.events & (CF_EV_B | CF_EV_X | CF_EV_N)) != 0u));

  /* 5. dominance. */
  c = cf_argmax(avg_b[0], avg_b[1], avg_b[2]);
  if (!cf.silent && c != cf.dom && avg_b[c] >= (uint16_t)(avg_b[cf.dom] + 32u)) {
    cf.cand_n = cf.cand == c ? (uint16_t)(cf.cand_n + 1u) : 1u;
    cf.cand = c;
  } else {
    cf.cand = CF_NONE;
    cf.cand_n = 0u;
  }
  if (cf.cand_n >= 30u) {
    cf.dom = c;
    cf.cand_n = 0u;
    d_ev = 1u;
  }

  /* 6. mode rule. */
  if (!cf.silent) {
    cf_mode_rule(mode, e_ev, d_ev);
  }

  /* 7. counters, palette gate, at most one request under the budget. */
  SAT_INC(cf.since_cut);
  SAT_INC(cf.since_layout);
  SAT_INC(cf.gate_off);
  SAT_INC(cf.since_pulse);
  SAT_INC(cf.lg);
  if (cf_in.wire) {
    cf_wire_gate();
  }
  cf_snapshot();
  cf_apply_request();
}

void cf_init(void) {
  uint8_t *bytes = (uint8_t *)&cf;
  uint16_t i;
  for (i = 0u; i < sizeof(cf); ++i) {
    bytes[i] = 0u;
  }
  cf.since_sw = 0xFFFFu;
  cf_latched_scene = 0xFFu;
  cf_latched_palette = 0u;
  cf_previous_beat_keys = 0u;
}

uint8_t cf_tick(void) {
  uint8_t shown;

  cf.fx = 0u;
#ifndef CF_UNMEASURED_FLASH
  /* Flash/gate needs independent photosensitivity measurement (#96) before a
   * shipped build may flash. This is the single point that downgrades every
   * armed request: with no absolute report and no pre-arm, palettes 3 and 7
   * run as unarmed 2 and 6 (hard cuts) and cf.flash is never latched. Build
   * with CF_UNMEASURED_FLASH (make UNMEASURED_FLASH=1) to measure it. */
  cf_in.scene_abs = 0u;
  cf_in.pre_armed = 0u;
#endif
  if (!cf.started) {
    /* Power-on entry is exempt from the debounce. */
    cf.started = 1u;
    cf.deb_val = cf_in.scene;
    cf.deb_n = 1u;
    if (cf_in.scene == CF_SCENE) {
      cf.shown = 1u;
      cf.fx |= CF_FX_ENTER;
      cf_enter(1u);
      cf_snapshot();
    }
    return cf.shown;
  }
  if (cf_in.scene == cf.deb_val) {
    SAT_INC(cf.deb_n);
  } else {
    cf.deb_val = cf_in.scene;
    cf.deb_n = 1u;
  }
  shown = cf.shown;
  if (shown) {
    if (cf.entering) {
      cf.entering = 0u;
      cf.fx |= CF_FX_ENTER;
      cf_enter(0u);
      cf_snapshot();
    } else {
      cf_step();
    }
  }
  SAT_INC(cf.since_sw);
  if ((uint8_t)(cf.deb_val == CF_SCENE) != shown && cf.deb_n >= CF_P_STABLE &&
      cf.since_sw >= CF_CUT_MIN) {
    cf.since_sw = 0u;
    cf.shown = (uint8_t)!shown;
    cf.entering = cf.shown;
    if (!cf.shown) {
      cf.flash = 0u;
      cf.fx |= CF_FX_EXIT;
    }
  }
  return shown;
}

uint8_t cf_level(uint8_t region) {
  return region < 3u ? cf.view.lvl[region] : 0u;
}

uint8_t cf_colour(uint8_t region) {
  uint8_t three = (uint8_t)(cf.view.layout & 2u);
  uint8_t entry;
  /* C64: levels 3 and 2 show the base colour, levels 1 and 0 black. */
  if (region > 2u || cf.view.lvl[region] < 2u) {
    return 0u;
  }
  /* (region + rot) mod n with region, rot < n. */
  entry = (uint8_t)(region + cf.view.rot);
  if (entry >= (three ? 3u : 2u)) {
    entry -= three ? 3u : 2u;
  }
  return cf_palette[(uint8_t)((cf.view.family << 5) | (three << 3) | (cf.view.combo << 2) | entry)];
}

uint8_t cf_cell_region(uint8_t layout, uint8_t column, uint8_t row) {
  /* b_i = g * ((i * (S / g)) / n) in 8-pixel cells; rows count from the bottom. */
  uint8_t n = (layout & 2u) ? 3u : 2u;
  uint8_t cells = (layout & 1u) ? 25u : 40u;
  uint8_t pos = (layout & 1u) ? (uint8_t)(24u - row) : column;
  uint8_t region = 0u;
  uint8_t i;
  for (i = 1u; i < n; ++i) {
    if (pos >= (uint8_t)((i * cells) / n)) {
      region = i;
    }
  }
  return region;
}

/* Index 0..7 of the only set bit, or CF_NONE when none or several are set
 * (several can be matrix ghosts or joystick aliases). */
static uint8_t cf_single_key(uint8_t held) {
  uint8_t index = 0u;

  if (held == 0u || (held & (uint8_t)(held - 1u)) != 0u) {
    return CF_NONE;
  }
  while (!(held & 1u)) {
    held >>= 1;
    ++index;
  }
  return index;
}

/* Keyboard HID is a reduced, digital-only transport with no energy key:
 * energy and bands are HID_E_LO, the spectrum is 0, beat is Space and bar is
 * Return. F (fill/drop) is ignored, so there are no X events.
 * usb-hid-transport-v0 keys as (CIA1 port A column, port B row): scene keys
 * 1..8 are (7,0) (7,3) (1,0) (1,3) (2,0) (2,3) (3,0) (3,3); palette keys
 * Q W E R T Y U I are (7,6) (1,1) (1,6) (2,1) (2,6) (3,1) (3,6) (4,1); Space
 * is (7,4) and Return (0,1). */
void cf_read_keys(uint8_t fallback_scene) {
  uint8_t c1 = cf_keys[1];
  uint8_t c2 = cf_keys[2];
  uint8_t c3 = cf_keys[3];
  uint8_t c7 = cf_keys[7];
  uint8_t scene;
  uint8_t palette;
  uint8_t beat_keys;

  /* Bit i of each byte is key i of the group; the low and high nibbles of
   * the column pairs are picked with constant masks. */
  scene = (uint8_t)((c7 & 0x01u) | ((c7 & 0x08u) >> 2) | ((c1 & 0x01u) << 2) | (c1 & 0x08u) |
                    ((c2 & 0x01u) << 4) | ((c2 & 0x08u) << 2) | ((c3 & 0x01u) << 6) | ((c3 & 0x08u) << 4));
  palette = (uint8_t)(((c7 & 0x40u) >> 6) | (c1 & 0x02u) | ((c1 & 0x40u) >> 4) | ((c2 & 0x02u) << 2) |
                      ((c2 & 0x40u) >> 2) | ((c3 & 0x02u) << 4) | (c3 & 0x40u) | ((cf_keys[4] & 0x02u) << 6));
  beat_keys = (uint8_t)(((c7 >> 4) & 1u) | (cf_keys[0] & 2u));
  scene = cf_single_key(scene);
  palette = cf_single_key(palette);
  if (scene != CF_NONE) {
    cf_latched_scene = scene;
  }
  if (palette != CF_NONE) {
    cf_latched_palette = palette;
  }
  memset(&cf_in, 0, sizeof(cf_in));
  /* Until a scene key arrives the procedural scene is used; it is never 7. */
  cf_in.scene = cf_latched_scene != CF_NONE ? cf_latched_scene : fallback_scene;
  /* Flash can arm only from reports holding exactly one scene key and one
   * palette key together, as usb_hid.py sends them. */
  cf_in.scene_abs = (uint8_t)(scene != CF_NONE && palette != CF_NONE);
  cf_in.palette = cf_latched_palette;
  cf_in.wire = 1u;
  cf_in.energy = CF_HID_E_LO;
  memset(cf_in.band, CF_HID_E_LO, sizeof(cf_in.band));
  if (beat_keys & (uint8_t)~cf_previous_beat_keys) {
    cf_in.events = CF_EV_B;
  }
  cf_previous_beat_keys = beat_keys;
}

uint8_t cf_irq_on_time(uint8_t d012, uint8_t d011, uint16_t irq_line, uint16_t frame_lines) {
  uint16_t line = (uint16_t)(d012 | ((uint16_t)(d011 & 0x80u) << 1));
  uint16_t late;

  if (line >= frame_lines) {
    return 0u;
  }
  late = line >= irq_line ? (uint16_t)(line - irq_line) : (uint16_t)(line + frame_lines - irq_line);
  return (uint8_t)(late < CF_IRQ_WINDOW);
}
