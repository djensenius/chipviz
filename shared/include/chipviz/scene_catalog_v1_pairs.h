#ifndef CHIPVIZ_SCENE_CATALOG_V1_PAIRS_H
#define CHIPVIZ_SCENE_CATALOG_V1_PAIRS_H

#include <stdint.h>

#include "scene_catalog_v1_ids.h"

/* Catalog validity only; this does not establish receiver availability. */
#ifdef __CC65__
/* cc65 does not support the C99 inline keyword, even in C99 mode. */
static
#else
static inline
#endif
int chipviz_catalog_v1_pair_valid(uint32_t family, uint32_t variant) {
  static const unsigned char max_variant[CHIPVIZ_CATALOG_V1_FAMILY_COUNT] = {
    0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0
  };
  return family < CHIPVIZ_CATALOG_V1_FAMILY_COUNT && variant <= max_variant[family];
}

#endif
