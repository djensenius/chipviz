/* Standalone fake-core stress check for ASan/UBSan; no emulator involved. */
#include <assert.h>
#include "sms_port_trace.h"

_Static_assert(_Generic((read_port)0, unsigned char (*)(unsigned int): 1, default: 0),
               "unexpected read slot ABI");
_Static_assert(_Generic((write_port)0, void (*)(unsigned int, unsigned char): 1, default: 0),
               "unexpected write slot ABI");

struct fake_z80 { uint32_t padding[16], cycles; };

extern struct fake_z80 Z80;
extern uint32_t z80_cycle_ratio;
extern read_port z80_readport;
extern write_port z80_writeport;
unsigned char z80_ms_port_r(unsigned int port);
void z80_ms_port_w(unsigned int port, unsigned char value);
void fake_reset(void);
unsigned int fake_read(unsigned int port);
void fake_write(unsigned int port, unsigned int value);
uint64_t fake_count(void);
unsigned int fake_value(unsigned int index);
int main(void) {
  char owner = 0, other = 0;
  read_port wrong_slot = z80_ms_port_r;
  struct event event;
  size_t count = 999;
  unsigned int i;
  fake_reset();
  assert(sms_trace_install(&z80_readport, &z80_writeport, z80_ms_port_r, z80_ms_port_w,
                           &Z80.cycles, &z80_cycle_ratio, 4096, &owner) == 0);
  assert(sms_trace_uninstall(&other, &z80_readport, &z80_writeport) != 0);
  assert(sms_trace_owned(&owner, &z80_readport, &z80_writeport));
  assert(!sms_trace_healthy(&other, &z80_readport, &z80_writeport));
  assert(!sms_trace_healthy(&owner, &wrong_slot, &z80_writeport));
  assert(sms_trace_begin(&owner, &wrong_slot, &z80_writeport, 0) != 0);
  assert(sms_trace_count(&owner, &wrong_slot, &z80_writeport, &count) != 0 && count == 999);
  assert(sms_trace_lost(&owner, &wrong_slot, &z80_writeport) == -1);
  assert(sms_trace_event(&owner, &wrong_slot, &z80_writeport, 0, &event) != 0);
  assert(sms_trace_begin(&other, &z80_readport, &z80_writeport, 0) != 0);
  assert(sms_trace_count(&other, &z80_readport, &z80_writeport, &count) != 0 && count == 999);
  assert(sms_trace_lost(&other, &z80_readport, &z80_writeport) == -1);
  assert(sms_trace_event(&other, &z80_readport, &z80_writeport, 0, &event) != 0);
  assert(sms_trace_begin(&owner, &z80_readport, &z80_writeport, 0) == 0);
  for (i = 0; i < 4096; i++) fake_write(i, i + 1);
  assert(sms_trace_end(&owner, &wrong_slot, &z80_writeport) != 0);
  assert(sms_trace_end(&other, &z80_readport, &z80_writeport) != 0);
  assert(sms_trace_end(&owner, &z80_readport, &z80_writeport) == 0);
  assert(sms_trace_count(&owner, &z80_readport, &z80_writeport, &count) == 0);
  assert(count == 4096 && fake_count() == 4096);
  assert(sms_trace_event(&owner, &z80_readport, &z80_writeport, 4095, &event) == 0);
  assert(event.port == 4095 && event.value == 0);
  assert(fake_value(63) == 64);
  assert(sms_trace_begin(&owner, &z80_readport, &z80_writeport, 1) == 0);
  assert(fake_read(0x12ff) == 0xaau);
  assert(sms_trace_end(&owner, &z80_readport, &z80_writeport) != 0 && fake_count() == 4097);
  assert(sms_trace_lost(&owner, &z80_readport, &z80_writeport) == 1);
  assert(sms_trace_count(&owner, &z80_readport, &z80_writeport, &count) != 0);
  assert(sms_trace_event(&owner, &z80_readport, &z80_writeport, 0, &event) != 0);
  assert(sms_trace_uninstall(&owner, &z80_readport, &z80_writeport) == 0);
  assert(z80_readport == z80_ms_port_r && z80_writeport == z80_ms_port_w);
  return 0;
}
