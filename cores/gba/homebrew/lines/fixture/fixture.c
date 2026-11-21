#include "fixture.h"
#include "../lines.h"

#include <string.h>

static void put16(uint8_t *p, unsigned value) {
  p[0] = (uint8_t)value;
  p[1] = (uint8_t)(value >> 8);
}

static void put32(uint8_t *p, uint32_t value) {
  put16(p, (unsigned)value);
  put16(p + 2, (unsigned)(value >> 16));
}

static uint16_t keys_at(unsigned scenario, uint32_t step) {
  uint16_t keys = 0;
  if (!scenario && step && step % 30u == 0) {
    keys |= LINES_KEY_A | LINES_KEY_B;
  }
  if ((!scenario && step >= 200u && step < 700u) ||
      (scenario && step >= 520u && step < 900u)) {
    keys |= LINES_KEY_L;
  }
  return keys;
}

static void report(fixture_emit_t emit, volatile uint16_t *guarded, const lines_state_t *state,
                   const lines_render_t *render, unsigned seq, unsigned scenario,
                   unsigned flags, unsigned result, unsigned batches) {
  uint8_t h[FIXTURE_HEADER_SIZE] = {'G', 'L', 'F', 'X', 1};
  h[5] = (uint8_t)seq;
  h[7] = (uint8_t)scenario;
  put32(h + 8, state->step);
  put16(h + 12, state->phase);
  h[14] = state->fold;
  h[15] = state->energy;
  h[16] = state->target;
  h[17] = state->phrase_beats;
  put16(h + 18, state->beat_age);
  put16(h + 20, state->section_age);
  put16(h + 22, (unsigned)render->next);
  put16(h + 24, guarded[0]);
  put16(h + 26, guarded[LINES_PAGE_HALFWORDS + 1u]);
  h[28] = (uint8_t)flags;
  h[29] = (uint8_t)result;
  put16(h + 30, batches);
  emit(h, guarded + 1, render->snapshot.palette);
}

void fixture_run(volatile uint16_t *guarded, fixture_emit_t emit) {
  static const uint16_t steps[2][6] = {
    {0, 480, 511, 543, 960, 1023},
    {0, 512, 543, 575, 1024, 1087}
  };
  lines_state_t state, later;
  lines_snapshot_t snapshot, changed;
  lines_render_t render;
  unsigned scenario, item, seq = 0;
  for (scenario = 0; scenario < 2; ++scenario) {
    lines_init(&state);
    lines_observe(&state, 0, keys_at(scenario, 0));
    for (item = 0; item < 6; ++item) {
      unsigned flags = 0, batches = 0;
      int result = 0;
      size_t i;
      while (state.step < steps[scenario][item]) {
        uint32_t step = state.step + 1u;
        lines_observe(&state, step, keys_at(scenario, step));
      }
      guarded[0] = 0x5aa5u;
      guarded[LINES_PAGE_HALFWORDS + 1u] = 0xa55au;
      for (i = 0; i < LINES_PAGE_HALFWORDS; ++i) {
        guarded[i + 1u] = 0xa5a5u;
      }
      lines_snapshot(&state, &snapshot);
      lines_render_begin(&render, &snapshot);
      result = lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS - 1u, 37);
      for (i = 0; i < LINES_PAGE_HALFWORDS && guarded[i + 1u] == 0xa5a5u; ++i) {}
      if (result == -1 && render.next == 0 && i == LINES_PAGE_HALFWORDS) {
        flags |= 1u;
      }
      result = 0;
      later = state;
      lines_observe(&later, state.step + 8u, keys_at(scenario, state.step) ^ LINES_KEY_L);
      lines_snapshot(&later, &changed);
      if (memcmp(&snapshot, &render.snapshot, sizeof(snapshot)) == 0 &&
          memcmp(snapshot.palette, changed.palette, sizeof(snapshot.palette)) != 0) {
        flags |= 4u;
      }
      if (!scenario && item == 2) {
        for (i = 0; i < 5; ++i) {
          result = lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 37);
          ++batches;
        }
        lines_render_cancel(&render);
        if (result == 0 && lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 37) == 0 &&
            render.next == 185u && !render.active) {
          flags |= 2u;
        }
        report(emit, guarded, &state, &render, seq++, scenario, flags, 0, batches);
        lines_render_begin(&render, &snapshot);
        batches = 0;
      }
      while (result == 0 && batches < 600u) {
        result = lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS,
                                    item & 1u ? 240u : 37u);
        ++batches;
      }
      report(emit, guarded, &state, &render, seq++, scenario, flags,
             result == 1 ? 1u : 0u, batches);
    }
  }
}
