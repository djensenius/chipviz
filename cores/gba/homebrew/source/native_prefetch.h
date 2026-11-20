#ifndef CHIPVIZ_GBA_NATIVE_PREFETCH_H
#define CHIPVIZ_GBA_NATIVE_PREFETCH_H

#include <stdint.h>

/* A native page has 40 block rows and 45 ordered overlay operations.
 * Two rows or nine overlay operations are prepared per field poll, so a
 * stable candidate needs 25 polls. Scene debounce supplies 30 polls. */
#define GBA_PREFETCH_ROWS 40u
#define GBA_PREFETCH_OVERLAYS 45u
#define GBA_PREFETCH_ROWS_PER_POLL 2u
#define GBA_PREFETCH_OVERLAYS_PER_POLL 9u
#define GBA_PREFETCH_NONE 0u
#define GBA_PREFETCH_BACKGROUND 1u
#define GBA_PREFETCH_OVERLAY 2u

typedef struct {
  uint8_t active, scene, palette, rows, overlays, ready;
  uint16_t keys;
} gba_native_prefetch_t;

void gba_native_prefetch_init(gba_native_prefetch_t *prefetch);
/* Returns 1 when a new native page snapshot must be made. A scene 7 value
 * cancels preparation. Palette and keys are sampled only at page start. */
uint8_t gba_native_prefetch_observe(gba_native_prefetch_t *prefetch, uint8_t scene,
                                    uint8_t palette, uint16_t keys);
/* Returns one bounded unit of work and its [first,end) indices. */
uint8_t gba_native_prefetch_next(gba_native_prefetch_t *prefetch, uint8_t *first, uint8_t *end);

#endif
