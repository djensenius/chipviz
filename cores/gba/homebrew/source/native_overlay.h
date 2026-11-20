#ifndef CHIPVIZ_GBA_NATIVE_OVERLAY_H
#define CHIPVIZ_GBA_NATIVE_OVERLAY_H

#include <stdint.h>

/* Preserve sampled A/B/Select/Start states in order without combining states
 * from different polls. If input outruns presentation, retain the oldest 16
 * transitions and discard newer ones; the current held state is used after
 * the queue drains, so a dropped release cannot leave an overlay stuck. */
#define GBA_NATIVE_OVERLAY_QUEUE 16u

typedef struct {
  uint8_t states[GBA_NATIVE_OVERLAY_QUEUE];
  uint8_t head, count, sampled;
} gba_native_overlay_t;

void gba_native_overlay_init(gba_native_overlay_t *overlay);
void gba_native_overlay_sample(gba_native_overlay_t *overlay, uint16_t keys);
uint16_t gba_native_overlay_next(gba_native_overlay_t *overlay);

#endif
