#include "joybus_link.h"

#include <stdio.h>
#include <stdlib.h>

static unsigned checks;
static unsigned failures;

static void require(int condition, const char *message) {
  checks++;
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    failures++;
  }
}

/* Same golden vectors as tests/test_transports.py (host encoder output for
 * procedural frames 0 and 63).
 */
static const uint8_t golden0[16] = {0x80, 0x2d, 0x00, 0x1d, 0x00, 0x00, 0x3a, 0x57,
                                    0x02, 0x02, 0x74, 0x91, 0x01, 0x00, 0x50, 0x50};
static const uint8_t golden63[16] = {0x00, 0x2d, 0x2f, 0x4c, 0x02, 0x3f, 0x69, 0x86,
                                     0x03, 0x08, 0xa3, 0xc0, 0x40, 0x01, 0x67, 0x00};

static void marked(uint8_t packet[16], uint8_t seq, uint8_t fill) {
  for (int i = 0; i < 16; i++) {
    packet[i] = fill;
  }
  packet[1] = JOYBUS_LINK_MAGIC;
  packet[5] = (uint8_t)(seq & JOYBUS_LINK_MARKER_MASK);
  packet[9] = joybus_link_checksum(packet);
}

static joybus_link_mode_t step_seq(joybus_link_t *link, uint8_t seq, uint8_t fill) {
  uint8_t packet[16];
  marked(packet, seq, fill);
  return joybus_link_step(link, 1, packet);
}

/* Drive a link to LIVE with silent valid frames starting at seq 0; returns next seq. */
static uint8_t go_live(joybus_link_t *link) {
  uint8_t seq = 0;
  joybus_link_init(link);
  for (int i = 0; i <= JOYBUS_LINK_RECOVER_FRAMES; i++) {
    step_seq(link, seq++, 0);
  }
  return seq;
}

static void marker(void) {
  uint8_t idle[16] = {0};
  require(joybus_link_marker_valid(golden0) && joybus_link_marker_valid(golden63),
          "host golden vectors validate in the ROM");
  require(joybus_link_sequence(golden63) == 63, "sequence is the low six bits of byte 5");
  require(!joybus_link_marker_valid(idle), "idle pad (all zero) has no marker");
  int detected = 1;
  for (int i = 0; i < 16; i++) {
    for (int bit = 0; bit < 8; bit++) {
      uint8_t corrupt[16];
      memcpy(corrupt, golden0, sizeof(corrupt));
      corrupt[i] ^= (uint8_t)(1u << bit);
      detected &= !joybus_link_marker_valid(corrupt);
    }
  }
  require(detected, "every single-bit corruption is rejected");
  int bounded = 1;
  for (int seq = 0; seq < 64; seq++) {
    uint8_t packet[16];
    marked(packet, (uint8_t)seq, 0xFF);
    bounded &= packet[9] <= JOYBUS_LINK_MARKER_MASK && joybus_link_marker_valid(packet);
  }
  require(bounded, "checksum fits the six readable bits");
}

static void no_device_and_idle_pad(void) {
  joybus_link_t link;
  uint8_t idle[16] = {0};
  uint8_t pressed[16] = {0x90, 0x3f, 0x7f, 0x81, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0};
  int procedural = 1;

  joybus_link_init(&link);
  for (int i = 0; i < 200; i++) {
    procedural &= joybus_link_step(&link, 0, idle) == JOYBUS_LINK_PROCEDURAL;
  }
  require(procedural, "no device stays procedural");

  procedural = 1;
  for (int i = 0; i < 200; i++) {
    procedural &= joybus_link_step(&link, 1, idle) == JOYBUS_LINK_PROCEDURAL;
  }
  require(procedural, "connected idle pad without marker stays procedural");

  procedural = 1;
  for (int i = 0; i < 200; i++) {
    pressed[5] = (uint8_t)(i & 0x3F);
    pressed[2] = (uint8_t)(i * 7);
    procedural &= joybus_link_step(&link, 1, pressed) == JOYBUS_LINK_PROCEDURAL;
  }
  require(procedural, "pad with buttons and stick input never replaces procedural");
}

