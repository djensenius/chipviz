#include "native_prefetch.h"

void gba_native_prefetch_init(gba_native_prefetch_t *prefetch) {
  prefetch->active = 0u;
  prefetch->scene = 0u;
  prefetch->palette = 0u;
  prefetch->keys = 0u;
  prefetch->rows = prefetch->ready = 0u;
}

uint8_t gba_native_prefetch_observe(gba_native_prefetch_t *prefetch, uint8_t scene,
                                    uint8_t palette, uint16_t keys, uint16_t stable) {
  if (scene == 7u || stable < GBA_PREFETCH_START_POLL) {
    gba_native_prefetch_init(prefetch);
    return 0u;
  }
  if (prefetch->active && prefetch->scene == scene) {
    return 0u;
  }
  prefetch->active = 1u;
  prefetch->scene = scene;
  prefetch->palette = palette;
  prefetch->keys = keys;
  prefetch->rows = prefetch->ready = 0u;
  return 1u;
}

uint8_t gba_native_prefetch_next(gba_native_prefetch_t *prefetch, uint8_t *first, uint8_t *end) {
  if (!prefetch->active || prefetch->ready || prefetch->rows >= GBA_PREFETCH_ROWS) {
    return GBA_PREFETCH_NONE;
  }
  *first = prefetch->rows;
  prefetch->rows = (uint8_t)(prefetch->rows + GBA_PREFETCH_ROWS_PER_POLL);
  *end = prefetch->rows;
  return GBA_PREFETCH_BACKGROUND;
}

void gba_native_prefetch_complete(gba_native_prefetch_t *prefetch) {
  if (prefetch->active && prefetch->rows == GBA_PREFETCH_ROWS) {
    prefetch->ready = 1u;
  }
}
