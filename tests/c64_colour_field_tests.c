/* Host checks for the C64 colour-field core against the normative fixtures in
 * shared/specs/colour-field-v0.md. Operator-only (F12, F16), resampled (F15,
 * F18) and host-side (F23) fixtures do not apply to the C64 data-role build. */
#include "colour_field.h"

#include <stdio.h>
#include <string.h>

#define STEPS 600u

typedef struct {
  uint8_t e, band[3], spec[8], flags, scene, palette, abs_off;
  uint8_t notes[2][2];
} frame_t;

typedef struct {
  uint8_t shown, mode, family, layout, combo, rot, lvl[3], fx;
} trace_t;

typedef void (*gen_fn)(uint16_t step, frame_t *frame);

static unsigned checks;
static unsigned failures;
static uint16_t lcg;
static trace_t tr[STEPS];

static void require(int condition, const char *fixture, const char *message) {
  checks++;
  if (!condition) {
    fprintf(stderr, "FAIL %s: %s\n", fixture, message);
    failures++;
  }
}

static uint8_t lcg_next(void) {
  lcg = (uint16_t)(lcg * 25173u + 13849u);
  return (uint8_t)(lcg >> 8);
}

static void fill(frame_t *f, uint8_t v) {
  f->e = v;
  memset(f->band, v, sizeof(f->band));
  memset(f->spec, v, sizeof(f->spec));
}

static uint8_t note_onset(const frame_t *now, const frame_t *prev) {
  uint8_t i;
  uint8_t j;
  for (i = 0; i < 2u; i++) {
    uint8_t seen = 0;
    if (now->notes[i][1] < 64u) {
      continue;
    }
    for (j = 0; j < 2u; j++) {
      seen |= (uint8_t)(prev->notes[j][0] == now->notes[i][0] && prev->notes[j][1] == now->notes[i][1]);
    }
    if (!seen) {
      return 1;
    }
  }
  return 0;
}

/* wire = 0: local p with operator pre-arm (fixture setup); wire = 1: palette
 * from the frame through the P_STABLE gate. */
static void run(gen_fn gen, uint8_t p, uint8_t wire) {
  frame_t prev;
  frame_t f;
  uint16_t step;
  uint8_t i;

  memset(&prev, 0, sizeof(prev));
  cf_init();
  lcg = 1;
  for (step = 0; step < STEPS; step++) {
    memset(&f, 0, sizeof(f));
    f.scene = 7;
    f.palette = p;
    gen(step, &f);
    memset(&cf_in, 0, sizeof(cf_in));
    cf_in.scene = f.scene;
    cf_in.scene_abs = (uint8_t)!f.abs_off;
    cf_in.palette = f.palette;
    cf_in.wire = wire;
    cf_in.pre_armed = (uint8_t)!wire;
    cf_in.energy = f.e;
    memcpy(cf_in.band, f.band, sizeof(cf_in.band));
    memcpy(cf_in.spectrum, f.spec, sizeof(cf_in.spectrum));
    if (((f.flags & 1u) && !(prev.flags & 1u)) || ((f.flags & 2u) && !(prev.flags & 2u))) {
      cf_in.events |= CF_EV_B;
    }
    if ((f.flags & 4u) && !(prev.flags & 4u)) {
      cf_in.events |= CF_EV_X;
    }
    if (note_onset(&f, &prev)) {
      cf_in.events |= CF_EV_N;
    }
    prev = f;
    tr[step].shown = cf_tick();
    tr[step].mode = cf.view.mode;
    tr[step].family = cf.view.family;
    tr[step].layout = cf.view.layout;
    tr[step].combo = cf.view.combo;
    tr[step].rot = cf.view.rot;
    for (i = 0; i < 3u; i++) {
      tr[step].lvl[i] = cf_level(i);
    }
    tr[step].fx = cf.fx;
  }
}

static int same_view(uint16_t a, uint16_t b) {
  return tr[a].mode == tr[b].mode && tr[a].family == tr[b].family && tr[a].layout == tr[b].layout &&
         tr[a].combo == tr[b].combo && tr[a].rot == tr[b].rot && memcmp(tr[a].lvl, tr[b].lvl, 3) == 0;
}

