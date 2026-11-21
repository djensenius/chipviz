#include "lines.h"

#include <stdio.h>
#include <string.h>

static unsigned checks, failures;
static uint16_t guarded[LINES_PAGE_HALFWORDS + 2u];
static uint16_t expected[LINES_PAGE_HALFWORDS];
static uint16_t plane[LINES_PAGE_HALFWORDS];
static uint16_t reference[LINES_PAGE_HALFWORDS];

static void check(int condition, const char *message) {
  ++checks;
  if (!condition) {
    ++failures;
    printf("FAIL: %s\n", message);
  }
}

static int finish(lines_render_t *render, uint16_t *page, size_t budget) {
  int result;
  do {
    result = lines_render_batch(render, page, LINES_PAGE_HALFWORDS, budget);
  } while (result == 0 && render->active);
  return result;
}

/* Scalar one-shot oracle, writing individual indices to ordinary host RAM. */
static void render_reference(const lines_snapshot_t *snapshot) {
  unsigned x, y;
  memset(reference, 0, sizeof(reference));
  for (y = 0; y < LINES_HEIGHT; ++y) {
    for (x = 0; x < LINES_WIDTH; ++x) {
      unsigned dx = x < 120u ? 239u - 2u * x : 2u * x - 239u;
      unsigned dy = y < 80u ? 159u - 2u * y : 2u * y - 159u;
      unsigned side = dx * 2u / 3u;
      unsigned depth = (dy * (64u - snapshot->fold) +
                        (dx > dy * 3u / 2u ? side : dy) * snapshot->fold) / 64u;
      unsigned color = (depth + snapshot->phase / 2u) % 24u < 2u ? 2u :
                       dx % (12u + depth / 8u) < 2u ? 1u : 0u;
      reference[y * 120u + x / 2u] |= (uint16_t)(color << ((x & 1u) * 8u));
    }
  }
}

static void samples(void) {
  lines_state_t a, b;
  lines_snapshot_t sa, sb;
  unsigned i;
  lines_init(&a);
  lines_init(&b);
  for (i = 0; i < 400; ++i) {
    uint16_t keys = (i % 37u < 4u ? LINES_KEY_A | LINES_KEY_B : 0u) |
                    (i % 100u < 50u ? LINES_KEY_L : 0u);
    check(lines_observe(&a, i, keys) && lines_observe(&b, i, keys), "replay samples accepted");
  }
  lines_snapshot(&a, &sa);
  lines_snapshot(&b, &sb);
  check(memcmp(&sa, &sb, sizeof(sa)) == 0, "deterministic replay");

  lines_init(&a);
  check(lines_observe(&a, 0, LINES_KEY_A), "first held sample");
  check(lines_observe(&a, 1, LINES_KEY_A | LINES_KEY_B), "held A with B");
  check(a.phrase_beats == 0, "entry hold and B are not beats");
  lines_observe(&a, 2, 0);
  lines_observe(&a, 3, LINES_KEY_A | LINES_KEY_B);
  check(a.phrase_beats == 1 && a.target == 0, "simultaneous A+B is one A beat");
  lines_observe(&a, 4, LINES_KEY_A);
  check(a.phrase_beats == 1, "held A does not repeat");
  for (i = 5; i <= 514; ++i) {
    lines_observe(&a, i, LINES_KEY_A);
  }
  check(a.target == 0 && a.phrase_beats == 1, "partial phrase waits for stall timeout");
  lines_observe(&a, 515, LINES_KEY_A);
  check(a.target == 64 && a.fold == 1 && a.phrase_beats == 0,
        "stalled fallback starts gradual fold and resets phrase");
  for (i = 516; i <= 578; ++i) {
    lines_observe(&a, i, LINES_KEY_A);
  }
  check(a.fold == 64, "stalled fallback reaches hallway");
  for (i = 579; i <= 1026; ++i) {
    lines_observe(&a, i, LINES_KEY_A);
  }
  check(a.fold == 64 && a.target == 64, "stalled hallway remains stable");
  lines_observe(&a, 1027, LINES_KEY_A);
  check(a.target == 0 && a.fold == 63, "stalled fallback returns toward plane");
  for (i = 1028; i <= 1090; ++i) {
    lines_observe(&a, i, LINES_KEY_A);
  }
  check(a.fold == 0, "stalled fallback reaches plane");

  lines_init(&a);
  lines_observe(&a, 0, 0);
  for (i = 1; i < 512; ++i) {
    lines_observe(&a, i, 0);
  }
  lines_observe(&a, 512, LINES_KEY_A);
  check(a.target == 0 && a.phrase_beats == 1 && a.beat_age == 0,
        "edge at stall boundary wins over fallback");

  lines_init(&a);
  lines_observe(&a, 0, 0);
  for (i = 1; i <= 508; ++i) {
    lines_observe(&a, i, 0);
  }
  b = a;
  lines_observe(&a, 516, LINES_KEY_A);
  for (i = 509; i <= 516; ++i) {
    lines_observe(&b, i, i == 516 ? LINES_KEY_A : 0);
  }
  check(memcmp(&a, &b, sizeof(a)) == 0 && a.target == 64 && a.phrase_beats == 1,
        "batched steps process fallback before a later edge");

  lines_init(&a);
  lines_observe(&a, UINT32_MAX, 0);
  check(lines_observe(&a, 0, LINES_KEY_A), "step counter wraps");
  check(a.phrase_beats == 1, "wrapped step processes edge");
  b = a;
  check(!lines_observe(&a, 0, 0), "duplicate sample ignored");
  check(!lines_observe(&a, 9, 0), "oversized delta rejected");
  check(memcmp(&a, &b, sizeof(a)) == 0, "rejected samples leave state unchanged");
  check(lines_observe(&a, 1, LINES_KEY_A | LINES_KEY_L), "next valid sample accepted");
  check(a.energy == 17, "L energy rises gently");
  lines_observe(&a, 2, 0);
  check(a.energy == 16, "released energy returns gently");
}

