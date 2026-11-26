/* Host-only GX calibration shim. Never linked into the SMS ROM. */
#include "sms_port_trace.h"

typedef void (*poll_observer)(unsigned int, unsigned int);

static read_port original;
static poll_observer observer;

void sms_hook_configure(read_port read, poll_observer observe) {
  original = read;
  observer = observe;
}

unsigned char sms_hook_read(unsigned int port) {
  unsigned char value = original(port);
  if ((port & 255u) == 0xdcu) observer(port, value);
  return value;
}
