#include "lines.h"

#include <string.h>

#define PHRASE_BEATS 16u
#define MIN_SECTION_STEPS 192u
#define FALLBACK_STEPS 512u
#define FOLD_MAX 64u
#define PHASE_PERIOD 48u

void lines_init(lines_state_t *state) {
  memset(state, 0, sizeof(*state));
  state->energy = 16;
}

int lines_observe(lines_state_t *state, uint32_t step, uint16_t keys) {
  uint32_t delta, i;
  uint8_t energy_target = (keys & LINES_KEY_L) ? 32u : 16u;
  if (!state->initialized) {
    state->initialized = 1;
    state->step = step;
    state->keys = keys;
    return 1;
  }
  delta = step - state->step;
  if (!delta || delta > LINES_MAX_STEP_DELTA) {
    return 0;
  }
  for (i = 0; i < delta; ++i) {
    uint8_t phrase = 0;
    ++state->phase;
    if (state->phase == PHASE_PERIOD) {
      state->phase = 0;
    }
    if (state->section_age < MIN_SECTION_STEPS) {
      ++state->section_age;
    }
    if (state->beat_age < FALLBACK_STEPS) {
      ++state->beat_age;
    }
    /* A sampled edge belongs to the final step of this observation. */
    if (i + 1u == delta && (keys & LINES_KEY_A) && !(state->keys & LINES_KEY_A)) {
      state->beat_age = 0;
      if (++state->phrase_beats == PHRASE_BEATS) {
        state->phrase_beats = 0;
        phrase = 1;
      }
    }
    if (state->section_age >= MIN_SECTION_STEPS &&
        (phrase || state->beat_age >= FALLBACK_STEPS)) {
      state->target = (uint8_t)(FOLD_MAX - state->target);
      state->beat_age = 0;
      state->section_age = 0;
      state->phrase_beats = 0;
    }
    if (state->fold < state->target) {
      ++state->fold;
    } else if (state->fold > state->target) {
      --state->fold;
    }
    if (state->energy < energy_target) {
      ++state->energy;
    } else if (state->energy > energy_target) {
      --state->energy;
    }
  }
  state->step = step;
  state->keys = keys;
  return 1;
}

void lines_snapshot(const lines_state_t *state, lines_snapshot_t *snapshot) {
  unsigned level = state->energy;
  memset(snapshot, 0, sizeof(*snapshot));
  snapshot->phase = state->phase;
  snapshot->fold = state->fold;
  snapshot->energy = state->energy;
  /* Soft BGR555 ramp; the complete shared hardware palette is staged. */
  snapshot->palette[0] = 0x0801u;
  snapshot->palette[1] = (uint16_t)((level / 4u) | ((level / 2u) << 5) |
                                    ((level * 3u / 4u) << 10));
  snapshot->palette[2] = (uint16_t)((level / 2u) | ((level * 3u / 4u) << 5) |
                                    ((level * 3u / 4u) << 10));
}

void lines_render_begin(lines_render_t *render, const lines_snapshot_t *snapshot) {
  render->snapshot = *snapshot;
  render->next = 0;
  render->active = 1;
}

void lines_render_cancel(lines_render_t *render) {
  render->active = 0;
}

static unsigned pixel(const lines_snapshot_t *snapshot, unsigned x, unsigned y) {
  /* Doubled coordinates mirror around the gaps between the centre pixels. */
  unsigned dx = x < 120u ? 239u - 2u * x : 2u * x - 239u;
  unsigned dy = y < 80u ? 159u - 2u * y : 2u * y - 159u;
  unsigned radial = dx > dy * 3u / 2u ? dx * 2u / 3u : dy;
  unsigned depth = (dy * (FOLD_MAX - snapshot->fold) +
                    radial * snapshot->fold) / FOLD_MAX;
  unsigned spacing = 12u + depth / 8u;
  unsigned column = dx % spacing;
  unsigned cross = (depth + snapshot->phase / 2u) % 24u;
  if (cross < 2u) {
    return 2u;
  }
  return column < 2u ? 1u : 0u;
}

int lines_render_batch(lines_render_t *render, volatile uint16_t *page,
                       size_t page_halfwords, size_t budget) {
  size_t end;
  if (!page || page_halfwords < LINES_PAGE_HALFWORDS) {
    return -1;
  }
  if (!render->active) {
    return 0;
  }
  if (budget > LINES_BATCH_HALFWORDS) {
    budget = LINES_BATCH_HALFWORDS;
  }
  end = render->next + budget;
  if (end > LINES_PAGE_HALFWORDS) {
    end = LINES_PAGE_HALFWORDS;
  }
  while (render->next < end) {
    size_t index = render->next;
    unsigned x = (unsigned)(index % (LINES_WIDTH / 2u)) * 2u;
    unsigned y = (unsigned)(index / (LINES_WIDTH / 2u));
    unsigned left = pixel(&render->snapshot, x, y);
    unsigned right = pixel(&render->snapshot, x + 1u, y);
    page[index] = (uint16_t)(left | (right << 8));
    ++render->next;
  }
  if (render->next == LINES_PAGE_HALFWORDS) {
    render->active = 0;
    return 1;
  }
  return 0;
}