static void regular_phrases(unsigned rate) {
  lines_state_t state;
  unsigned i, beats = 0, switches = 0, plane_steps = 0, hall_steps = 0;
  lines_init(&state);
  lines_observe(&state, 0, 0);
  for (i = 1; i <= 2000; ++i) {
    unsigned edge = i * rate / 3600u != (i - 1u) * rate / 3600u;
    uint8_t previous_target = state.target, previous_fold = state.fold;
    uint16_t keys = edge ? LINES_KEY_A | LINES_KEY_B : 0u;
    beats += edge;
    lines_observe(&state, i, keys);
    if (state.target != previous_target) {
      ++switches;
      check(edge && beats % 16u == 0, "regular input switches on phrase boundary");
    }
    check((state.target != previous_target) == (edge && beats % 16u == 0),
          "every regular phrase switches exactly once");
    check(state.fold <= previous_fold + 1u && previous_fold <= state.fold + 1u,
          "regular phrase fold is gradual");
    plane_steps += state.fold == 0;
    hall_steps += state.fold == 64;
  }
  check(switches >= 2 && plane_steps >= 100 && hall_steps >= 100,
        "regular input reaches and holds both extremes");
}

static void rapid_phrases(void) {
  lines_state_t state;
  unsigned i, last_switch = 0, switches = 0;
  lines_init(&state);
  lines_observe(&state, 0, 0);
  for (i = 1; i <= 700; ++i) {
    uint8_t target = state.target;
    lines_observe(&state, i, i & 1u ? LINES_KEY_A : 0);
    if (state.target != target) {
      check(i - last_switch >= 192, "rapid phrase respects minimum section");
      check(i == 223u + switches * 192u, "early phrase boundaries are discarded");
      if (switches) {
        check(state.fold == (target ? 63 : 1), "rapid phrase starts from an extreme");
      }
      last_switch = i;
      ++switches;
    }
  }
  check(switches >= 2, "rapid input eventually changes sections");
}