static int constant(void) {
  uint16_t s;
  for (s = 1; s < STEPS; s++) {
    if (!same_view(s, 0)) {
      return 0;
    }
  }
  return 1;
}

/* fx steps must equal the zero-terminated list exactly. */
static int fx_steps(uint8_t flag, const uint16_t *list) {
  uint16_t s;
  uint16_t n = 0;
  for (s = 0; s < STEPS; s++) {
    if (tr[s].fx & flag) {
      if (list[n] != s) {
        return 0;
      }
      n++;
    }
  }
  return list[n] == 0;
}

static uint16_t fx_count(uint8_t flag) {
  uint16_t s;
  uint16_t n = 0;
  for (s = 0; s < STEPS; s++) {
    n = (uint16_t)(n + ((tr[s].fx & flag) != 0));
  }
  return n;
}

/* Steps where the displayed levels change, as a zero-terminated list. */
static int level_changes(uint16_t until, const uint16_t *list) {
  uint16_t s;
  uint16_t n = 0;
  for (s = 1; s < until; s++) {
    if (memcmp(tr[s].lvl, tr[s - 1].lvl, 3) != 0) {
      if (list[n] != s) {
        return 0;
      }
      n++;
    }
  }
  return list[n] == 0;
}

static int levels(uint16_t step, uint8_t a, uint8_t b) {
  return tr[step].lvl[0] == a && tr[step].lvl[1] == b;
}

static int max_changes_per_60(void) {
  uint16_t s;
  uint16_t w;
  int best = 0;
  for (s = 0; s + 60u <= STEPS; s++) {
    int n = 0;
    for (w = (uint16_t)(s + 1u); w < s + 60u; w++) {
      n += !same_view(w, (uint16_t)(w - 1u));
    }
    best = n > best ? n : best;
  }
  return best;
}

static void g_f0(uint16_t step, frame_t *f) {
  (void)step;
  (void)f;
}

static void g_f1(uint16_t step, frame_t *f) {
  static const uint8_t spec[8] = {40, 40, 120, 200, 200, 120, 40, 40};
  f->e = 160;
  f->band[0] = 40;
  f->band[1] = 200;
  f->band[2] = 40;
  memcpy(f->spec, spec, 8);
  f->flags = (uint8_t)(step >= 300u && step <= 305u ? 4u : 0u);
}

static void g_f2(uint16_t step, frame_t *f) {
  uint8_t i;
  (void)step;
  f->e = (uint8_t)(150 + (int)(lcg_next() % 25u) - 12);
  for (i = 0; i < 3u; i++) {
    f->band[i] = (uint8_t)(150 + (int)(lcg_next() % 25u) - 12);
  }
  for (i = 0; i < 8u; i++) {
    f->spec[i] = (uint8_t)(150 + (int)(lcg_next() % 25u) - 12);
  }
}

static void g_f3(uint16_t step, frame_t *f) {
  static const uint16_t at[] = {37, 101, 113, 250, 400, 523};
  uint8_t i;
  fill(f, 40);
  for (i = 0; i < 6u; i++) {
    if (step == at[i] || step == at[i] + 1u) {
      fill(f, 220);
    }
  }
}

static void g_f5(uint16_t step, frame_t *f) {
  uint16_t phase = (uint16_t)(step % 30u);
  fill(f, 60);
  if (step >= 30u && phase < 4u && step / 30u <= 19u) {
    f->e = 180;
    f->band[0] = 200;
    f->spec[0] = f->spec[1] = 200;
  }
  if (step >= 30u && phase == 0u) {
    f->flags |= 1u;
  }
  if (step >= 120u && step % 120u == 0u) {
    f->flags |= 2u;
  }
}

static void g_f6(uint16_t step, frame_t *f) {
  uint8_t i;
  if (step >= 60u && step < 420u) {
    f->e = (uint8_t)(lcg_next() % 13u);
    for (i = 0; i < 3u; i++) {
      f->band[i] = (uint8_t)(lcg_next() % 13u);
    }
    for (i = 0; i < 8u; i++) {
      f->spec[i] = (uint8_t)(lcg_next() % 13u);
    }
  } else {
    fill(f, step == 480u || step == 481u ? 220u : 120u);
  }
}

