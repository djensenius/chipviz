/* Fake core for the host-only trace tests; deliberately not an emulator. */
#include <stdint.h>

struct fake_z80 { uint32_t padding[16], cycles; } Z80;
uint32_t z80_cycle_ratio = 1u << 20;
static unsigned int log_kind[64], log_port[64], log_value[64], count;

unsigned int z80_ms_port_r(unsigned int port) {
  log_kind[count] = 0;
  log_port[count] = port;
  log_value[count++] = port ^ 0x55u;
  return port ^ 0x55u;
}

void z80_ms_port_w(unsigned int port, unsigned int value) {
  log_kind[count] = 1;
  log_port[count] = port;
  log_value[count++] = value;
}

static unsigned int other_read(unsigned int port) { return port; }
unsigned int (*z80_readport)(unsigned int) = z80_ms_port_r;
void (*z80_writeport)(unsigned int, unsigned int) = z80_ms_port_w;

void fake_reset(void) {
  count = 0;
  Z80.cycles = 0;
  z80_cycle_ratio = 1u << 20;
  z80_readport = z80_ms_port_r;
  z80_writeport = z80_ms_port_w;
}
void fake_replace_read(void) { z80_readport = other_read; }
void fake_cycle(uint32_t value) { Z80.cycles = value; }
unsigned int fake_read(unsigned int port) { return z80_readport(port); }
void fake_write(unsigned int port, unsigned int value) { z80_writeport(port, value); }
unsigned int fake_count(void) { return count; }
unsigned int fake_kind(unsigned int i) { return log_kind[i]; }
unsigned int fake_port(unsigned int i) { return log_port[i]; }
unsigned int fake_value(unsigned int i) { return log_value[i]; }
