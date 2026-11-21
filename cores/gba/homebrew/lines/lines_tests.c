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

static unsigned pixel_at(const uint16_t *page, unsigned x, unsigned y) {
  return (page[y * (LINES_WIDTH / 2u) + x / 2u] >> ((x & 1u) * 8u)) & 255u;
}

static int prefix_and_canaries_match(size_t written) {
  size_t i;
  if (written > LINES_PAGE_HALFWORDS ||
      guarded[0] != 0xa5a5u || guarded[LINES_PAGE_HALFWORDS + 1u] != 0xa5a5u) {
    return 0;
  }
  for (i = 0; i < LINES_PAGE_HALFWORDS; ++i) {
    if (guarded[i + 1u] != (i < written ? expected[i] : 0xa5a5u)) {
      return 0;
    }
  }
  return 1;
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

  lines_init(&a);
  lines_init(&b);
  lines_observe(&a, 0, 0);
  lines_observe(&b, 0, 0);
  check(lines_observe(&a, 4, LINES_KEY_A | LINES_KEY_B | LINES_KEY_L),
        "multi-step L and A sample accepted");
  for (i = 1; i <= 4; ++i) {
    lines_observe(&b, i, LINES_KEY_L | (i == 4 ? LINES_KEY_A | LINES_KEY_B : 0));
  }
  check(memcmp(&a, &b, sizeof(a)) == 0 && a.energy == 20 &&
        a.phrase_beats == 1 && a.beat_age == 0,
        "new L applies across the gap and A+B edge on the final step");
  b = a;
  check(lines_observe(&a, 8, LINES_KEY_A | LINES_KEY_B),
        "multi-step L release accepted");
  for (i = 5; i <= 8; ++i) {
    lines_observe(&b, i, LINES_KEY_A | LINES_KEY_B);
  }
  check(memcmp(&a, &b, sizeof(a)) == 0 && a.energy == 16 &&
        a.phrase_beats == 1 && a.beat_age == 4,
        "released L applies across the gap without a held A repeat");
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

static void phase_wrap(void) {
  lines_state_t state;
  lines_snapshot_t before, after, old_phase;
  lines_render_t render;
  uint32_t i;
  int accepted;
  lines_init(&state);
  accepted = lines_observe(&state, 0, 0);
  for (i = 1; i <= 65520u; ++i) {
    accepted &= lines_observe(&state, i, 0);
  }
  lines_snapshot(&state, &before);
  before.fold = 32;
  old_phase = before;
  old_phase.phase = 65520u;
  for (; i <= 65568u; ++i) {
    accepted &= lines_observe(&state, i, 0);
  }
  lines_snapshot(&state, &after);
  after.fold = 32;
  check(accepted, "long-running phase samples accepted");
  check(memcmp(before.palette, after.palette, sizeof(before.palette)) == 0,
        "phase comparison uses the same palette");
  lines_render_begin(&render, &before);
  check(finish(&render, plane, LINES_BATCH_HALFWORDS) == 1, "pre-wrap phase page completed");
  lines_render_begin(&render, &old_phase);
  check(finish(&render, reference, LINES_BATCH_HALFWORDS) == 1, "legacy phase page completed");
  check(memcmp(plane, reference, sizeof(plane)) == 0,
        "pre-wrap phase preserves legacy pixels");
  lines_render_begin(&render, &after);
  check(finish(&render, expected, LINES_BATCH_HALFWORDS) == 1, "post-wrap phase page completed");
  check(memcmp(plane, expected, sizeof(plane)) == 0,
        "pages 48 steps apart match across the former 16-bit phase wrap");
}

static void phase_boundaries(void) {
  lines_state_t state;
  lines_snapshot_t initial, p15 = {0}, p16 = {0}, p47 = {0}, p48 = {0};
  lines_snapshot_t before = {0}, after = {0};
  lines_render_t render;
  uint32_t i;
  int accepted, phases_correct = 1;
  lines_init(&state);
  accepted = lines_observe(&state, 0, 0);
  lines_snapshot(&state, &initial);
  for (i = 1; i <= 65536u; ++i) {
    accepted &= lines_observe(&state, i, 0);
    phases_correct &= state.phase == i % 48u;
    if (i == 15u) lines_snapshot(&state, &p15);
    if (i == 16u) lines_snapshot(&state, &p16);
    if (i == 47u) lines_snapshot(&state, &p47);
    if (i == 48u) lines_snapshot(&state, &p48);
    if (i == 65535u) lines_snapshot(&state, &before);
    if (i == 65536u) lines_snapshot(&state, &after);
  }
  check(accepted && phases_correct, "phase advances on every sampled step");
  check(p15.phase == 15 && p16.phase == 16 && p47.phase == 47 && p48.phase == 0 &&
        before.phase == 15 && after.phase == 16, "phase crosses both wrap boundaries");
  initial.fold = p15.fold = p16.fold = p47.fold = p48.fold = before.fold = after.fold = 32;
  check(memcmp(initial.palette, before.palette, sizeof(initial.palette)) == 0 &&
        memcmp(initial.palette, after.palette, sizeof(initial.palette)) == 0,
        "phase boundary pages use the same palette");
  lines_render_begin(&render, &initial);
  check(finish(&render, plane, LINES_BATCH_HALFWORDS) == 1, "initial phase page completed");
  lines_render_begin(&render, &p47);
  check(finish(&render, expected, LINES_BATCH_HALFWORDS) == 1, "phase 47 page completed");
  lines_render_begin(&render, &p48);
  check(finish(&render, reference, LINES_BATCH_HALFWORDS) == 1, "phase 48 page completed");
  check(memcmp(plane, reference, sizeof(plane)) == 0 &&
        memcmp(expected, reference, sizeof(expected)) != 0 &&
        pixel_at(expected, 117, 79) == 2 && pixel_at(reference, 117, 79) == 0,
        "phase 47 moves into phase 0 rather than freezing");
  lines_render_begin(&render, &p15);
  check(finish(&render, plane, LINES_BATCH_HALFWORDS) == 1, "phase 15 page completed");
  lines_render_begin(&render, &before);
  check(finish(&render, expected, LINES_BATCH_HALFWORDS) == 1, "step 65535 page completed");
  check(memcmp(plane, expected, sizeof(plane)) == 0 &&
        pixel_at(expected, 93, 79) == 2, "step 65535 matches phase 15");
  lines_render_begin(&render, &p16);
  check(finish(&render, plane, LINES_BATCH_HALFWORDS) == 1, "phase 16 page completed");
  lines_render_begin(&render, &after);
  check(finish(&render, reference, LINES_BATCH_HALFWORDS) == 1, "step 65536 page completed");
  check(memcmp(plane, reference, sizeof(plane)) == 0 &&
        memcmp(expected, reference, sizeof(expected)) != 0 &&
        pixel_at(reference, 93, 79) == 0,
        "step 65536 moves to phase 16 without a discontinuity");
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

static void batch_contract(void) {
  lines_state_t state;
  lines_snapshot_t snapshot;
  lines_render_t render = {0};
  size_t previous;
  unsigned i;
  int result, prefix_untouched;
  lines_init(&state);
  lines_observe(&state, 0, 0);
  lines_snapshot(&state, &snapshot);
  render_reference(&snapshot);
  memcpy(expected, reference, sizeof(expected));
  memset(guarded, 0xa5, sizeof(guarded));

  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 17) == 0 &&
        render.next == 0 && prefix_and_canaries_match(0), "fresh render is incomplete");
  lines_render_cancel(&render);
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 17) == 0 &&
        render.next == 0 && prefix_and_canaries_match(0), "cancelled fresh render is incomplete");
  lines_render_begin(&render, &snapshot);
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS - 1u, 17) == -1 &&
        render.next == 0 && prefix_and_canaries_match(0), "short destination leaves whole page untouched");
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 0) == 0 &&
        render.next == 0 && prefix_and_canaries_match(0), "zero batch leaves whole page untouched");
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 17) == 0 &&
        render.next == 17 && prefix_and_canaries_match(17), "small batch writes exactly its prefix");
  guarded[18] = 0;
  check(!prefix_and_canaries_match(17), "prefix check detects an out-of-budget write");
  guarded[18] = 0xa5a5u;
  lines_render_cancel(&render);
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 240) == 0 &&
        render.next == 17 && prefix_and_canaries_match(17),
        "cancelled partial render remains incomplete without writes");

  memset(guarded, 0xa5, sizeof(guarded));
  lines_render_begin(&render, &snapshot);
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 17) == 0 &&
        render.next == 17 && prefix_and_canaries_match(17), "restart begins at zero");
  /* Poison the whole earlier prefix so rewriting any rendered pixel pair
   * with its original value cannot pass unnoticed. */
  for (i = 0; i < 17u; ++i) {
    guarded[i + 1u] = (uint16_t)(expected[i] ^ 0xffffu);
  }
  result = lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, (size_t)-1);
  prefix_untouched = 1;
  for (i = 0; i < 17u; ++i) {
    if (guarded[i + 1u] != (uint16_t)(expected[i] ^ 0xffffu)) {
      prefix_untouched = 0;
    }
  }
  check(result == 0 && render.next == 17 + LINES_BATCH_HALFWORDS &&
        prefix_untouched,
        "capped batch leaves the earlier prefix untouched");
  for (i = 0; i < 17u; ++i) {
    guarded[i + 1u] = expected[i];
  }
  check(result == 0 &&
        render.next == 17 + LINES_BATCH_HALFWORDS &&
        prefix_and_canaries_match(17 + LINES_BATCH_HALFWORDS),
        "capped batch writes the next span and leaves the suffix untouched");
  for (i = 0; i < LINES_PAGE_HALFWORDS / LINES_BATCH_HALFWORDS; ++i) {
    if (render.next >= LINES_PAGE_HALFWORDS ||
        LINES_PAGE_HALFWORDS - render.next <= LINES_BATCH_HALFWORDS) {
      break;
    }
    previous = render.next;
    result = lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, LINES_BATCH_HALFWORDS);
    check(result == 0 && render.next == previous + LINES_BATCH_HALFWORDS &&
          prefix_and_canaries_match(previous + LINES_BATCH_HALFWORDS),
          "intermediate batch writes exactly its prefix");
    if (result != 0 || render.next != previous + LINES_BATCH_HALFWORDS) {
      break;
    }
  }
  check(render.next == LINES_PAGE_HALFWORDS - 223u && prefix_and_canaries_match(render.next),
        "last partial batch starts at the expected progress");
  result = lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, LINES_BATCH_HALFWORDS);
  check(result == 1 && render.next == LINES_PAGE_HALFWORDS &&
        prefix_and_canaries_match(LINES_PAGE_HALFWORDS), "final partial batch completes the page");
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 0) == 1 &&
        render.next == LINES_PAGE_HALFWORDS && prefix_and_canaries_match(LINES_PAGE_HALFWORDS),
        "completed render stays complete on repeated calls");
  check(lines_render_batch(&render, NULL, LINES_PAGE_HALFWORDS, 1) == -1 &&
        lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS - 1u, 1) == -1 &&
        prefix_and_canaries_match(LINES_PAGE_HALFWORDS), "completed render still validates destination");
  lines_render_cancel(&render);
  check(lines_render_batch(&render, guarded + 1, LINES_PAGE_HALFWORDS, 1) == 1 &&
        prefix_and_canaries_match(LINES_PAGE_HALFWORDS), "cancelling a complete render preserves completion");
}

int main(void) {
  samples();
  regular_phrases(96);
  regular_phrases(120);
  regular_phrases(126);
  rapid_phrases();
  phase_wrap();
  phase_boundaries();
  rendering();
  batch_contract();
  printf("gba lines tests: %u checks, %u failures\n", checks, failures);
  return failures ? 1 : 0;
}
