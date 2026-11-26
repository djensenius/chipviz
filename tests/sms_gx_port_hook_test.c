/* Host-only cross-hook test using the synthetic core, not an emulator. */
#include <assert.h>
#include "sms_port_trace.h"
#include "sms_gx_port_hook.c"

_Static_assert(_Generic(&sms_hook_read, read_port: 1, default: 0),
               "poll hook must match the read slot ABI");
_Static_assert(_Generic(&sms_hook_configure, void (*)(read_port, poll_observer): 1, default: 0),
               "poll hook must accept the read slot ABI");

struct fake_z80 { uint32_t padding[16], cycles; };
extern struct fake_z80 Z80;
extern uint32_t z80_cycle_ratio;
extern read_port z80_readport;
extern write_port z80_writeport;
void z80_ms_port_w(unsigned int port, unsigned char value);
void fake_reset(void);
void fake_cycle(uint32_t value);
unsigned int fake_read(unsigned int port);
uint64_t fake_count(void);

static unsigned int observations, observed_port, observed_value;

static void observe_poll(unsigned int port, unsigned int value) {
  observations++;
  observed_port = port;
  observed_value = value;
}

int main(void) {
  char owner = 0;
  read_port original;
  struct event event;
  size_t count = 0;
  fake_reset();
  original = z80_readport;
  sms_hook_configure(original, observe_poll);
  z80_readport = sms_hook_read;
  assert(sms_trace_install(&z80_readport, &z80_writeport, sms_hook_read, z80_ms_port_w,
                           &Z80.cycles, &z80_cycle_ratio, 8, &owner) == 0);
  assert(sms_trace_begin(&owner, &z80_readport, &z80_writeport, 7) == 0);
  fake_cycle(42);
  assert(fake_read(0x1dcu) == ((0x1dcu ^ 0x55u) & 0xffu));
  fake_cycle(100);
  assert(fake_read(0x13fu) == ((0x13fu ^ 0x55u) & 0xffu));
  assert(sms_trace_end(&owner, &z80_readport, &z80_writeport) == 0);
  assert(fake_count() == 2 && observations == 1);
  assert(observed_port == 0x1dcu && observed_value == ((0x1dcu ^ 0x55u) & 0xffu));
  assert(sms_trace_count(&owner, &z80_readport, &z80_writeport, &count) == 0 && count == 2);
  assert(sms_trace_event(&owner, &z80_readport, &z80_writeport, 0, &event) == 0);
  assert(event.frame == 7 && event.cycle == 42 && event.port == 0x1dcu &&
         event.value == observed_value && event.kind == 0);
  assert(sms_trace_event(&owner, &z80_readport, &z80_writeport, 1, &event) == 0);
  assert(event.frame == 7 && event.cycle == 100 && event.port == 0x13fu &&
         event.value == ((0x13fu ^ 0x55u) & 0xffu) && event.kind == 0);
  assert(sms_trace_uninstall(&owner, &z80_readport, &z80_writeport) == 0);
  assert(z80_readport == sms_hook_read);
  z80_readport = original;
  assert(fake_read(0x1dcu) == ((0x1dcu ^ 0x55u) & 0xffu));
  assert(fake_count() == 3 && observations == 1);
  return 0;
}
