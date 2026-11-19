/* Host checks for the C64 colour-field core against the normative fixtures in
 * shared/specs/colour-field-v0.md. Operator-only (F11, F12, F16), resampled
 * (F15, F18) and host-side (F23) fixtures do not apply to the C64 target build.
 * Built twice: by default flash/gate never arms (#96), so every flash run
 * checks that 3 and 7 behave as unarmed 2 and 6; with -DCF_UNMEASURED_FLASH
 * the flash fixtures run as the spec states them. */
#include "colour_field.h"

#include <stdio.h>
#include <string.h>

#define STEPS 600u

#ifdef CF_UNMEASURED_FLASH
#define FLASH 1
#else
#define FLASH 0
#endif

typedef struct {
  uint8_t e, band[3], spec[8], flags, scene, palette, abs_off;
  uint8_t notes[2][2];
} frame_t;

typedef struct {
  uint8_t shown, mode, family, layout, combo, rot, lvl[3], fx, flash;
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
    tr[step].flash = cf.flash;
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

/* Runs a flash/gate fixture (palette 3 or 7). Returns 1 in a
 * CF_UNMEASURED_FLASH build, where the caller checks the spec's flash
 * results. In the default build it checks the downgrade instead and returns
 * 0: no step is in mode 3, latches flash or pulses, and a local-palette run
 * is step-for-step the run of palette p - 1 (hard cuts). */
static trace_t flash_ref[STEPS];
static int run_flash(gen_fn gen, uint8_t p, uint8_t wire, const char *fixture) {
  run(gen, p, wire);
  if (!FLASH) {
    uint16_t s;
    int ok = 1;
    for (s = 0; s < STEPS; s++) {
      ok &= tr[s].mode != 3u && !tr[s].flash && !(tr[s].fx & CF_FX_PULSE);
    }
    require(ok, fixture, "default build: never armed, latched or pulsed");
    if (!wire) {
      memcpy(flash_ref, tr, sizeof(tr));
      run(gen, (uint8_t)(p - 1u), wire);
      require(memcmp(flash_ref, tr, sizeof(tr)) == 0, fixture, "default build: 3/7 run exactly as 2/6");
    }
  }
  return FLASH;
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

static void g_f4(uint16_t step, frame_t *f) {
  fill(f, 80);
  if (step == 45u || step == 46u || step == 130u || step == 352u) {
    f->notes[0][0] = 60;
    f->notes[0][1] = 100;
  } else if (step == 200u) {
    f->notes[0][0] = 60;
    f->notes[0][1] = 40;
  }
}

static void g_f8(uint16_t step, frame_t *f) {
  fill(f, 80);
  if (step >= 100u && step <= 299u) {
    f->notes[0][0] = 60;
    f->notes[0][1] = 100;
  }
  if (step >= 200u && step <= 299u) {
    f->notes[1][0] = 64;
    f->notes[1][1] = 100;
  }
}

static void g_f9(uint16_t step, frame_t *f) {
  fill(f, 100);
  if (step >= 20u && step <= 580u && (step - 20u) % 40u == 0u) {
    f->flags |= 1u;
  }
  if (step >= 40u && step <= 520u && (step - 40u) % 120u == 0u) {
    f->flags |= 2u;
  }
}

static void g_f17(uint16_t step, frame_t *f) {
  f->e = (uint8_t)(step > 0u && step % 31u == 0u && step / 31u <= 19u ? 24u : 0u);
  f->flags = (uint8_t)(step & 1u);
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
    /* Without the flash build 3 and 7 are hard cuts, lit like 2 and 6. */
    uint8_t lit = (uint8_t)((p & 1u) == 0u || (!FLASH && (p & 3u) == 3u));
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
    if (run_flash(g_f1, (uint8_t)(base + 3u), 0, "F1")) {
      require(fx_steps(CF_FX_PULSE, at300) && tr[300].combo == 1 && tr[300].rot == 1 && tr[300].lvl[0] == 3,
              "F1", "flash pulse combo 1 rot 1");
    }
  }
}

static void fixture_f2(void) {
  run(g_f2, 0, 0);
  require(constant(), "F2", "held constant");
  run(g_f2, 1, 0);
  require(constant() && levels(0, 2, 2), "F2", "spectral 2/2");
  run(g_f2, 2, 0);
  require(fx_count(CF_FX_CUT | CF_FX_T) == 0, "F2", "no events");
  if (run_flash(g_f2, 3, 0, "F2")) {
    require(constant() && levels(0, 1, 1) && fx_count(CF_FX_PULSE) == 0, "F2", "flash rests dim");
  }
}

static void fixture_f3(void) {
  static const uint16_t cuts[] = {37, 101, 250, 400, 523, 0};
  static const uint16_t pulses[] = {37, 101, 113, 250, 400, 523, 0};
  run(g_f3, 0, 0);
  require(constant(), "F3", "held never changes");
  run(g_f3, 2, 0);
  require(fx_steps(CF_FX_CUT, cuts), "F3", "cuts at 37 101 250 400 523");
  if (run_flash(g_f3, 3, 0, "F3")) {
    require(fx_steps(CF_FX_PULSE, pulses), "F3", "pulses at all six");
  }
}

static void fixture_f4(void) {
  static const uint16_t at[] = {45, 130, 352, 0};
  run(g_f4, 2, 0);
  require(fx_steps(CF_FX_CUT, at), "F4", "cuts at 45 130 352");
  if (run_flash(g_f4, 3, 0, "F4")) {
    require(fx_steps(CF_FX_PULSE, at), "F4", "pulses at 45 130 352");
  }
}

static void fixture_f8(void) {
  static const uint16_t at[] = {100, 200, 0};
  run(g_f8, 2, 0);
  require(fx_steps(CF_FX_CUT, at), "F8", "cuts at 100 and 200");
  if (run_flash(g_f8, 3, 0, "F8")) {
    require(fx_steps(CF_FX_PULSE, at), "F8", "pulses at 100 and 200");
  }
}

/* Beats at 20 + 40k and bars (B from bar, the Return key on C64) at 40 + 120k
 * never coincide: 20 B events. */
static void fixture_f9(void) {
  static uint16_t at[21];
  uint16_t s;
  uint8_t n = 0;
  for (s = 20; s <= 580u; s++) {
    if ((s - 20u) % 40u == 0u || (s >= 40u && (s - 40u) % 120u == 0u && s <= 520u)) {
      at[n++] = s;
    }
  }
  at[n] = 0;
  require(n == 20u, "F9", "20 beat/bar steps");
  run(g_f9, 0, 0);
  require(constant(), "F9", "held never changes");
  run(g_f9, 2, 0);
  require(fx_steps(CF_FX_CUT, at), "F9", "a cut at every beat and bar");
  if (run_flash(g_f9, 3, 0, "F9")) {
    require(fx_steps(CF_FX_PULSE, at), "F9", "a pulse at every beat and bar");
  }
}

static void fixture_f17(void) {
  uint8_t p;
  for (p = 3; p < 8u; p += 4u) {
    uint16_t s;
    uint16_t n = 0;
    uint16_t last = 0;
    int ok = 1;
    if (!run_flash(g_f17, p, 0, "F17")) {
      continue;
    }
    for (s = 0; s < STEPS; s++) {
      if (tr[s].fx & CF_FX_PULSE) {
        if (n < 23u) {
          ok &= s == 1u + 8u * n;
        } else if (n == 23u) {
          ok &= s == 187u;
        } else if (n == 24u) {
          ok &= s == 195u;
        }
        ok &= n == 0u || s - last >= 8u;
        last = s;
        n++;
      }
    }
    require(ok && n == 75u, "F17", "75 pulse starts 1+8k to 177, then 187, 195, never closer than 8");
    require(!(tr[127].fx & CF_FX_PULSE), "F17", "since_pulse holds off a start at 127");
    for (s = 30; s < STEPS; s = (uint16_t)(s + 31u)) {
      ok &= tr[s].lvl[0] != 3u;
    }
    require(ok, "F17", "silence entries at 31k - 1 are not lit");
    require(max_changes_per_60() <= 16, "F17", "at most 16 displayed changes in 60 steps");
  }
}

static void fixture_f5(void) {
  static const uint16_t layouts[] = {120, 240, 360, 480, 0};
  run(g_f5, 0, 0);
  require(constant(), "F5", "held never changes");
  run(g_f5, 2, 0);
  require(fx_count(CF_FX_CUT) == 19 && fx_steps(CF_FX_LAYOUT, layouts), "F5", "19 cuts, layouts every 120");
  require(tr[120].layout == 1 && tr[240].layout == 2 && tr[360].layout == 3 && tr[480].layout == 0, "F5",
          "V2 H2 V3 H3 V2");
  if (run_flash(g_f5, 3, 0, "F5")) {
    require(fx_count(CF_FX_PULSE) == 19, "F5", "19 pulses");
  }
}

static void fixture_f6(void) {
  static const uint16_t changes[] = {60, 70, 420, 430, 480, 490, 0};
  static const uint16_t hits[] = {420, 480, 0};
  run(g_f6, 1, 0);
  require(level_changes(STEPS, changes) && levels(59, 2, 2) && levels(70, 0, 0) && levels(480, 3, 3) &&
              levels(490, 2, 2), "F6", "spectral steps");
  run(g_f6, 2, 0);
  require(fx_steps(CF_FX_CUT, hits), "F6", "cuts at 420 and 480");
  if (run_flash(g_f6, 3, 0, "F6")) {
    require(fx_steps(CF_FX_PULSE, hits), "F6", "pulses at 420 and 480");
  }
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
  if (run_flash(g_f7, 3, 0, "F7")) {
    require(fx_steps(CF_FX_PULSE, at300), "F7", "pulse at 300");
  }
}

static void fixture_f10(void) {
  if (run_flash(g_f10, 0, 1, "F10")) {
    require(tr[229].mode == 0 && tr[230].mode == 3 && tr[599].mode == 3, "F10", "mode 3 from 230");
    require(levels(230, 1, 1), "F10", "resting dim");
  } else {
    require(tr[229].mode == 0 && tr[230].mode == 2 && tr[599].mode == 2, "F10", "default build: mode 2 from 230");
  }
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
  if (run_flash(g_f14, 3, 0, "F14")) {
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
  }
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
  if (run_flash(g_f19, 3, 0, "F19")) {
    require(fx_steps(CF_FX_PULSE, cuts), "F19", "11 pulses");
    for (s = 0; s < STEPS; s++) {
      dim &= tr[s].lvl[0] != 0;
    }
    require(dim, "F19", "flash rest dim, never 0");
  }
}

static void fixture_f20(void) {
  static const uint16_t spec[] = {200, 210, 220, 400, 410, 420, 0};
  static const uint16_t at400[] = {400, 0};
  run(g_f20, 1, 0);
  require(level_changes(STEPS, spec) && levels(0, 3, 3) && levels(220, 0, 0) && levels(420, 3, 3), "F20",
          "spectral saturation");
  run(g_f20, 2, 0);
  require(fx_steps(CF_FX_CUT, at400), "F20", "one cut at 400");
  if (run_flash(g_f20, 3, 0, "F20")) {
    require(fx_steps(CF_FX_PULSE, at400) && levels(0, 1, 1) && levels(599, 1, 1), "F20", "one pulse, dim rest");
  }
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
  int armed = run_flash(g_f22, 0, 1, "F22");
  for (s = 0; s < STEPS; s++) {
    ok &= tr[s].shown;
  }
  require(ok, "F22", "scene 7 shown throughout");
  if (armed) {
    require(tr[329].mode == 0 && tr[330].mode == 3 && levels(330, 1, 1), "F22", "arms at 329, mode 3 from 330");
  } else {
    require(tr[329].mode == 0 && tr[330].mode == 2 && tr[599].mode == 2, "F22", "default build: mode 2 from 330");
  }
}

static void fixture_f24_keys(void) {
  f24_abs_off = 0;
  if (run_flash(g_f24, 0, 1, "F24")) {
    require(tr[59].mode == 0 && tr[60].mode == 3 && tr[159].mode == 3 && tr[160].mode == 2, "F24",
            "absolute R arms at 59, mode 3 from 60 to 159");
  } else {
    require(tr[59].mode == 0 && tr[60].mode == 2 && tr[599].mode == 2 && fx_count(CF_FX_APPLY) == 1, "F24",
            "default build: absolute R runs as 2 and never arms");
  }
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

/* Keyboard matrix bits (CIA1 column, row mask) for the keys used below. */
static void key(uint8_t column, uint8_t mask) {
  cf_keys[column] |= mask;
}

static void keys_none(void) {
  memset(cf_keys, 0, sizeof(cf_keys));
}

/* Runs n steps of the keyboard poll with the given held keys; returns 1 when
 * any step was in mode 3 (flash/gate armed). */
static uint8_t key_shown; /* cf_tick's result on the last key step */
static int key_steps(uint16_t n, void (*held)(void)) {
  int armed = 0;
  while (n-- > 0u) {
    keys_none();
    held();
    cf_read_keys(2u);
    key_shown = cf_tick();
    armed |= cf_mode() == 3u;
  }
  return armed;
}

static void held_8(void) { key(3, 0x08u); }
static void held_8q(void) { key(3, 0x08u); key(7, 0x40u); }
static void held_8r(void) { key(3, 0x08u); key(2, 0x02u); }
static void held_r(void) { key(2, 0x02u); }
static void held_18r(void) { key(7, 0x01u); key(3, 0x08u); key(2, 0x02u); }

static void keys(void) {
  cf_init();
  keys_none();
  cf_read_keys(2u);
  require(cf_in.scene == 2u && !cf_in.scene_abs && cf_in.palette == 0u && cf_in.wire && cf_in.energy == CF_HID_E_LO &&
              cf_in.band[2] == CF_HID_E_LO && cf_in.spectrum[0] == 0u && cf_in.events == 0u, "keys",
          "no keys: procedural scene, HID_E_LO, palette 0");
  held_8r();
  cf_read_keys(2u);
  require(cf_in.scene == 7u && cf_in.palette == 3u && cf_in.scene_abs, "keys", "8+R is an absolute 7/3 report");
  keys_none();
  cf_read_keys(2u);
  require(cf_in.scene == 7u && cf_in.palette == 3u && !cf_in.scene_abs, "keys", "released keys stay latched");
  held_18r();
  cf_read_keys(2u);
  require(cf_in.scene == 7u && !cf_in.scene_abs, "keys", "two scene keys: no scene, not absolute");
  keys_none();
  key(7, 0x10u); /* Space */
  cf_read_keys(2u);
  require(cf_in.events == CF_EV_B, "keys", "Space press is a beat edge");
  cf_read_keys(2u);
  require(cf_in.events == 0u, "keys", "held Space does not repeat");
  key(0, 0x02u); /* Return while Space is held */
  cf_read_keys(2u);
  require(cf_in.events == CF_EV_B, "keys", "Return press is a bar edge");
  keys_none();
  cf_read_keys(2u);
  require(cf_in.events == 0u, "keys", "release is not an edge");

  /* Palette R without 8 (or with several scene keys) never arms flash/gate. */
  cf_init();
  require(!key_steps(40, held_8q) && !key_steps(200, held_r), "keys", "R without 8 never arms");
  cf_init();
  require(!key_steps(40, held_8q) && !key_steps(200, held_18r), "keys", "R with 1 and 8 never arms");
  require(key_shown && cf_mode() == 2u, "keys", "unarmed R runs as hard cuts in scene 7");
  cf_init();
  if (FLASH) {
    require(!key_steps(40, held_8q) && key_steps(200, held_8r), "keys", "8+R arms flash/gate");
  } else {
    require(!key_steps(40, held_8q) && !key_steps(200, held_8r) && key_shown && cf_mode() == 2u && !cf.flash,
            "keys", "default build: 8+R runs as hard cuts and never latches");
  }
  cf_init();
  key_steps(1, keys_none); /* power-on adopts the procedural scene 2 */
  require(!key_steps(30, held_8) && !key_shown, "keys", "8 not shown on its first 30 steps");
  key_steps(1, held_8);
  require(key_shown, "keys", "8 shows scene 7 from the 31st step");
}

/* The raster IRQ's on-time check for every line of a PAL (312 lines, IRQ at
 * 300) and an NTSC (263 lines, IRQ at 16) frame: the main loop reads D012 and
 * then D011, so bit 8 may also come from the next line. */
static int irq_window(uint16_t line, uint16_t frame_lines) {
  return frame_lines == 312u ? (line >= 300u || line <= 3u) : (line >= 16u && line <= 31u);
}

static uint8_t irq_read(uint16_t low_line, uint16_t high_line, uint16_t irq_line, uint16_t frame_lines) {
  return cf_irq_on_time((uint8_t)low_line, (uint8_t)((high_line >> 1) & 0x80u), irq_line, frame_lines);
}

static void irq_lines(void) {
  static const uint16_t models[2][2] = {{300u, 312u}, {16u, 263u}};
  uint8_t m;
  for (m = 0; m < 2u; m++) {
    uint16_t irq_line = models[m][0];
    uint16_t frame_lines = models[m][1];
    uint16_t line;
    uint16_t accepted = 0;
    int exact = 1;
    int race = 1;
    for (line = 0; line < frame_lines; line++) {
      uint16_t next = (uint16_t)((line + 1u) % frame_lines);
      uint8_t on_time = irq_read(line, line, irq_line, frame_lines);
      exact &= on_time == irq_window(line, frame_lines);
      accepted = (uint16_t)(accepted + on_time);
      /* The line moved on between the two reads (bit 8 from the next line):
       * accept only when the line read from D012 is inside the window, so a
       * 255 -> 256 or last-line -> 0 mix can never pass. */
      if (irq_read(line, next, irq_line, frame_lines)) {
        race &= irq_window(line, frame_lines);
      }
    }
    require(exact && accepted == 16u, m ? "irq NTSC" : "irq PAL", "accepted lines are exactly the window");
    require(race, m ? "irq NTSC" : "irq PAL", "a D012/D011 read race is never accepted outside the window");
  }
  /* The overrun the window must reject: about a frame late, at the bottom of
   * the display or the lower border (PAL 244-255 with bit 8 clear). */
  require(!irq_read(244u, 244u, 300u, 312u) && !irq_read(255u, 255u, 300u, 312u), "irq PAL",
          "lines 244-255 are late");
}

int main(void) {
  geometry();
  irq_lines();
  colours();
  keys();
  fixture_f0();
  fixture_f1();
  fixture_f2();
  fixture_f3();
  fixture_f4();
  fixture_f5();
  fixture_f6();
  fixture_f7();
  fixture_f8();
  fixture_f9();
  fixture_f10();
  fixture_f13();
  fixture_f14();
  fixture_f17();
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
