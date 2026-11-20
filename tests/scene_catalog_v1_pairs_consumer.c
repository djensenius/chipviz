#include "../shared/include/chipviz/scene_catalog_v1_pairs.h"

int main(void) {
#ifdef CHIPVIZ_TEST_USE_PAIR
  uint32_t family = 2, variant = 1;
  int valid = chipviz_catalog_v1_pair_valid(family++, variant++);
  return !valid || family != 3 || variant != 2;
#else
  return 0;
#endif
}
