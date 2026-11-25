#include "../host/scene_catalog_v1.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const uint8_t golden[145] = {
  [0] = 0x43, [1] = 0x56, [2] = 0x58, [3] = 0x31, [4] = 1, [5] = 1,
  [24] = 0xc7, [44] = 7, [45] = 2, [56] = 0xc2,
  [141] = 0xc3, [142] = 0xcc, [143] = 0xbb, [144] = 0x8b
};

static void require(int condition, const char *message) {
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    exit(1);
  }
}

static void repair(uint8_t *wire) {
  uint32_t crc = UINT32_C(0xffffffff);
  size_t i;
  unsigned bit;
  wire[56] = 0;
  for (i = 24; i < 56; i++) {
    wire[56] ^= wire[i];
  }
  for (i = 0; i < 141; i++) {
    crc ^= wire[i];
    for (bit = 0; bit < 8; bit++) {
      crc = (crc >> 1) ^ ((crc & 1u) ? UINT32_C(0xedb88320) : 0u);
    }
  }
  crc ^= UINT32_C(0xffffffff);
  for (i = 0; i < 4; i++) {
    wire[141 + i] = (uint8_t)(crc >> (8u * i));
  }
}

static void reject_unchanged(const uint8_t *wire, size_t length,
                             ChipvizCatalogV1Status expected) {
  ChipvizCatalogV1Record output, before;
  memset(&output, 0xa5, sizeof(output));
  memcpy(&before, &output, sizeof(output));
  require(chipviz_catalog_v1_validate(wire, length, &output) == expected, "expected error");
  require(memcmp(&output, &before, sizeof(output)) == 0, "error leaves output unchanged");
}

static void literal_vectors(void) {
  static const struct {
    uint8_t family, variant, flags, count, checksum, crc[4];
    ChipvizCatalogV1Status status;
  } vectors[] = {
    {2, 0, 0, 0, 0xc7, {0x52, 0x55, 0x2f, 0xd6}, CHIPVIZ_CATALOG_V1_OK},
    {2, 2, 0, 0, 0xc7, {0x5f, 0x1c, 0x03, 0xe5}, CHIPVIZ_CATALOG_V1_OK},
    {2, 3, 0, 0, 0xc7, {0xf9, 0xbb, 0x2d, 0x11}, CHIPVIZ_CATALOG_V1_INVALID},
    {14, 0, 0, 0, 0xcb, {0x51, 0x75, 0xce, 0xb2}, CHIPVIZ_CATALOG_V1_INVALID},
    {7, 0, 0x20, 0, 0xe2, {0xe9, 0x57, 0xaa, 0x5a}, CHIPVIZ_CATALOG_V1_INVALID},
    {7, 0, 0, 5, 0xc7, {0xb5, 0x99, 0x9f, 0x81}, CHIPVIZ_CATALOG_V1_INVALID},
    {7, 0, 0, 0, 0xc3, {0x42, 0x10, 0x3a, 0x80}, CHIPVIZ_CATALOG_V1_INVALID}
  };
  ChipvizCatalogV1Record output;
  uint8_t wire[145];
  size_t i;
  require(chipviz_catalog_v1_validate(golden, sizeof(golden), &output) == CHIPVIZ_CATALOG_V1_OK,
          "published record accepted");
  require(memcmp(output.envelope, golden, sizeof(golden)) == 0, "envelope preserved");
  require(output.sequence == 0 && output.tick == 0 && output.family == 7 &&
          output.variant == 0 && memcmp(output.metadata, golden + 57, 84) == 0,
          "decoded metadata preserved");
  for (i = 0; i < sizeof(vectors) / sizeof(vectors[0]); i++) {
    memcpy(wire, golden, sizeof(wire));
    wire[44] = vectors[i].family;
    wire[6] = vectors[i].variant;
    wire[46] = vectors[i].flags;
    wire[47] = vectors[i].count;
    wire[56] = vectors[i].checksum;
    memcpy(wire + 141, vectors[i].crc, 4);
    if (vectors[i].status == CHIPVIZ_CATALOG_V1_OK) {
      require(chipviz_catalog_v1_validate(wire, sizeof(wire), &output) == vectors[i].status,
              "published valid vector");
      require(memcmp(wire, output.envelope, sizeof(wire)) == 0 &&
              output.family == vectors[i].family && output.variant == vectors[i].variant,
              "vector fields preserved");
    } else {
      reject_unchanged(wire, sizeof(wire), vectors[i].status);
    }
  }
}

