/* Standalone fake-core stress check for ASan/UBSan; no emulator involved. */
#include <assert.h>
#include <stddef.h>
#include <stdint.h>

typedef unsigned int (*read_port)(unsigned int);
typedef void (*write_port)(unsigned int, unsigned int);
struct fake_z80 { uint32_t padding[16], cycles; };
struct event { uint32_t frame, cycle, port, value, kind; };

extern struct fake_z80 Z80;
extern uint32_t z80_cycle_ratio;
extern read_port z80_readport;
extern write_port z80_writeport;
unsigned int z80_ms_port_r(unsigned int port);
void z80_ms_port_w(unsigned int port, unsigned int value);
void fake_reset(void);
unsigned int fake_read(unsigned int port);
void fake_write(unsigned int port, unsigned int value);
uint64_t fake_count(void);
unsigned int fake_value(unsigned int index);
int sms_trace_install(read_port *, write_port *, read_port, write_port,
                      const uint32_t *, const uint32_t *, size_t, const void *);
int sms_trace_begin(uint32_t frame);
int sms_trace_end(void);
size_t sms_trace_count(void);
int sms_trace_event(size_t index, struct event *out);
int sms_trace_owned(const void *, read_port *, write_port *);
int sms_trace_uninstall(const void *, read_port *, write_port *);

int main(void) {
  char owner = 0, other = 0;
  struct event event;
  unsigned int i;
  fake_reset();
  assert(sms_trace_install(&z80_readport, &z80_writeport, z80_ms_port_r, z80_ms_port_w,
                           &Z80.cycles, &z80_cycle_ratio, 4096, &owner) == 0);
  assert(sms_trace_uninstall(&other, &z80_readport, &z80_writeport) != 0);
  assert(sms_trace_owned(&owner, &z80_readport, &z80_writeport));
  assert(sms_trace_begin(0) == 0);
  for (i = 0; i < 4096; i++) fake_write(i, i + 1);
  assert(sms_trace_end() == 0);
  assert(sms_trace_count() == 4096 && fake_count() == 4096);
  assert(sms_trace_event(4095, &event) == 0);
  assert(event.port == 4095 && event.value == 4096);
  assert(fake_value(63) == 64);
  assert(sms_trace_begin(1) == 0);
  assert(fake_read(7) == (7u ^ 0x55u));
  assert(sms_trace_end() != 0 && fake_count() == 4097);
  assert(sms_trace_uninstall(&owner, &z80_readport, &z80_writeport) == 0);
  assert(z80_readport == z80_ms_port_r && z80_writeport == z80_ms_port_w);
  return 0;
}
