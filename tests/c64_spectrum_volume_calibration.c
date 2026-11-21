/* Standalone test PRG; never linked into the production program. */
#include <stdint.h>

#define CELLS 1000u

/* Character ROM code 32 is blank; code 160 is solid reverse space.
 * The four corners use VIC colours white, purple, green and blue. */
void sv_calibration_prepare(volatile uint8_t *screen, volatile uint8_t *colour) {
  uint16_t i;
  for (i = 0u; i < CELLS; ++i) {
    screen[i] = 32u;
    colour[i] = 0u;
  }
  screen[0] = 160u;
  colour[0] = 1u;
  screen[39] = 160u;
  colour[39] = 4u;
  screen[960] = 160u;
  colour[960] = 5u;
  screen[999] = 160u;
  colour[999] = 6u;
}

#ifndef SV_CALIBRATION_HOST_TEST
#define REG(address) (*(volatile uint8_t *)(address))
#define SCREEN ((volatile uint8_t *)0x0400)
#define COLOUR ((volatile uint8_t *)0xd800)

/* Numeric SVC1; the last byte is written only after the display is ready. */
volatile uint8_t sv_calibration_ready[4];

int main(void) {
  uint8_t i;
  __asm__("sei");
  for (i = 0u; i < 4u; ++i) {
    sv_calibration_ready[i] = 0u;
  }
  REG(0x0001) |= 7u;
  REG(0xdc0d) = 0x7fu;
  REG(0xdd0d) = 0x7fu;
  REG(0xd01a) = 0u;
  REG(0xdd02) |= 3u;
  REG(0xdd00) = (uint8_t)((REG(0xdd00) & 0xfcu) | 3u);
  REG(0xd015) = 0u;
  REG(0xd011) = 0x0bu;
  REG(0xd016) = 0x08u;
  REG(0xd018) = 0x14u;
  REG(0xd020) = 0u;
  REG(0xd021) = 0u;
  sv_calibration_prepare(SCREEN, COLOUR);
  REG(0xd011) = 0x1bu;
  sv_calibration_ready[0] = 0x53u;
  sv_calibration_ready[1] = 0x56u;
  sv_calibration_ready[2] = 0x43u;
  sv_calibration_ready[3] = 0x31u;
  for (;;) {
  }
  return 0;
}
#endif
