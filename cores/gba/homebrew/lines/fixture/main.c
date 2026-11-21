#include "fixture.h"
#include "../lines.h"

#include <stdint.h>
#include <string.h>

#define REG_DISPCNT (*(volatile uint16_t *)(uintptr_t)0x04000000u)
#define REG_VCOUNT (*(volatile uint16_t *)(uintptr_t)0x04000006u)
#define PAGE_BYTES (LINES_WIDTH * LINES_HEIGHT)
#define PAYLOAD_BYTES (PAGE_BYTES - FIXTURE_HEADER_SIZE)

static volatile uint16_t *const display = (volatile uint16_t *)(uintptr_t)0x06000000u;
/* One halfword before page 1, plus the page and one halfword after it. */
static volatile uint16_t *const guarded = (volatile uint16_t *)(uintptr_t)0x06009ffeu;
static volatile uint16_t *const palette = (volatile uint16_t *)(uintptr_t)0x05000000u;

static void wait_vblank(void) {
  while (REG_VCOUNT >= 160u) {}
  while (REG_VCOUNT < 160u) {}
}

static uint8_t page_byte(const volatile uint16_t *page, unsigned offset) {
  uint16_t pair = page[offset / 2u];
  return (uint8_t)(pair >> ((offset & 1u) * 8u));
}

static uint8_t payload_byte(const volatile uint16_t *data, unsigned length, unsigned offset) {
  return data && offset < length ? page_byte(data, offset) : 0;
}

static void present(const uint8_t *header, const volatile uint16_t *data,
                    unsigned length, unsigned start, unsigned chunk) {
  unsigned i;
  uint8_t h[FIXTURE_HEADER_SIZE];
  memcpy(h, header, sizeof(h));
  h[6] = (uint8_t)chunk;
  /* Invalidate the header before changing payload; publish it last. */
  for (i = 0; i < FIXTURE_HEADER_SIZE / 2u; ++i) {
    display[i] = 0;
  }
  wait_vblank();
  wait_vblank();
  for (i = 0; i < PAYLOAD_BYTES; i += 2u) {
    unsigned lo = payload_byte(data, length, start + i);
    unsigned hi = payload_byte(data, length, start + i + 1u);
    display[(FIXTURE_HEADER_SIZE + i) / 2u] = (uint16_t)(lo | (hi << 8));
  }
  for (i = 0; i < FIXTURE_HEADER_SIZE; i += 2u) {
    display[i / 2u] = (uint16_t)(h[i] | ((unsigned)h[i + 1u] << 8));
  }
  for (i = 0; i < 4; ++i) {
    wait_vblank();
  }
}

static void emit(const uint8_t *header, const volatile uint16_t *page, const uint16_t *staged) {
  present(header, page, PAGE_BYTES, 0, 0);
  present(header, page, PAGE_BYTES, PAYLOAD_BYTES, 1);
  present(header, staged, 512u, 0, 2);
}

int main(void) {
  uint8_t done[FIXTURE_HEADER_SIZE] = {'G', 'L', 'F', 'X', 1, FIXTURE_EVENTS};
  unsigned i;
  /* Unique RGB565 output per index for the libretro transport, not scene colours. */
  for (i = 0; i < 256; ++i) {
    palette[i] = (uint16_t)((i & 31u) | ((i >> 5) << 5));
  }
  REG_DISPCNT = 0x0404u; /* Mode 4, BG2, page 0 visible throughout. */
  fixture_run(guarded, emit);
  present(done, 0, 0, 0, 3);
  for (;;) {
    wait_vblank();
  }
}
