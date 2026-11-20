/* Host-only GX calibration shim. Never linked into the SMS ROM. */
#include <stdint.h>

typedef unsigned int (*port_read)(unsigned int);
typedef void (*poll_observer)(unsigned int, unsigned int);

static port_read original;
static poll_observer observer;

void sms_hook_configure(port_read read, poll_observer observe) {
  original = read;
  observer = observe;
}

unsigned int sms_hook_read(unsigned int port) {
  unsigned int value = original(port);
  if ((port & 255u) == 0xdcu) observer(port, value);
  return value;
}