static void g_f7(uint16_t step, frame_t *f) {
  uint8_t i;
  fill(f, 60);
  f->e = 150;
  f->band[step < 300u ? 0 : 2] = 200;
  for (i = 0; i < 4u; i++) {
    f->spec[step < 300u ? i : i + 4u] = 200;
  }
}

static void g_f10(uint16_t step, frame_t *f) {
  g_f1(step, f);
  f->flags = 0;
  f->palette = (uint8_t)(step >= 200u ? 3u : (step >= 100u && step < 110u) ? 2u : 0u);
}

static void g_f13(uint16_t step, frame_t *f) {
  f->e = (uint8_t)(step > 0u && step % 31u == 0u ? 30u : 0u);
  memset(f->band, 250, 3);
}

static void g_f14(uint16_t step, frame_t *f) {
  f->e = (uint8_t)(step % 5u == 0u ? 255u : 0u);
}

/* F19: digital-only HID, as on the C64 keyboard path (energy HID_E_LO). */
static void g_f19(uint16_t step, frame_t *f) {
  f->e = (uint8_t)(step >= 320u && step < 380u ? CF_HID_E_HI : CF_HID_E_LO);
  memset(f->band, f->e, 3);
  f->flags = (uint8_t)(step >= 50u && (step - 50u) % 60u == 0u ? 1u : 0u);
}

static void g_f20(uint16_t step, frame_t *f) {
  f->e = 100;
  memset(f->band, step >= 200u && step < 400u ? 10 : 250, 3);
}

static void g_f21(uint16_t step, frame_t *f) {
  fill(f, 100);
  if (step < 100u || (step >= 300u && step < 320u) || step >= 450u) {
    f->scene = 6;
  } else if (step < 200u) {
    f->scene = (uint8_t)(step & 1u ? 6u : 7u);
  }
}

static void g_f22(uint16_t step, frame_t *f) {
  fill(f, 100);
  f->palette = (uint8_t)(step >= 60u ? 3u : 0u);
  if (step >= 60u && step < 300u) {
    f->scene = (uint8_t)(((step - 60u) / 20u) & 1u ? 6u : 5u);
  }
}

/* F24 keyboard variant: absolute palette keys Q, W, E, then R for 30-129,
 * then E. abs_off models a report without both keys held (never arms). */
static uint8_t f24_abs_off;
static void g_f24(uint16_t step, frame_t *f) {
  g_f19(0, f);
  f->flags = 0;
  f->palette = (uint8_t)(step < 10u ? 0u : step < 20u ? 1u : step < 30u ? 2u : step < 130u ? 3u : 2u);
  f->abs_off = f24_abs_off;
}

static void fixture_f0(void) {
  uint8_t p;
  for (p = 0; p < 8u; p++) {
    uint8_t lit = (uint8_t)((p & 1u) == 0u);
    run(g_f0, p, 0);
    require(constant(), "F0", "constant trace");
    require(tr[0].layout == 0 && tr[0].combo == 0, "F0", "V2 combo 0");
    require(levels(0, lit ? 3 : 0, lit ? 3 : 0), "F0", "held/cuts level 3, spectral/flash black");
    require(fx_count(CF_FX_CUT | CF_FX_PULSE) == 0, "F0", "no cuts or pulses");
  }
}

static void fixture_f1(void) {
  static const uint16_t at300[] = {300, 0};
  uint8_t f;
  for (f = 0; f < 2u; f++) {
    uint8_t base = (uint8_t)(f * 4u);
    run(g_f1, base, 0);
    require(tr[0].combo == 1 && tr[299].layout == 0 && tr[300].layout == 1 && tr[599].layout == 1,
            "F1", "held combo 1, H2 at 300");
    require(fx_steps(CF_FX_LAYOUT, at300), "F1", "held layout change at 300 only");
    run(g_f1, (uint8_t)(base + 1u), 0);
    require(levels(0, 0, 3) && levels(599, 0, 3), "F1", "spectral 0/3");
    require(tr[299].layout == 0 && tr[300].layout == 1 && tr[300].combo == 1, "F1", "spectral H2 combo 1");
    run(g_f1, (uint8_t)(base + 2u), 0);
    require(fx_steps(CF_FX_CUT, at300) && tr[300].layout == 1 && tr[300].combo == 1, "F1", "cut to H2 combo 1");
    run(g_f1, (uint8_t)(base + 3u), 0);
    require(fx_steps(CF_FX_PULSE, at300) && tr[300].combo == 1 && tr[300].rot == 1 && tr[300].lvl[0] == 3,
            "F1", "flash pulse combo 1 rot 1");
  }
}

