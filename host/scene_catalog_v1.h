#ifndef CHIPVIZ_SCENE_CATALOG_V1_H
#define CHIPVIZ_SCENE_CATALOG_V1_H

#include <stddef.h>
#include <stdint.h>

#define CHIPVIZ_CATALOG_V1_RECORD_SIZE 145u
#define CHIPVIZ_CATALOG_V1_METADATA_SIZE 84u

typedef enum {
  CHIPVIZ_CATALOG_V1_OK,
  CHIPVIZ_CATALOG_V1_INVALID,
  CHIPVIZ_CATALOG_V1_UNAVAILABLE
} ChipvizCatalogV1Status;

typedef struct {
  uint8_t envelope[CHIPVIZ_CATALOG_V1_RECORD_SIZE];
  uint64_t sequence;
  uint64_t tick;
  uint8_t family;
  uint8_t variant;
  uint8_t metadata[CHIPVIZ_CATALOG_V1_METADATA_SIZE];
} ChipvizCatalogV1Record;

/*
 * Validate exactly one record. The caller owns both buffers; input must point
 * to length readable bytes, and output to a suitably aligned writable object.
 * They may overlap. On success output owns an independent copy of the envelope
 * and metadata; on any error, including unavailable Starfield, output is
 * unchanged. Null pointers or any length other than 145 are invalid.
 *
 * This checks neither a complete file nor session continuity. It does not
 * verify assets or provenance, establish renderer availability, authorization,
 * transport safety, or physical safety.
 */
ChipvizCatalogV1Status chipviz_catalog_v1_validate(
    const uint8_t *input, size_t length, ChipvizCatalogV1Record *output);

#endif