static void silent_and_active_live(void) {
  joybus_link_t link;
  uint8_t seq = 0;
  int live = 1;

  joybus_link_init(&link);
  for (int i = 0; i < JOYBUS_LINK_RECOVER_FRAMES; i++) {
    live &= step_seq(&link, seq++, 0) == JOYBUS_LINK_PROCEDURAL;
  }
  require(live, "first valid frames stay procedural until the recovery count");
  require(step_seq(&link, seq++, 0) == JOYBUS_LINK_LIVE, "silent valid frames go live");
  live = 1;
  for (int i = 0; i < 300; i++) {
    live &= step_seq(&link, seq++, 0) == JOYBUS_LINK_LIVE;
  }
  require(live, "silent valid advancing frames stay live (no fallback), across sequence wrap");

  seq = go_live(&link);
  live = 1;
  for (int i = 0; i < 100; i++) {
    live &= step_seq(&link, seq++, (uint8_t)(0x20 + i)) == JOYBUS_LINK_LIVE;
  }
  require(live, "active valid frames stay live");
  require(link.held[0] == (uint8_t)(0x20 + 99), "live frames refresh the held packet");
}

static void stale_hold_then_procedural(void) {
  joybus_link_t link;
  uint8_t seq = go_live(&link);
  uint8_t frozen[16];
  int hold = 1;

  step_seq(&link, seq, 0x44);
  marked(frozen, seq, 0x44);
  for (int i = 1; i < JOYBUS_LINK_HOLD_FRAMES; i++) {
    hold &= joybus_link_step(&link, 1, frozen) == JOYBUS_LINK_HOLD;
  }
  require(hold, "frozen sequence holds the last live frame");
  require(link.held[0] == 0x44, "hold keeps the last live packet");
  require(joybus_link_step(&link, 1, frozen) == JOYBUS_LINK_PROCEDURAL,
          "hold falls back to procedural after JOYBUS_LINK_HOLD_FRAMES");
  int procedural = 1;
  for (int i = 0; i < 100; i++) {
    procedural &= joybus_link_step(&link, 1, frozen) == JOYBUS_LINK_PROCEDURAL;
  }
  require(procedural, "a frozen transport stays procedural");

  seq = go_live(&link);
  uint8_t idle[16] = {0};
  require(joybus_link_step(&link, 1, idle) == JOYBUS_LINK_HOLD, "marker loss holds");
  require(step_seq(&link, seq++, 0) == JOYBUS_LINK_HOLD, "first valid frame after loss only re-anchors");
  require(step_seq(&link, seq++, 0) == JOYBUS_LINK_LIVE, "advancing frame during hold resumes live");

  go_live(&link);
  require(joybus_link_step(&link, 0, idle) == JOYBUS_LINK_PROCEDURAL, "disconnect is procedural immediately");
}

static void recovery_and_corruption(void) {
  joybus_link_t link;
  uint8_t seq = 10;
  uint8_t corrupt[16];

  joybus_link_init(&link);
  step_seq(&link, seq++, 0);
  for (int i = 0; i < JOYBUS_LINK_RECOVER_FRAMES - 1; i++) {
    step_seq(&link, seq++, 0);
  }
  marked(corrupt, seq, 0);
  corrupt[9] ^= 0x01;
  require(joybus_link_step(&link, 1, corrupt) == JOYBUS_LINK_PROCEDURAL, "corrupt marker is unmarked");
  int procedural = 1;
  for (int i = 0; i < JOYBUS_LINK_RECOVER_FRAMES; i++) {
    procedural &= step_seq(&link, seq++, 0) == JOYBUS_LINK_PROCEDURAL;
  }
  require(procedural, "corrupt frame resets the recovery count (no flapping)");
  require(step_seq(&link, seq++, 0) == JOYBUS_LINK_LIVE, "recovers after the full count");

  joybus_link_init(&link);
  seq = 61;
  step_seq(&link, seq, 0);
  step_seq(&link, seq, 0);
  int mode = JOYBUS_LINK_PROCEDURAL;
  for (int i = 0; i < JOYBUS_LINK_RECOVER_FRAMES; i++) {
    seq = (uint8_t)((seq + 1) & 0x3F);
    mode = step_seq(&link, seq, 0);
  }
  require(mode == JOYBUS_LINK_LIVE, "valid repeats do not reset recovery and seq wraps 63 -> 0");
}

int main(void) {
  marker();
  no_device_and_idle_pad();
  silent_and_active_live();
  stale_hold_then_procedural();
  recovery_and_corruption();
  printf("n64 joybus link tests: %u checks, %u failures\n", checks, failures);
  return failures ? EXIT_FAILURE : EXIT_SUCCESS;
}