static void rendering(void) {
  lines_state_t state;
  lines_snapshot_t snapshot, frozen;
  lines_render_t render, one;
  size_t i;
  unsigned x, y, differences = 0;
  int result;
  lines_init(&state);
  lines_observe(&state, 0, 0);
  lines_snapshot(&state, &snapshot);
  frozen = snapshot;
  lines_render_begin(&one, &snapshot);
  check(finish(&one, plane, LINES_BATCH_HALFWORDS) == 1, "plane completed");
  for (y = 0; y < LINES_HEIGHT; ++y) {
    for (x = 0; x < LINES_WIDTH / 2u; ++x) {
      uint16_t left = plane[y * 120u + x];
      uint16_t right = plane[y * 120u + (119u - x)];
      check((left & 255u) == (right >> 8) && (left >> 8) == (right & 255u),
            "plane mirrored horizontally");
      check(left == plane[(159u - y) * 120u + x], "plane mirrored vertically");
    }
  }
  for (i = 1; i <= 543; ++i) {
    lines_observe(&state, (uint32_t)i, LINES_KEY_L | (i % 30u == 0 ? LINES_KEY_A : 0));
  }
  check(state.fold == 64, "phrase folds plane into hallway");
  lines_snapshot(&state, &snapshot);
  check(memcmp(frozen.palette, snapshot.palette, sizeof(snapshot.palette)) != 0, "L changes staged palette");
  lines_render_begin(&one, &snapshot);
  check(finish(&one, expected, LINES_BATCH_HALFWORDS) == 1, "hallway completed");
  render_reference(&snapshot);
  check(memcmp(expected, reference, sizeof(reference)) == 0, "bounded output matches one-shot scalar oracle");
  for (i = 0; i < LINES_PAGE_HALFWORDS; ++i) {
    differences += expected[i] != plane[i];
  }
  check(differences > 1000, "fold changes geometry");

  memset(guarded, 0xa5, sizeof(guarded));
  lines_render_begin(&render, &snapshot);
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS - 1u, 100) == -1,
        "short destination rejected");
  check(lines_render_batch(&render, NULL, LINES_PAGE_HALFWORDS, 100) == -1, "null destination rejected");
  check(render.next == 0 && guarded[1] == 0xa5a5u, "invalid destination does not write or progress");
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 0) == 0 && render.next == 0,
        "zero budget does not progress");
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, (size_t)-1) == 0 &&
        render.next == LINES_BATCH_HALFWORDS, "large budget capped");
  lines_render_cancel(&render);
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 20) == 0 &&
        render.next == LINES_BATCH_HALFWORDS, "cancel stops writes");
  lines_render_begin(&render, &snapshot);
  frozen = snapshot;
  lines_observe(&state, 544, 0);
  lines_observe(&state, 545, LINES_KEY_A);
  check(memcmp(&snapshot, &frozen, sizeof(frozen)) == 0, "later observations do not mutate snapshot palette");
  lines_snapshot(&state, &snapshot);
  check(memcmp(&render.snapshot, &frozen, sizeof(frozen)) == 0, "render snapshot and palette frozen");
  result = finish(&render, guarded + 1, 37);
  check(result == 1 && memcmp(guarded + 1, expected, sizeof(expected)) == 0,
        "restart replaces whole page and incremental matches full-budget output");
  check(guarded[0] == 0xa5a5u && guarded[LINES_PAGE_HALFWORDS + 1u] == 0xa5a5u,
        "page canaries intact");
  for (i = 0; i < LINES_PAGE_HALFWORDS; ++i) {
    check(guarded[i + 1u] != 0xa5a5u, "every halfword replaced");
  }
}

int main(void) {
  samples();
  regular_phrases(96);
  regular_phrases(120);
  regular_phrases(126);
  rapid_phrases();
  rendering();
  printf("gba lines tests: %u checks, %u failures\n", checks, failures);
  return failures ? 1 : 0;
}