static void fixture_f2(void) {
  run(g_f2, 0, 0);
  require(constant(), "F2", "held constant");
  run(g_f2, 1, 0);
  require(constant() && levels(0, 2, 2), "F2", "spectral 2/2");
  run(g_f2, 2, 0);
  require(fx_count(CF_FX_CUT | CF_FX_T) == 0, "F2", "no events");
  run(g_f2, 3, 0);
  require(constant() && levels(0, 1, 1) && fx_count(CF_FX_PULSE) == 0, "F2", "flash rests dim");
}

static void fixture_f3(void) {
  static const uint16_t cuts[] = {37, 101, 250, 400, 523, 0};
  static const uint16_t pulses[] = {37, 101, 113, 250, 400, 523, 0};
  run(g_f3, 0, 0);
  require(constant(), "F3", "held never changes");
  run(g_f3, 2, 0);
  require(fx_steps(CF_FX_CUT, cuts), "F3", "cuts at 37 101 250 400 523");
  run(g_f3, 3, 0);
  require(fx_steps(CF_FX_PULSE, pulses), "F3", "pulses at all six");
}

static void fixture_f5(void) {
  static const uint16_t layouts[] = {120, 240, 360, 480, 0};
  run(g_f5, 0, 0);
  require(constant(), "F5", "held never changes");
  run(g_f5, 2, 0);
  require(fx_count(CF_FX_CUT) == 19 && fx_steps(CF_FX_LAYOUT, layouts), "F5", "19 cuts, layouts every 120");
  require(tr[120].layout == 1 && tr[240].layout == 2 && tr[360].layout == 3 && tr[480].layout == 0, "F5",
          "V2 H2 V3 H3 V2");
  run(g_f5, 3, 0);
  require(fx_count(CF_FX_PULSE) == 19, "F5", "19 pulses");
}

static void fixture_f6(void) {
  static const uint16_t changes[] = {60, 70, 420, 430, 480, 490, 0};
  static const uint16_t hits[] = {420, 480, 0};
  run(g_f6, 1, 0);
  require(level_changes(STEPS, changes) && levels(59, 2, 2) && levels(70, 0, 0) && levels(480, 3, 3) &&
              levels(490, 2, 2), "F6", "spectral steps");
  run(g_f6, 2, 0);
  require(fx_steps(CF_FX_CUT, hits), "F6", "cuts at 420 and 480");
  run(g_f6, 3, 0);
  require(fx_steps(CF_FX_PULSE, hits), "F6", "pulses at 420 and 480");
}

static void fixture_f7(void) {
  static const uint16_t at300[] = {300, 0};
  uint16_t s;
  uint16_t changed = 0;
  run(g_f7, 0, 0);
  for (s = 1; s < STEPS; s++) {
    if (tr[s].combo != tr[s - 1u].combo) {
      require(s >= 330u && s <= 345u && tr[s].combo == 2, "F7", "held 0->2 between 330 and 345");
      changed++;
    }
  }
  require(changed == 1 && tr[0].combo == 0 && fx_count(CF_FX_LAYOUT) == 0, "F7", "held changes once");
  run(g_f7, 1, 0);
  require(levels(299, 3, 1) && levels(300, 2, 2) && levels(309, 2, 2) && levels(310, 1, 3), "F7", "spectral");
  run(g_f7, 2, 0);
  require(fx_steps(CF_FX_CUT, at300), "F7", "cut at 300");
  run(g_f7, 3, 0);
  require(fx_steps(CF_FX_PULSE, at300), "F7", "pulse at 300");
}

