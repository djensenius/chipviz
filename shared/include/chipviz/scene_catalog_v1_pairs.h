#ifndef CHIPVIZ_SCENE_CATALOG_V1_PAIRS_H
#define CHIPVIZ_SCENE_CATALOG_V1_PAIRS_H

#include <stdint.h>

#define CHIPVIZ_CATALOG_V1_FAMILY_COUNT 14u

/* Catalog validity only; this does not establish receiver availability. */
static inline int chipviz_catalog_v1_pair_valid(uint32_t family, uint32_t variant) {
  static const unsigned char max_variant[CHIPVIZ_CATALOG_V1_FAMILY_COUNT] = {
    0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0
  };
  return family < CHIPVIZ_CATALOG_V1_FAMILY_COUNT && variant <= max_variant[family];
}

#endif
