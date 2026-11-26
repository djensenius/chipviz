/* Standalone test PRG only. Never linked into the production program. */
#include "spectrum_volume.h"

#include <stdint.h>

#define REG(address) (*(volatile uint8_t *)(address))
#define SCREEN ((volatile uint8_t *)0x0400)
#define COLOUR ((volatile uint8_t *)0xd800)

/* SV D 1, with the last byte published only after all display writes.
 * The blank build uses SV B 1. Numeric bytes avoid PETSCII conversion. */
volatile uint8_t sv_display_ready[4];
static sv_state_t state;
#ifndef SV_DISPLAY_BLANK
static uint8_t bands[SV_BANDS];
#endif

int main(void) {
  uint16_t i;
  __asm__("sei");
  for (i = 0u; i < 4u; ++i) {
    sv_display_ready[i] = 0u;
  }
  /* I/O visible, bank 0, no sprites, standard 40x25 text. */
  REG(0x0001) |= 7u;
  REG(0xdc0d) = 0x7fu;
  REG(0xdd0d) = 0x7fu;
  REG(0xd01a) = 0u;
  REG(0xdd02) |= 3u;
  REG(0xdd00) = (uint8_t)((REG(0xdd00) & 0xfcu) | 3u);
  REG(0xd015) = 0u;
  REG(0xd011) = 0x0bu; /* display disabled while preparing */
  REG(0xd016) = 0x08u;
  REG(0xd018) = 0x14u;
  REG(0xd020) = 0u;
  REG(0xd021) = 0u;
  for (i = 0u; i < SV_CELLS; ++i) {
    SCREEN[i] = 32u;
    COLOUR[i] = 0u;
  }
  sv_init(&state, 0u);
#ifndef SV_DISPLAY_BLANK
  bands[0] = 255u;
  sv_step(&state, bands, 255u, 0u, SCREEN, COLOUR);
#endif
  REG(0xd011) = 0x1bu;
  sv_display_ready[0] = 0x53u;
  sv_display_ready[1] = 0x56u;
#ifdef SV_DISPLAY_BLANK
  sv_display_ready[2] = 0x42u;
#else
  sv_display_ready[2] = 0x44u;
#endif
  sv_display_ready[3] = 0x31u;
  for (;;) {
  }
  return 0;
}