static void argument_and_overlap_tests(void) {
  union {
    ChipvizCatalogV1Record output;
    uint8_t bytes[sizeof(ChipvizCatalogV1Record) + 145];
  } storage;
  union {
    struct {
      uint64_t prefix[4];
      ChipvizCatalogV1Record output;
    } aligned;
    uint8_t bytes[sizeof(ChipvizCatalogV1Record) + 32];
  } later;
  uint8_t longer[146];
  uint8_t before[sizeof(storage.bytes)];
  size_t i;
  const size_t lengths[] = {0, 33, 144, 146};
  for (i = 0; i < sizeof(lengths) / sizeof(lengths[0]); i++) {
    memcpy(longer, golden, 145);
    longer[145] = 0;
    reject_unchanged(longer, lengths[i], CHIPVIZ_CATALOG_V1_INVALID);
  }
  reject_unchanged(NULL, 145, CHIPVIZ_CATALOG_V1_INVALID);
  require(chipviz_catalog_v1_validate(golden, 145, NULL) == CHIPVIZ_CATALOG_V1_INVALID,
          "null output");
  for (i = 0; i < 2; i++) {
    memset(&storage, 0xa5, sizeof(storage));
    memcpy(storage.bytes + (i ? 20 : 0), golden, 145);
    require(chipviz_catalog_v1_validate(storage.bytes + (i ? 20 : 0), 145, &storage.output)
            == CHIPVIZ_CATALOG_V1_OK, "overlap success");
    require(memcmp(storage.output.envelope, golden, 145) == 0, "overlap envelope");
  }
  memset(&later, 0xa5, sizeof(later));
  memcpy(later.bytes, golden, 145);
  require(chipviz_catalog_v1_validate(later.bytes, 145, &later.aligned.output)
          == CHIPVIZ_CATALOG_V1_OK, "output overlaps later in input");
  require(memcmp(later.aligned.output.envelope, golden, 145) == 0, "later overlap envelope");
  memset(&storage, 0xa5, sizeof(storage));
  memcpy(storage.bytes + 20, golden, 145);
  memcpy(before, storage.bytes, sizeof(before));
  require(chipviz_catalog_v1_validate(storage.bytes + 20, 144, &storage.output)
          == CHIPVIZ_CATALOG_V1_INVALID, "overlap bad length");
  require(memcmp(before, storage.bytes, sizeof(before)) == 0, "length error overlap unchanged");
  storage.bytes[20] = 0;
  memcpy(before, storage.bytes, sizeof(before));
  require(chipviz_catalog_v1_validate(storage.bytes + 20, 145, &storage.output)
          == CHIPVIZ_CATALOG_V1_INVALID, "overlap invalid");
  require(memcmp(before, storage.bytes, sizeof(before)) == 0, "overlap error unchanged");
  memcpy(storage.bytes + 20, golden, 145);
  storage.bytes[20 + 44] = 6;
  storage.bytes[20 + 57] = 1;
  /* 2000-01-01 in little endian. */
  storage.bytes[20 + 89] = 0x00;
  storage.bytes[20 + 90] = 0xac;
  storage.bytes[20 + 91] = 0xcf;
  storage.bytes[20 + 92] = 0xac;
  storage.bytes[20 + 93] = 0xdc;
  storage.bytes[20 + 109] = 1;
  repair(storage.bytes + 20);
  memcpy(before, storage.bytes, sizeof(before));
  require(chipviz_catalog_v1_validate(storage.bytes + 20, 145, &storage.output)
          == CHIPVIZ_CATALOG_V1_UNAVAILABLE, "overlap starfield unavailable");
  require(memcmp(before, storage.bytes, sizeof(before)) == 0, "unavailable overlap unchanged");
}

static void notes_and_integrity(void) {
  uint8_t wire[145];
  ChipvizCatalogV1Record output;
  size_t i;
  memcpy(wire, golden, sizeof(wire));
  wire[54] = 255;
  wire[55] = 255;
  repair(wire);
  require(chipviz_catalog_v1_validate(wire, 145, &output) == CHIPVIZ_CATALOG_V1_OK,
          "inactive notes unrestricted");
  require(memcmp(wire, output.envelope, 145) == 0, "inactive note bytes preserved");
  wire[47] = 4;
  repair(wire);
  reject_unchanged(wire, 145, CHIPVIZ_CATALOG_V1_INVALID);
  for (i = 0; i < 145; i++) {
    memcpy(wire, golden, sizeof(wire));
    wire[i] ^= 1;
    reject_unchanged(wire, 145, CHIPVIZ_CATALOG_V1_INVALID);
  }
}

int main(void) {
  literal_vectors();
  argument_and_overlap_tests();
  notes_and_integrity();
  puts("scene catalog v1 C tests passed");
  return 0;
}
