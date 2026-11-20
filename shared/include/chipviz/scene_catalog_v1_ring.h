#ifndef CHIPVIZ_SCENE_CATALOG_V1_RING_H
#define CHIPVIZ_SCENE_CATALOG_V1_RING_H

#include <stdint.h>

#include "scene_catalog_v1_ids.h"

typedef enum {
  CHIPVIZ_RING_OK,
  CHIPVIZ_RING_INVALID,
  CHIPVIZ_RING_RESTART_REQUIRED
} ChipvizCatalogV1RingStatus;

/* Bit v means variant v is implemented, validated and currently usable.
 * A zero mask makes the family unavailable. All configured variants must be
 * catalog-valid; an unavailable configured variant excludes that family.
 * Initialize configured variants to zero for the catalog defaults. */
typedef struct {
  uint32_t usable[CHIPVIZ_CATALOG_V1_FAMILY_COUNT];
  uint32_t configured[CHIPVIZ_CATALOG_V1_FAMILY_COUNT];
} ChipvizCatalogV1Snapshot;

typedef struct {
  uint32_t family;
  uint32_t variant;
} ChipvizCatalogV1Request;

typedef struct {
  ChipvizCatalogV1Snapshot snapshot;
  uint32_t ring;
  unsigned state;
} ChipvizCatalogV1Ring;

/*
 * Caller must reset before first use and after restart-required. Snapshots
 * must remain immutable during each call, and input, state and output objects
 * must not overlap. Calls for a session must be serialized.
 *
 * Preview computes the nonempty ascending-family ring (bit family), without
 * opting in. Start requires explicit opt_in == 1 and exact confirmation of
 * that ring against the current snapshot. Start cannot replace a session.
 * The caller must independently verify and confirm the receiver's exact ring.
 */
void chipviz_catalog_v1_ring_reset(ChipvizCatalogV1Ring *session);
ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_preview(
    const ChipvizCatalogV1Snapshot *snapshot, uint32_t *ring);
ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_start(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot,
    unsigned opt_in, uint32_t confirmed_ring);

/*
 * Observe every relevant availability or configuration change immediately,
 * even if it is later reverted. Also pass a fresh current snapshot on every
 * selection. Any snapshot change latches restart-required until reset;
 * the caller must stop input and arrange the specified non-flashing fallback.
 * A null snapshot is invalid, not evidence that availability is unchanged.
 *
 * Relative requests require the caller's actual current usable pair in the ring.
 * Direct requests allow any usable catalog-valid variant of a ring family.
 * Relative requests restore the destination family's configured variant;
 * a singleton leaves the current pair unchanged. Rejections
 * leave output unchanged; ordinary rejections also leave session unchanged.
 * Requests do not apply or authorize a transition: callers must enforce all
 * scene-7 gates, other safety rules, and presentation independently.
 */
ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_observe(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot);
ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_next(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot,
    uint32_t current_family, uint32_t current_variant, ChipvizCatalogV1Request *request);
ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_previous(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot,
    uint32_t current_family, uint32_t current_variant, ChipvizCatalogV1Request *request);
ChipvizCatalogV1RingStatus chipviz_catalog_v1_ring_direct(
    ChipvizCatalogV1Ring *session, const ChipvizCatalogV1Snapshot *snapshot,
    uint32_t family, uint32_t variant, ChipvizCatalogV1Request *request);

#endif
