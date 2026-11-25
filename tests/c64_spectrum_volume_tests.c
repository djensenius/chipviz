#include "spectrum_volume.h"

#include <stdio.h>
#include <string.h>

static unsigned checks;
static unsigned failures;

static void require(int condition, const char *message) {
  ++checks;
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    ++failures;
  }
}

typedef struct {
  uint8_t before[16];
  uint8_t cells[SV_CELLS];
  uint8_t after[16];
} surface_t;

static void setup(surface_t *screen, surface_t *colour) {
  memset(screen, 0xa5, sizeof(*screen));
  memset(colour, 0x5a, sizeof(*colour));
  memset(screen->cells, 32, sizeof(screen->cells));
  memset(colour->cells, 0, sizeof(colour->cells));
}

static unsigned count(const uint8_t *cells, uint8_t value) {
  unsigned i;
  unsigned n = 0u;
  for (i = 0u; i < SV_CELLS; ++i) {
    n += cells[i] == value;
  }
  return n;
}

static void check_image(const sv_state_t *state, const surface_t *screen, const surface_t *colour) {
  static const unsigned base[] = {8u, 15u, 22u};
  static const unsigned shift[] = {2u, 1u, 0u};
  static const unsigned width[] = {1u, 1u, 2u};
  static const uint8_t inks[] = {6u, 4u, 5u, 9u, 2u, 11u, 8u, 12u};
  uint8_t expected[SV_CELLS], expected_ink[SV_CELLS];
  unsigned band, slice, row, x, i;

  memset(expected, 32, sizeof(expected));
  memset(expected_ink, 0, sizeof(expected_ink));
  for (band = 0u; band < SV_BANDS; ++band) {
    for (slice = 0u; slice < 3u; ++slice) {
      for (row = 0u; row < state->height[band]; ++row) {
        for (x = 0u; x < width[slice]; ++x) {
          unsigned pos = (base[slice] - row) * 40u + 4u + band * 4u + shift[slice] + x;
          expected[pos] = 160u;
          expected_ink[pos] = inks[(state->palette[band] + band + slice) & 7u];
        }
      }
    }
  }
  require(memcmp(expected, screen->cells, SV_CELLS) == 0, "complete slice image matches state");
  for (i = 0u; i < SV_CELLS; ++i) {
    if (expected[i] == 160u) {
      require(colour->cells[i] == expected_ink[i], "visible ink matches slice palette");
    }
  }
}

static void single_bands(void) {
  surface_t screen, colour;
  sv_state_t state;
  uint8_t bands[SV_BANDS];
  unsigned band, step, slice, row, x;
  static const unsigned base[] = {8u, 15u, 22u};
  static const unsigned shift[] = {2u, 1u, 0u};
  static const unsigned width[] = {1u, 1u, 2u};

  for (band = 0u; band < SV_BANDS; ++band) {
    setup(&screen, &colour);
    memset(bands, 0, sizeof(bands));
    sv_init(&state, 0u);
    bands[band] = 255u;
    for (step = 0u; step < 5u; ++step) {
      sv_step(&state, bands, 255u, 0u, screen.cells, colour.cells);
      require(state.height[band] == step + 1u, "height rises one cell per step");
      require(count(screen.cells, 160u) == (step + 1u) * 4u, "only selected band rises");
    }
    for (slice = 0u; slice < 3u; ++slice) {
      for (row = 0u; row < 5u; ++row) {
        for (x = 0u; x < width[slice]; ++x) {
          unsigned pos = (base[slice] - row) * 40u + 4u + band * 4u + shift[slice] + x;
          require(screen.cells[pos] == 160u, "staggered slice geometry");
          require(colour.cells[pos] > 1u && colour.cells[pos] < 16u, "valid nonwhite VIC ink");
        }
      }
    }
    bands[band] = 0u;
    for (step = 0u; step < 5u; ++step) {
      sv_step(&state, bands, 255u, 0u, screen.cells, colour.cells);
      require(state.height[band] == 4u - step, "height falls one cell per step");
    }
    require(count(screen.cells, 32u) == SV_CELLS, "silence clears every bar");
  }
}

