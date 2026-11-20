#include "scene_catalog_v1.h"

#include <string.h>

static uint64_t read_le(const uint8_t *bytes, size_t length) {
  uint64_t value = 0;
  size_t i;
  for (i = 0; i < length; i++) {
    value |= (uint64_t)bytes[i] << (8u * i);
  }
  return value;
}

/* Decode two's complement without an out-of-range unsigned-to-signed cast. */
static int64_t read_i64_le(const uint8_t *bytes) {
  uint64_t value = read_le(bytes, 8);
  if ((value & (UINT64_C(1) << 63)) != 0) {
    return -1 - (int64_t)(~value);
  }
  return (int64_t)value;
}

static int64_t read_i32_le(const uint8_t *bytes) {
  uint64_t value = read_le(bytes, 4);
  return (value & UINT64_C(0x80000000)) != 0
      ? (int64_t)value - INT64_C(4294967296) : (int64_t)value;
}

static uint32_t crc32(const uint8_t *bytes, size_t length) {
  uint32_t crc = UINT32_C(0xffffffff);
  size_t i;
  unsigned bit;
  for (i = 0; i < length; i++) {
    crc ^= bytes[i];
    for (bit = 0; bit < 8; bit++) {
      crc = (crc >> 1) ^ ((crc & 1u) ? UINT32_C(0xedb88320) : 0u);
    }
  }
  return crc ^ UINT32_C(0xffffffff);
}

static int any_nonzero(const uint8_t *bytes, size_t length) {
  size_t i;
  for (i = 0; i < length; i++) {
    if (bytes[i] != 0) {
      return 1;
    }
  }
  return 0;
}

ChipvizCatalogV1Status chipviz_catalog_v1_validate(
    const uint8_t *input, size_t length, ChipvizCatalogV1Record *output) {
  static const uint8_t max_variant[] = {0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0};
  const int64_t min_epoch = INT64_C(946684800000);
  const int64_t max_epoch = INT64_C(4102444799999);
  ChipvizCatalogV1Record result;
  uint8_t checksum = 0;
  size_t i;

  if (input == NULL || output == NULL || length != CHIPVIZ_CATALOG_V1_RECORD_SIZE) {
    return CHIPVIZ_CATALOG_V1_INVALID;
  }
  memset(&result, 0, sizeof(result));
  memcpy(result.envelope, input, sizeof(result.envelope));
  if (memcmp(result.envelope, "CVX1\1\1", 6) != 0 || result.envelope[7] != 0 ||
      crc32(result.envelope, 141) != read_le(result.envelope + 141, 4)) {
    return CHIPVIZ_CATALOG_V1_INVALID;
  }
  result.sequence = read_le(result.envelope + 8, 8);
  result.tick = read_le(result.envelope + 16, 8);
  result.family = result.envelope[44];
  result.variant = result.envelope[6];
  memcpy(result.metadata, result.envelope + 57, sizeof(result.metadata));
  for (i = 24; i < 57; i++) {
    checksum ^= result.envelope[i];
  }
  if (result.sequence != result.tick || result.envelope[24] != 0xc7 ||
      result.envelope[25] != 0 || checksum != 0 ||
      read_le(result.envelope + 26, 2) != (result.sequence & UINT64_C(0xffff)) ||
      (result.envelope[46] & 0xe0u) != 0 || result.envelope[47] > 4 ||
      result.family >= sizeof(max_variant) || result.variant > max_variant[result.family]) {
    return CHIPVIZ_CATALOG_V1_INVALID;
  }
  for (i = 0; i < result.envelope[47]; i++) {
    if (result.envelope[48 + 2 * i] > 127) {
      return CHIPVIZ_CATALOG_V1_INVALID;
    }
  }
  if (result.family != 6) {
    if (any_nonzero(result.metadata, sizeof(result.metadata))) {
      return CHIPVIZ_CATALOG_V1_INVALID;
    }
  } else {
    int64_t epoch = read_i64_le(result.metadata + 32);
    int64_t latitude = read_i32_le(result.metadata + 40);
    int64_t longitude = read_i32_le(result.metadata + 44);
    int64_t elevation = read_i32_le(result.metadata + 48);
    if (!any_nonzero(result.metadata, 32) || !any_nonzero(result.metadata + 52, 32) ||
        epoch < min_epoch || epoch > max_epoch ||
        latitude < -INT64_C(900000000) || latitude > INT64_C(900000000) ||
        longitude < -INT64_C(1800000000) || longitude > INT64_C(1800000000) ||
        elevation < -INT64_C(12000000) || elevation > INT64_C(100000000)) {
      return CHIPVIZ_CATALOG_V1_INVALID;
    }
    /* Rearranged exact rational bound; the difference and product fit u64. */
    if (result.tick > (uint64_t)(max_epoch - epoch) * UINT64_C(60) / UINT64_C(1000)) {
      return CHIPVIZ_CATALOG_V1_INVALID;
    }
    return CHIPVIZ_CATALOG_V1_UNAVAILABLE;
  }
  memcpy(output, &result, sizeof(result));
  return CHIPVIZ_CATALOG_V1_OK;
}
