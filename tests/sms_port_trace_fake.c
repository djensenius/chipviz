/* Fake core for the host-only trace tests; deliberately not an emulator. */
#include "sms_port_trace.h"
#include <limits.h>

struct fake_z80 { uint32_t padding[16], cycles; } Z80;
uint32_t z80_cycle_ratio = 1u << 20;
static unsigned int log_kind[64], log_port[64], log_value[64];
static uint64_t count;

static void record(unsigned int kind, unsigned int port, unsigned int value) {
  if (count < 64) {
    log_kind[count] = kind;
    log_port[count] = port;
    log_value[count] = value;
  }
  count++;
}

unsigned char z80_ms_port_r(unsigned int port) {
  unsigned char value = (unsigned char)(port ^ 0x55u);
  record(0, port, value);
  return value;
}

void z80_ms_port_w(unsigned int port, unsigned char value) {
  record(1, port, value);
}

static unsigned char other_read(unsigned int port) { return (unsigned char)port; }
read_port z80_readport = z80_ms_port_r;
write_port z80_writeport = z80_ms_port_w;

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
void fake_write(unsigned int port, unsigned int value) { z80_writeport(port, (unsigned char)value); }
uint64_t fake_count(void) { return count; }
unsigned int fake_kind(unsigned int i) { return i < 64 && i < count ? log_kind[i] : UINT_MAX; }
unsigned int fake_port(unsigned int i) { return i < 64 && i < count ? log_port[i] : UINT_MAX; }
unsigned int fake_value(unsigned int i) { return i < 64 && i < count ? log_value[i] : UINT_MAX; }
