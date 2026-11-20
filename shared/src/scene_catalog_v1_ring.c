#include "../include/chipviz/scene_catalog_v1_ring.h"
#include "../include/chipviz/scene_catalog_v1_pairs.h"

#include <stddef.h>
#include <string.h>

enum { RING_RESET, RING_ACTIVE, RING_RESTART };

void chipviz_catalog_v1_ring_reset(ChipvizCatalogV1Ring *session) {
  if (session != NULL) {
    memset(session, 0, sizeof(*session));
  }
}

ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_preview(
    const ChipvizCatalogV1Snapshot *snapshot, uint32_t *ring) {
  uint32_t result = 0;
  unsigned family, variant;
  if (snapshot == NULL || ring == NULL) {
    return CHIPVIZ_RING_INVALID;
  }
  for (family = 0; family < CHIPVIZ_CATALOG_V1_FAMILY_COUNT; family++) {
    if (!chipviz_catalog_v1_pair_valid(family, snapshot->configured[family])) {
      return CHIPVIZ_RING_INVALID;
    }
    for (variant = 0; variant < 32; variant++) {
      if ((snapshot->usable[family] & (UINT32_C(1) << variant)) != 0 &&
          !chipviz_catalog_v1_pair_valid(family, variant)) {
        return CHIPVIZ_RING_INVALID;
      }
    }
    if ((snapshot->usable[family] & (UINT32_C(1) << snapshot->configured[family])) != 0) {
      result |= UINT32_C(1) << family;
    }
  }
  if (result == 0) {
    return CHIPVIZ_RING_INVALID;
  }
  *ring = result;
  return CHIPVIZ_RING_OK;
}

ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_start(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot,
    unsigned opt_in, uint32_t confirmed_ring) {
  uint32_t ring;
  if (session == NULL || session->state != RING_RESET || opt_in != 1 ||
      chipviz_catalog_v1_ring_preview(snapshot, &ring) != CHIPVIZ_RING_OK ||
      confirmed_ring != ring) {
    return CHIPVIZ_RING_INVALID;
  }
  session->snapshot = *snapshot;
  session->ring = ring;
  session->state = RING_ACTIVE;
  return CHIPVIZ_RING_OK;
}

ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_observe(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot) {
  unsigned family;
  if (session == NULL) {
    return CHIPVIZ_RING_INVALID;
  }
  if (session->state == RING_RESTART) {
    return CHIPVIZ_RING_RESTART_REQUIRED;
  }
  if (session->state != RING_ACTIVE || snapshot == NULL) {
    return CHIPVIZ_RING_INVALID;
  }
  for (family = 0; family < CHIPVIZ_CATALOG_V1_FAMILY_COUNT; family++) {
    if (snapshot->usable[family] != session->snapshot.usable[family] ||
        snapshot->configured[family] != session->snapshot.configured[family]) {
      session->state = RING_RESTART;
      return CHIPVIZ_RING_RESTART_REQUIRED;
    }
  }
  return CHIPVIZ_RING_OK;
}

static ChipvizCatalogV1RingStatus relative(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot,
    uint32_t current, uint32_t current_variant, ChipvizCatalogV1Request *request, int forward) {
  ChipvizCatalogV1RingStatus status;
  unsigned family, i;
  status = chipviz_catalog_v1_ring_observe(session, snapshot);
  if (status != CHIPVIZ_RING_OK) {
    return status;
  }
  if (request == NULL || !chipviz_catalog_v1_pair_valid(current, current_variant) ||
      (session->ring & (UINT32_C(1) << current)) == 0 ||
      (session->snapshot.usable[current] & (UINT32_C(1) << current_variant)) == 0) {
    return CHIPVIZ_RING_INVALID;
  }
  family = current;
  for (i = 0; i < CHIPVIZ_CATALOG_V1_FAMILY_COUNT; i++) {
    family = forward ? (family + 1u) % CHIPVIZ_CATALOG_V1_FAMILY_COUNT :
        (family + CHIPVIZ_CATALOG_V1_FAMILY_COUNT - 1u) % CHIPVIZ_CATALOG_V1_FAMILY_COUNT;
    if ((session->ring & (UINT32_C(1) << family)) != 0) {
      request->family = family;
      request->variant = family == current ? current_variant : session->snapshot.configured[family];
      return CHIPVIZ_RING_OK;
    }
  }
  return CHIPVIZ_RING_INVALID;
}

ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_next(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot,
    uint32_t current_family, uint32_t current_variant, ChipvizCatalogV1Request *request) {
  return relative(session, snapshot, current_family, current_variant, request, 1);
}

ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_previous(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot,
    uint32_t current_family, uint32_t current_variant, ChipvizCatalogV1Request *request) {
  return relative(session, snapshot, current_family, current_variant, request, 0);
}

ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_direct(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot,
    uint32_t family, uint32_t variant, ChipvizCatalogV1Request *request) {
  ChipvizCatalogV1RingStatus status;
  status = chipviz_catalog_v1_ring_observe(session, snapshot);
  if (status != CHIPVIZ_RING_OK) {
    return status;
  }
  if (request == NULL || !chipviz_catalog_v1_pair_valid(family, variant) ||
      (session->ring & (UINT32_C(1) << family)) == 0 ||
      (session->snapshot.usable[family] & (UINT32_C(1) << variant)) == 0) {
    return CHIPVIZ_RING_INVALID;
  }
  request->family = family;
  request->variant = variant;
  return CHIPVIZ_RING_OK;
}
