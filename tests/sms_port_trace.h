/* Host-only shim ABI; port signatures match the GX Z80 slot declarations. */
#ifndef SMS_PORT_TRACE_H
#define SMS_PORT_TRACE_H

#include <stddef.h>
#include <stdint.h>

typedef unsigned char (*read_port)(unsigned int);
typedef void (*write_port)(unsigned int, unsigned char);

struct event {
  uint32_t frame, cycle, port, value, kind;
};

int sms_trace_install(read_port *, write_port *, read_port, write_port,
                      const uint32_t *, const uint32_t *, size_t, const void *);
int sms_trace_begin(const void *, read_port *, write_port *, uint32_t frame);
int sms_trace_end(const void *, read_port *, write_port *);
int sms_trace_count(const void *, read_port *, write_port *, size_t *out);
int sms_trace_lost(const void *, read_port *, write_port *);
int sms_trace_healthy(const void *, read_port *, write_port *);
int sms_trace_event(const void *, read_port *, write_port *, size_t index, struct event *out);
int sms_trace_owned(const void *, read_port *, write_port *);
int sms_trace_uninstall(const void *, read_port *, write_port *);

#endif
