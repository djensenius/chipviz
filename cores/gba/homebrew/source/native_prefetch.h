#ifndef CHIPVIZ_GBA_NATIVE_PREFETCH_H
#define CHIPVIZ_GBA_NATIVE_PREFETCH_H

#include <stdint.h>

/* Start at the 27th stable scene poll, matching the native renderer's
 * approximately four-poll page-start snapshot. The final batch also draws
 * the overlay; actual target timing must be measured independently. */
#define GBA_PREFETCH_ROWS 40u
#define GBA_PREFETCH_ROWS_PER_POLL 10u
#define GBA_PREFETCH_START_POLL 27u
#define GBA_PREFETCH_NONE 0u
#define GBA_PREFETCH_BACKGROUND 1u

typedef struct {
  uint8_t active, scene, palette, rows, ready;
  uint16_t keys;
} gba_native_prefetch_t;

void gba_native_prefetch_init(gba_native_prefetch_t *prefetch);
/* Returns 1 at page start. A changed candidate or scene 7 cancels a page.
 * stable is the current scene's consecutive-poll count. */
uint8_t gba_native_prefetch_observe(gba_native_prefetch_t *prefetch, uint8_t scene,
                                    uint8_t palette, uint16_t keys, uint16_t stable);
/* Returns one bounded unit of work and its [first,end) indices. */
uint8_t gba_native_prefetch_next(gba_native_prefetch_t *prefetch, uint8_t *first, uint8_t *end);
/* Call only after drawing the final batch and the complete overlay. */
void gba_native_prefetch_complete(gba_native_prefetch_t *prefetch);

#endif