static void fixture_f10(void) {
  run(g_f10, 0, 1);
  require(tr[229].mode == 0 && tr[230].mode == 3 && tr[599].mode == 3, "F10", "mode 3 from 230");
  require(levels(230, 1, 1), "F10", "resting dim");
  require(fx_count(CF_FX_APPLY) == 1, "F10", "steps 100-109 have no effect");
}

static void fixture_f13(void) {
  static const uint16_t changes[] = {30, 40, 50, 60, 92, 102, 112, 122, 0};
  uint8_t f;
  for (f = 1; f < 8u; f += 4u) {
    run(g_f13, f, 0);
    require(level_changes(130, changes), "F13", "spectral black/up pattern");
    require(max_changes_per_60() <= 4, "F13", "at most 4 changes in 60 steps");
  }
  run(g_f13, 0, 0);
  require(constant(), "F13", "held unchanged");
}

static void fixture_f14(void) {
  uint16_t s;
  int ok = 1;
  run(g_f14, 3, 0);
  for (s = 0; s < STEPS; s++) {
    int pulse = (tr[s].fx & CF_FX_PULSE) != 0;
    ok &= pulse == (s % 10u == 5u);
  }
  require(ok && fx_count(CF_FX_PULSE) == 60, "F14", "pulses every 10 steps");
  for (s = 0; s <= 28u; s++) {
    ok &= levels(s, 1, 1) || levels(s, 3, 3);
  }
  require(ok, "F14", "dim rest through 28");
  for (s = 30; s < STEPS; s++) {
    uint8_t want = (uint8_t)((s % 10u) >= 5u && (s % 10u) <= 8u ? 3u : 0u);
    ok &= levels(s, want, want);
  }
  require(ok, "F14", "0,0,0,0,0,3,3,3,3,0 from step 30");
  run(g_f14, 2, 0);
  require(fx_count(CF_FX_CUT) == 30, "F14", "30 cuts");
  run(g_f14, 1, 0);
  require(constant(), "F14", "spectral unchanged");
}

static void fixture_f19(void) {
  static const uint16_t cuts[] = {50, 110, 170, 230, 290, 320, 350, 410, 470, 530, 590, 0};
  static const uint16_t t320[] = {320, 0};
  static const uint16_t spec[] = {320, 330, 380, 390, 0};
  uint16_t s;
  int dim = 1;
  run(g_f19, 0, 0);
  require(constant() && fx_steps(CF_FX_T, t320), "F19", "held constant, T at 320 only");
  run(g_f19, 1, 0);
  require(level_changes(STEPS, spec) && levels(0, 1, 1) && levels(320, 2, 2) && levels(330, 3, 3) &&
              levels(380, 2, 2) && levels(390, 1, 1), "F19", "spectral 1/1 2/2 3/3");
  run(g_f19, 2, 0);
  require(fx_steps(CF_FX_CUT, cuts), "F19", "11 cuts");
  run(g_f19, 3, 0);
  require(fx_steps(CF_FX_PULSE, cuts), "F19", "11 pulses");
  for (s = 0; s < STEPS; s++) {
    dim &= tr[s].lvl[0] != 0;
  }
  require(dim, "F19", "flash rest dim, never 0");
}

static void fixture_f20(void) {
  static const uint16_t spec[] = {200, 210, 220, 400, 410, 420, 0};
  static const uint16_t at400[] = {400, 0};
  run(g_f20, 1, 0);
  require(level_changes(STEPS, spec) && levels(0, 3, 3) && levels(220, 0, 0) && levels(420, 3, 3), "F20",
          "spectral saturation");
  run(g_f20, 2, 0);
  require(fx_steps(CF_FX_CUT, at400), "F20", "one cut at 400");
  run(g_f20, 3, 0);
  require(fx_steps(CF_FX_PULSE, at400) && levels(0, 1, 1) && levels(599, 1, 1), "F20", "one pulse, dim rest");
}