static void transitions(void) {
  surface_t screen, colour, screen2, colour2;
  sv_state_t state, state2;
  uint8_t bands[SV_BANDS];
  uint8_t previous_screen[SV_CELLS], previous_colour[SV_CELLS];
  unsigned step, i, screen_changes, colour_changes;

  setup(&screen, &colour);
  setup(&screen2, &colour2);
  sv_init(&state, 0u);
  sv_init(&state2, 8u);
  memset(bands, 255, sizeof(bands));
  for (step = 0u; step < 200u; ++step) {
    uint8_t energy = (uint8_t)(step & 1u ? 255u : 0u);
    uint8_t palette = (uint8_t)(step * 17u);
    for (i = 0u; i < SV_BANDS; ++i) {
      bands[i] = (uint8_t)(step * 29u + i * 47u);
    }
    memcpy(previous_screen, screen.cells, SV_CELLS);
    memcpy(previous_colour, colour.cells, SV_CELLS);
    sv_step(&state, bands, energy, palette, screen.cells, colour.cells);
    sv_step(&state2, bands, energy, palette, screen2.cells, colour2.cells);
    check_image(&state, &screen, &colour);
    require(memcmp(&state, &state2, sizeof(state)) == 0, "deterministic state and palette wrap");
    require(memcmp(&screen, &screen2, sizeof(screen)) == 0, "deterministic screen");
    require(memcmp(&colour, &colour2, sizeof(colour)) == 0, "deterministic colour RAM");
    screen_changes = colour_changes = 0u;
    for (i = 0u; i < SV_CELLS; ++i) {
      screen_changes += screen.cells[i] != previous_screen[i];
      colour_changes += colour.cells[i] != previous_colour[i];
      require(screen.cells[i] == 32u || screen.cells[i] == 160u, "only blank or graphics bar");
    }
    require(screen_changes <= 32u, "at most 32 screen cells change per step");
    require(colour_changes <= 52u, "bounded palette and growth updates");
    for (i = 0u; i < 16u; ++i) {
      require(screen.before[i] == 0xa5u && screen.after[i] == 0xa5u, "screen guards intact");
      require(colour.before[i] == 0x5au && colour.after[i] == 0x5au, "colour guards intact");
    }
  }

  memset(bands, 255, sizeof(bands));
  for (step = 0u; step < 16u; ++step) {
    sv_step(&state, bands, 255u, 3u, screen.cells, colour.cells);
  }
  require(count(screen.cells, 160u) == 160u, "full spectrum fills all slices");
  for (i = 0u; i < SV_BANDS; ++i) {
    require(state.height[i] == 5u && state.palette[i] == 3u, "stable input and palette settle");
  }
  /* At palette 3, band 0 uses indices 9, 2 and 11 in the three slices. */
  require(colour.cells[8u * 40u + 6u] == 9u &&
          colour.cells[15u * 40u + 5u] == 2u &&
          colour.cells[22u * 40u + 4u] == 11u &&
          colour.cells[22u * 40u + 5u] == 11u, "palette changes reach every slice");
  for (step = 0u; step < 5u; ++step) {
    sv_step(&state, bands, 0u, 3u, screen.cells, colour.cells);
  }
  require(state.height[0] == 3u, "low energy reduces nonzero band height");
  memset(bands, 0, sizeof(bands));
  for (step = 0u; step < 5u; ++step) {
    sv_step(&state, bands, 255u, 3u, screen.cells, colour.cells);
  }
  require(count(screen.cells, 32u) == SV_CELLS, "transition to silence clears all slices");

  setup(&screen, &colour);
  sv_init(&state, 255u);
  require(state.next_palette == 0u, "reset restarts palette cursor");
  for (i = 0u; i < SV_BANDS; ++i) {
    require(state.height[i] == 0u && state.palette[i] == 7u, "reset resets all bands");
  }
}

static void thresholds(void) {
  surface_t screen, colour;
  sv_state_t state;
  uint8_t bands[SV_BANDS] = {0u, 31u, 32u, 63u, 64u, 159u, 160u, 255u};
  static const uint8_t high[] = {0u, 0u, 1u, 1u, 2u, 4u, 5u, 5u};
  static const uint8_t low[] = {0u, 0u, 0u, 0u, 1u, 2u, 2u, 3u};
  unsigned step, band;

  setup(&screen, &colour);
  sv_init(&state, 0u);
  for (step = 0u; step < 5u; ++step) {
    sv_step(&state, bands, 64u, 0u, screen.cells, colour.cells);
  }
  for (band = 0u; band < SV_BANDS; ++band) {
    require(state.height[band] == high[band], "band quantization at energy threshold");
  }
  for (step = 0u; step < 5u; ++step) {
    sv_step(&state, bands, 63u, 0u, screen.cells, colour.cells);
  }
  for (band = 0u; band < SV_BANDS; ++band) {
    require(state.height[band] == low[band], "band quantization below energy threshold");
  }
}

int main(void) {
  single_bands();
  transitions();
  thresholds();
  if (failures) {
    fprintf(stderr, "c64 spectrum volume: %u of %u checks failed\n", failures, checks);
    return 1;
  }
  printf("c64 spectrum volume: %u checks passed\n", checks);
  return 0;
}
