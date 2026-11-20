#include "native_prefetch.h"

void gba_native_prefetch_init(gba_native_prefetch_t *prefetch) {
  prefetch->active = 0u;
  prefetch->scene = 0u;
  prefetch->palette = 0u;
  prefetch->keys = 0u;
  prefetch->rows = prefetch->overlays = prefetch->ready = 0u;
}

uint8_t gba_native_prefetch_observe(gba_native_prefetch_t *prefetch, uint8_t scene,
                                    uint8_t palette, uint16_t keys) {
  if (scene == 7u) {
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
  prefetch->rows = prefetch->overlays = prefetch->ready = 0u;
  return 1u;
}

uint8_t gba_native_prefetch_next(gba_native_prefetch_t *prefetch, uint8_t *first, uint8_t *end) {
  if (!prefetch->active || prefetch->ready) {
    return GBA_PREFETCH_NONE;
  }
  if (prefetch->rows < GBA_PREFETCH_ROWS) {
    *first = prefetch->rows;
    prefetch->rows = (uint8_t)(prefetch->rows + GBA_PREFETCH_ROWS_PER_POLL);
    *end = prefetch->rows;
    return GBA_PREFETCH_BACKGROUND;
  }
  *first = prefetch->overlays;
  prefetch->overlays = (uint8_t)(prefetch->overlays + GBA_PREFETCH_OVERLAYS_PER_POLL);
  *end = prefetch->overlays;
  if (prefetch->overlays == GBA_PREFETCH_OVERLAYS) {
    prefetch->ready = 1u;
  }
  return GBA_PREFETCH_OVERLAY;
}