static void fixture_f21(void) {
  static const uint16_t enter[] = {230, 0};
  static const uint16_t leave[] = {479, 0};
  uint16_t s;
  int ok = 1;
  run(g_f21, 0, 1);
  for (s = 0; s < STEPS; s++) {
    ok &= tr[s].shown == (s >= 230u && s <= 479u);
  }
  require(ok, "F21", "scene 7 shown for 230-479 only");
  require(fx_steps(CF_FX_ENTER, enter) && fx_steps(CF_FX_EXIT, leave), "F21", "one entry and one exit");
  require(same_view(230, 479) && levels(300, 3, 3) && tr[300].layout == 0 && tr[300].combo == 0, "F21",
          "held V2 combo 0 level 3");
}

static void fixture_f22(void) {
  uint16_t s;
  int ok = 1;
  run(g_f22, 0, 1);
  for (s = 0; s < STEPS; s++) {
    ok &= tr[s].shown;
  }
  require(ok, "F22", "scene 7 shown throughout");
  require(tr[329].mode == 0 && tr[330].mode == 3 && levels(330, 1, 1), "F22", "arms at 329, mode 3 from 330");
}

static void fixture_f24_keys(void) {
  f24_abs_off = 0;
  run(g_f24, 0, 1);
  require(tr[59].mode == 0 && tr[60].mode == 3 && tr[159].mode == 3 && tr[160].mode == 2, "F24",
          "absolute R arms at 59, mode 3 from 60 to 159");
  f24_abs_off = 1;
  run(g_f24, 0, 1);
  require(tr[59].mode == 0 && tr[60].mode == 2 && tr[599].mode == 2 && fx_count(CF_FX_APPLY) == 1, "F24",
          "non-absolute 3 runs as 2 and never arms");
}

static void geometry(void) {
  uint8_t c;
  uint8_t r;
  int ok = 1;
  for (c = 0; c < 40u; c++) {
    ok &= cf_cell_region(0, c, 0) == (c >= 20u);
    ok &= cf_cell_region(2, c, 0) == (c >= 26u ? 2 : c >= 13u ? 1 : 0);
  }
  for (r = 0; r < 25u; r++) {
    ok &= cf_cell_region(1, 0, r) == (r <= 12u);
    ok &= cf_cell_region(3, 0, r) == (r <= 8u ? 2 : r <= 16u ? 1 : 0);
  }
  require(ok, "geometry", "boundaries at 160 / 104,208 px and 96 / 64,128 px from the bottom");
}

static void colours(void) {
  cf_init();
  /* As cf_snapshot leaves it: the unused third region of V2 is level 0. */
  cf.view.lvl[0] = cf.view.lvl[1] = 3u;
  /* A V2 combo 0 is A.R / A.B: light red 10, blue 6; rot swaps them. */
  require(cf_colour(0) == 10u && cf_colour(1) == 6u && cf_colour(2) == 0u, "colours", "A V2 c0");
  cf.view.rot = 1u;
  require(cf_colour(0) == 6u && cf_colour(1) == 10u, "colours", "A V2 c0 rot 1");
  /* B V3 combo 1 is B.C / B.M / B.Y: cyan 3, purple 4, yellow 7; rot 2. */
  cf.view.family = 1u;
  cf.view.layout = 2u;
  cf.view.combo = 1u;
  cf.view.rot = 2u;
  cf.view.lvl[2] = 3u;
  require(cf_colour(0) == 7u && cf_colour(1) == 3u && cf_colour(2) == 4u, "colours", "B V3 c1 rot 2");
  /* Levels 2-3 show the base colour, 0-1 black. */
  cf.view.lvl[0] = 2u;
  cf.view.lvl[1] = 1u;
  cf.view.lvl[2] = 0u;
  require(cf_colour(0) == 7u && cf_colour(1) == 0u && cf_colour(2) == 0u, "colours", "level threshold");
}

int main(void) {
  geometry();
  colours();
  fixture_f0();
  fixture_f1();
  fixture_f2();
  fixture_f3();
  fixture_f5();
  fixture_f6();
  fixture_f7();
  fixture_f10();
  fixture_f13();
  fixture_f14();
  fixture_f19();
  fixture_f20();
  fixture_f21();
  fixture_f22();
  fixture_f24_keys();
  if (failures != 0u) {
    fprintf(stderr, "c64 colour field: %u of %u checks failed\n", failures, checks);
    return 1;
  }
  printf("c64 colour field: %u checks passed\n", checks);
  return 0;
}
