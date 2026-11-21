/* Single-threaded host test shim; not part of a ROM or a libretro core. */
#include "sms_port_trace.h"

enum { CAPACITY = 4096, READ = 0, WRITE = 1 };
static struct event events[CAPACITY];
static size_t count, limit;
static int lost, active, in_frame;
static uint32_t frame;
static const uint32_t *cycles, *ratio;
static read_port *read_slot, original_read;
static write_port *write_slot, original_write;
static const void *owner;

static unsigned char trace_read(unsigned int port);
static void trace_write(unsigned int port, unsigned char value);

static int healthy(void) {
  return active && *read_slot == trace_read && *write_slot == trace_write &&
         *ratio == (1u << 20) && !lost;
}

int sms_trace_healthy(const void *token, read_port *rs, write_port *ws) {
  return sms_trace_owned(token, rs, ws) && healthy();
}

static struct event *reserve(unsigned int port, uint32_t kind) {
  struct event *event;
  if (!in_frame || !healthy() || count == limit) {
    lost = 1;
    return NULL;
  }
  event = &events[count++];
  event->frame = frame;
  event->cycle = *cycles;
  event->port = port;
  event->value = 0;
  event->kind = kind;
  return event;
}

static unsigned char trace_read(unsigned int port) {
  struct event *event = reserve(port, READ);
  unsigned char value = original_read(port);
  if (event != NULL) event->value = value;
  return value;
}

static void trace_write(unsigned int port, unsigned char value) {
  struct event *event = reserve(port, WRITE);
  if (event != NULL) event->value = value;
  original_write(port, value);
}

int sms_trace_install(read_port *rs, write_port *ws, read_port r, write_port w,
                      const uint32_t *c, const uint32_t *q, size_t capacity,
                      const void *token) {
  if (active || rs == NULL || ws == NULL || r == NULL || w == NULL ||
      c == NULL || q == NULL || token == NULL ||
      *q != (1u << 20) || *rs != r || *ws != w ||
      capacity == 0 || capacity > CAPACITY) return -1;
  read_slot = rs;
  write_slot = ws;
  original_read = r;
  original_write = w;
  cycles = c;
  ratio = q;
  owner = token;
  count = 0;
  limit = capacity;
  lost = 0;
  in_frame = 0;
  active = 1;
  *rs = trace_read;
  *ws = trace_write;
  return 0;
}

int sms_trace_begin(const void *token, read_port *rs, write_port *ws, uint32_t number) {
  if (!sms_trace_healthy(token, rs, ws) || in_frame) return -1;
  frame = number;
  in_frame = 1;
  return 0;
}

int sms_trace_end(const void *token, read_port *rs, write_port *ws) {
  if (!sms_trace_owned(token, rs, ws) || !in_frame) return -1;
  in_frame = 0;
  return healthy() ? 0 : -1;
}

int sms_trace_count(const void *token, read_port *rs, write_port *ws, size_t *out) {
  if (!sms_trace_healthy(token, rs, ws) || out == NULL) return -1;
  *out = count;
  return 0;
}
int sms_trace_lost(const void *token, read_port *rs, write_port *ws) {
  return sms_trace_owned(token, rs, ws) ? lost : -1;
}
int sms_trace_event(const void *token, read_port *rs, write_port *ws, size_t index, struct event *out) {
  if (!sms_trace_healthy(token, rs, ws) || index >= count || out == NULL) return -1;
  *out = events[index];
  return 0;
}

int sms_trace_owned(const void *token, read_port *rs, write_port *ws) {
  return active && token != NULL && owner == token && read_slot == rs && write_slot == ws;
}

int sms_trace_uninstall(const void *token, read_port *rs, write_port *ws) {
  int ok;
  if (!sms_trace_owned(token, rs, ws)) return -1;
  ok = *read_slot == trace_read && *write_slot == trace_write;
  /* Do not overwrite another owner's replacement. Restore our own slots. */
  if (*read_slot == trace_read) *read_slot = original_read;
  if (*write_slot == trace_write) *write_slot = original_write;
  active = 0;
  in_frame = 0;
  owner = NULL;
  return ok ? 0 : -1;
}
