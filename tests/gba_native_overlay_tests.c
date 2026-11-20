#include "native_overlay.h"

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

static void expect_sequence(const uint16_t *samples, unsigned n, const uint16_t *expected, unsigned m) {
  gba_native_overlay_t overlay;
  unsigned i;
  gba_native_overlay_init(&overlay);
  for (i = 0; i < n; ++i) {
    gba_native_overlay_sample(&overlay, samples[i]);
  }
  for (i = 0; i < m; ++i) {
    require(gba_native_overlay_next(&overlay) == expected[i], "sampled states retain order");
  }
  require(gba_native_overlay_next(&overlay) == samples[n - 1], "latest held state follows queued transitions");
}

static void transitions(void) {
  static const uint16_t sequential[] = {1, 1, 0, 2, 2, 0};
  static const uint16_t sequential_expected[] = {1, 0, 2, 0};
  static const uint16_t simultaneous[] = {3, 3, 0};
  static const uint16_t simultaneous_expected[] = {3, 0};
  static const uint16_t overlap[] = {1, 3, 2, 0};
  static const uint16_t overlap_expected[] = {1, 3, 2, 0};
  static const uint16_t repeated[] = {1, 0, 1, 0};
  static const uint16_t repeated_expected[] = {1, 0, 1, 0};
  static const uint16_t all_controls[] = {4, 0, 8, 0, 12, 0};
  static const uint16_t all_expected[] = {4, 0, 8, 0, 12, 0};
  expect_sequence(sequential, 6, sequential_expected, 4);
  expect_sequence(simultaneous, 3, simultaneous_expected, 2);
  expect_sequence(overlap, 4, overlap_expected, 4);
  expect_sequence(repeated, 4, repeated_expected, 4);
  expect_sequence(all_controls, 6, all_expected, 6);
}

static void held_and_reset(void) {
  gba_native_overlay_t overlay;
  gba_native_overlay_init(&overlay);
  gba_native_overlay_sample(&overlay, 0x0201u);
  require(gba_native_overlay_next(&overlay) == 1, "energy shoulder is not an overlay");
  require(gba_native_overlay_next(&overlay) == 1, "held overlay remains held");
  gba_native_overlay_sample(&overlay, 0x0200u);
  require(gba_native_overlay_next(&overlay) == 0, "release clears held overlay");
  gba_native_overlay_sample(&overlay, 1u);
  gba_native_overlay_init(&overlay); /* field entry discards native backlog */
  gba_native_overlay_sample(&overlay, 2u); /* first poll during field exit */
  gba_native_overlay_sample(&overlay, 0u);
  require(gba_native_overlay_next(&overlay) == 2, "exit redraw records a fresh press");
  require(gba_native_overlay_next(&overlay) == 0, "exit redraw records its release");
  require(gba_native_overlay_next(&overlay) == 0, "pre-field press is not replayed");
}

static void overflow(void) {
  gba_native_overlay_t overlay;
  unsigned i;
  gba_native_overlay_init(&overlay);
  for (i = 0; i < 40u; ++i) {
    gba_native_overlay_sample(&overlay, (i & 1u) ? 0u : 1u);
  }
  require(overlay.count == GBA_NATIVE_OVERLAY_QUEUE, "overflow is bounded");
  for (i = 0; i < GBA_NATIVE_OVERLAY_QUEUE; ++i) {
    require(gba_native_overlay_next(&overlay) == ((i & 1u) ? 0u : 1u), "overflow preserves old ordering");
  }
  require(gba_native_overlay_next(&overlay) == 0u, "dropped release cannot leave input stuck");
}

int main(void) {
  transitions();
  held_and_reset();
  overflow();
  printf("gba native overlay tests: %u checks, %u failures\n", checks, failures);
  return failures ? EXIT_FAILURE : EXIT_SUCCESS;
}
