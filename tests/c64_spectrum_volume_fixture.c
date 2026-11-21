/* Test-only PRG: exercise the real scene on guarded ordinary RAM, not VIC
 * screen/colour RAM. The VICE frontend reads the bounded trace after done.
 * This is not linked into the production program. */
#include "spectrum_volume.h"

#include <stdint.h>
#include <string.h>

#define TRACE_STEPS 112u
#define TRACE_RECORD 26u
#define GUARD_SIZE 16u

typedef struct {
  uint8_t before[GUARD_SIZE];
  uint8_t cells[SV_CELLS];
  uint8_t after[GUARD_SIZE];
} guarded_t;

volatile uint8_t sv_fixture_trace[TRACE_STEPS * TRACE_RECORD];
/* Written last, so a reader never mistakes a partial trace for a result. */
volatile uint8_t sv_fixture_done[4];
/* Magic followed by the number of fully recorded steps. Test diagnostics
 * only; the count is published after each complete record. */
volatile uint8_t sv_fixture_progress[5];
static guarded_t screen, colour;
static sv_state_t state;
static uint8_t bands[SV_BANDS];

static void reset(uint8_t palette) {
  memset(&screen, 0xa5, sizeof(screen));
  memset(&colour, 0x5a, sizeof(colour));
  memset(screen.cells, 32, sizeof(screen.cells));
  memset(colour.cells, 0, sizeof(colour.cells));
  sv_init(&state, palette);
}

static uint8_t guards_ok(void) {
  uint8_t i;
  for (i = 0u; i < GUARD_SIZE; ++i) {
    if (screen.before[i] != 0xa5u || screen.after[i] != 0xa5u ||
        colour.before[i] != 0x5au || colour.after[i] != 0x5au) {
      return 0u;
    }
  }
  return 1u;
}

/* Two non-cryptographic 16-bit checksums over every byte, including stale
 * colour cells. These detect many corruptions but do not prove equality. */
static void checksum(const uint8_t *cells, uint16_t *sum, uint16_t *rolling) {
  uint16_t i;
  uint16_t s = 0u;
  uint16_t r = 0u;
  for (i = 0u; i < SV_CELLS; ++i) {
    s = (uint16_t)(s + cells[i]);
    r = (uint16_t)((uint16_t)(r << 1) | (r >> 15));
    r ^= cells[i];
  }
  *sum = s;
  *rolling = r;
}

static void record(uint8_t step) {
  uint8_t i;
  uint16_t sum, rolling;
  uint16_t pos = (uint16_t)step * TRACE_RECORD;
  for (i = 0u; i < SV_BANDS; ++i) {
    sv_fixture_trace[pos++] = state.height[i];
  }
  for (i = 0u; i < SV_BANDS; ++i) {
    sv_fixture_trace[pos++] = state.palette[i];
  }
  sv_fixture_trace[pos++] = state.next_palette;
  checksum(screen.cells, &sum, &rolling);
  sv_fixture_trace[pos++] = (uint8_t)sum;
  sv_fixture_trace[pos++] = (uint8_t)(sum >> 8);
  sv_fixture_trace[pos++] = (uint8_t)rolling;
  sv_fixture_trace[pos++] = (uint8_t)(rolling >> 8);
  checksum(colour.cells, &sum, &rolling);
  sv_fixture_trace[pos++] = (uint8_t)sum;
  sv_fixture_trace[pos++] = (uint8_t)(sum >> 8);
  sv_fixture_trace[pos++] = (uint8_t)rolling;
  sv_fixture_trace[pos++] = (uint8_t)(rolling >> 8);
  sv_fixture_trace[pos] = guards_ok();
}

int main(void) {
  static const uint8_t thresholds[SV_BANDS] = {0u, 31u, 32u, 63u, 64u, 159u, 160u, 255u};
  uint8_t step, band, energy, palette;

  memset((void *)sv_fixture_done, 0, sizeof(sv_fixture_done));
  sv_fixture_progress[4] = 0u;
  sv_fixture_progress[0] = 0x53u;
  sv_fixture_progress[1] = 0x56u;
  sv_fixture_progress[2] = 0x50u;
  sv_fixture_progress[3] = 0x31u;
  for (step = 0u; step < TRACE_STEPS; ++step) {
    memset(bands, 0, sizeof(bands));
    energy = 255u;
    palette = 0u;
    if (step < 80u) {
      band = (uint8_t)(step / 10u);
      if (step % 10u == 0u) {
        reset(0u);
      }
      if (step % 10u < 5u) {
        bands[band] = 255u;
      }
    } else if (step < 90u) {
      if (step == 80u) {
        reset(0u);
      }
      memcpy(bands, thresholds, sizeof(bands));
      energy = step < 85u ? 64u : 63u;
    } else if (step < 111u) {
      palette = 3u;
      if (step < 106u) {
        memset(bands, 255, sizeof(bands));
      }
    } else {
      reset(255u);
      palette = 255u;
    }
    sv_step(&state, bands, energy, palette, screen.cells, colour.cells);
    record(step);
    sv_fixture_progress[4] = (uint8_t)(step + 1u);
  }
  /* Numeric bytes avoid cc65's target-specific character literal mapping. */
  sv_fixture_done[0] = 0x53u;
  sv_fixture_done[1] = 0x56u;
  sv_fixture_done[2] = 0x30u;
  sv_fixture_done[3] = 0x31u;
  for (;;) {
  }
  return 0;
}
