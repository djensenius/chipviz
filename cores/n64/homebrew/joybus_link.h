#ifndef CHIPVIZ_N64_JOYBUS_LINK_H
#define CHIPVIZ_N64_JOYBUS_LINK_H

#include <stdint.h>
#include <string.h>

/* N64 Joybus transport v1 frame marker; see
 * shared/specs/n64-joybus-transport-v0.md. Bytes 1, 5 and 9 (buttons-low for
 * controllers 1..3) carry a 6-bit magic, sequence and checksum. libdragon
 * drops bits 0x80 (reset) and 0x40 (unused) of buttons-low, so only the low six
 * bits are usable and the high two must be zero.
 */
#define JOYBUS_LINK_PACKET_SIZE 16
#define JOYBUS_LINK_MARKER_MASK 0x3Fu
#define JOYBUS_LINK_MAGIC 0x2Du
#define JOYBUS_LINK_CHECKSUM_SEED 0xA5u
/* Marker loss while live: hold the last live packet this many frames. */
#define JOYBUS_LINK_HOLD_FRAMES 45
/* Live tolerates this many consecutive valid repeats of the same sequence (a
 * host slower than the ROM poll) before reporting hold.
 */
#define JOYBUS_LINK_REPEAT_GRACE 2
/* Advancing valid frames needed to leave procedural (no flapping). */
#define JOYBUS_LINK_RECOVER_FRAMES 4

typedef enum {
  JOYBUS_LINK_PROCEDURAL = 0,
  JOYBUS_LINK_HOLD = 1,
  JOYBUS_LINK_LIVE = 2,
} joybus_link_mode_t;

typedef struct {
  int live;
  int has_seq;
  uint8_t last_seq;
  int stale;
  int streak;
  uint8_t held[JOYBUS_LINK_PACKET_SIZE];
} joybus_link_t;

/* Rotate-left XOR over every byte except the checksum byte 9, folded to six
 * bits. Bytes 1 and 5 (magic, sequence) are included.
 */
static inline uint8_t joybus_link_checksum(const uint8_t packet[JOYBUS_LINK_PACKET_SIZE]) {
  unsigned c = JOYBUS_LINK_CHECKSUM_SEED;
  for (int i = 0; i < JOYBUS_LINK_PACKET_SIZE; i++) {
    if (i == 9) {
      continue;
    }
    c = (((c << 1) | (c >> 7)) & 0xFFu) ^ packet[i];
  }
  return (uint8_t)((c ^ (c >> 6)) & JOYBUS_LINK_MARKER_MASK);
}

static inline int joybus_link_marker_valid(const uint8_t packet[JOYBUS_LINK_PACKET_SIZE]) {
  return packet[1] == JOYBUS_LINK_MAGIC &&
         (packet[5] & ~JOYBUS_LINK_MARKER_MASK) == 0 &&
         packet[9] == joybus_link_checksum(packet);
}

static inline uint8_t joybus_link_sequence(const uint8_t packet[JOYBUS_LINK_PACKET_SIZE]) {
  return (uint8_t)(packet[5] & JOYBUS_LINK_MARKER_MASK);
}

static inline void joybus_link_init(joybus_link_t *link) {
  memset(link, 0, sizeof(*link));
}

/* Bit p of `connected_mask` is set when controller port p+1 is connected. The
 * link needs all four ports: an unplugged port reads as zeros and the marker
 * cannot always detect that (zeroed port-4 data can keep the checksum), so a
 * partial disconnect counts as no device.
 */
#define JOYBUS_LINK_ALL_PORTS 0x0Fu

static inline int joybus_link_ports_ready(unsigned connected_mask) {
  return (connected_mask & JOYBUS_LINK_ALL_PORTS) == JOYBUS_LINK_ALL_PORTS;
}

/* Pure per-frame selection: (connected, marker, sequence, packet) -> mode.
 * - No device (connected false; pass joybus_link_ports_ready(mask), so any
 *   missing port counts): procedural immediately, state reset.
 * - Live: a valid marker whose sequence differs from the previous valid one
 *   keeps live (even if every data byte is zero) and refreshes the held packet.
 *   Up to JOYBUS_LINK_REPEAT_GRACE consecutive valid repeats stay live. Anything
 *   else (longer repeats, invalid marker) holds the last live packet. After
 *   JOYBUS_LINK_HOLD_FRAMES frames without an advance it falls back to
 *   procedural.
 * - Procedural: an unmarked (ordinary) pad never goes live. Live resumes after
 *   JOYBUS_LINK_RECOVER_FRAMES advancing valid frames; an invalid frame resets
 *   that count, a valid repeat neither advances nor resets it.
 * On JOYBUS_LINK_LIVE use `packet`; on JOYBUS_LINK_HOLD use `link->held`.
 */
static inline joybus_link_mode_t joybus_link_step(joybus_link_t *link, int connected,
                                                  const uint8_t packet[JOYBUS_LINK_PACKET_SIZE]) {
  if (!connected) {
    joybus_link_init(link);
    return JOYBUS_LINK_PROCEDURAL;
  }

  int valid = joybus_link_marker_valid(packet);
  uint8_t seq = joybus_link_sequence(packet);
  int advancing = valid && link->has_seq && seq != link->last_seq;
  int repeat = valid && link->has_seq && seq == link->last_seq;
  link->has_seq = valid;
  link->last_seq = valid ? seq : 0;

  if (link->live) {
    if (advancing) {
      link->stale = 0;
      memcpy(link->held, packet, JOYBUS_LINK_PACKET_SIZE);
      return JOYBUS_LINK_LIVE;
    }
    link->stale++;
    if (link->stale >= JOYBUS_LINK_HOLD_FRAMES) {
      link->live = 0;
      link->stale = 0;
      link->streak = 0;
      return JOYBUS_LINK_PROCEDURAL;
    }
    if (repeat && link->stale <= JOYBUS_LINK_REPEAT_GRACE) {
      return JOYBUS_LINK_LIVE;
    }
    return JOYBUS_LINK_HOLD;
  }

  if (advancing) {
    link->streak++;
  } else if (!valid) {
    link->streak = 0;
  }
  if (link->streak >= JOYBUS_LINK_RECOVER_FRAMES) {
    link->live = 1;
    link->stale = 0;
    link->streak = 0;
    memcpy(link->held, packet, JOYBUS_LINK_PACKET_SIZE);
    return JOYBUS_LINK_LIVE;
  }
  return JOYBUS_LINK_PROCEDURAL;
}

#endif
