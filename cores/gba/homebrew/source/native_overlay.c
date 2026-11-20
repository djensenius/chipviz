#include "native_overlay.h"

void gba_native_overlay_init(gba_native_overlay_t *overlay) {
  overlay->head = 0u;
  overlay->count = 0u;
  overlay->sampled = 0u;
}

void gba_native_overlay_sample(gba_native_overlay_t *overlay, uint16_t keys) {
  uint8_t state = (uint8_t)(keys & 0x0fu);
  if (state == overlay->sampled) {
    return;
  }
  overlay->sampled = state;
  if (overlay->count < GBA_NATIVE_OVERLAY_QUEUE) {
    uint8_t tail = (uint8_t)((overlay->head + overlay->count) % GBA_NATIVE_OVERLAY_QUEUE);
    overlay->states[tail] = state;
    ++overlay->count;
  }
}

uint16_t gba_native_overlay_next(gba_native_overlay_t *overlay) {
  uint8_t state;
  if (!overlay->count) {
    return overlay->sampled;
  }
  state = overlay->states[overlay->head];
  overlay->head = (uint8_t)((overlay->head + 1u) % GBA_NATIVE_OVERLAY_QUEUE);
  --overlay->count;
  return state;
}
